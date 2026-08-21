# `EventSocket` reports the address it bound

Requires nothing. One change. `listen` already binds synchronously through `std` so a busy port is an `Err` from the call. It does not say which address that bind became. A caller that passes `0` (OS-assigned) has no way to learn the port without a second listener, and two binds of `0` are two ports.

The socket tests already want this. Their module comment says they bind port 0 and read back what the OS assigned. They actually bind a probe, drop it, and `listen` on that number:

```rust
// from crates/freddie_event_socket/tests/socket.rs
fn listen_anywhere<F>(on_message: F) -> (freddie_event_socket::EventSocket, u16, String)
where
    F: Fn(&str) + Send + Sync + 'static,
{
    // Port 0 twice: once to learn a free port, then again to bind it for real. `listen` takes a
    // port rather than a listener, and this is the one place that difference has to be bridged.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = probe.local_addr().expect("a bound address").port();
    drop(probe);
    let socket = freddie_event_socket::listen(port, on_message).expect("binding the free port");
    (socket, port, format!("ws://127.0.0.1:{port}"))
}
```

Between `drop(probe)` and `listen`, another process can take that port. `listen(0)` plus the bound address on the returned `EventSocket` is the bind that happened.

Mercury's daemon still passes a concrete `--port` (default 3883). It does not call `local_addr`. A daemon that runs more than one instance (isograph, filesystem-events.md change 1, after a pin-rev) binds `0` and writes `local_addr().port()` next to its lock.

## What the caller does

```rust
let socket = freddie_event_socket::listen(0, on_message)?;
let port = socket.local_addr().port();
// clients connect to ws://127.0.0.1:{port}
```

`listen(3883, ...)` still binds 3883 when it is free. `local_addr().port()` is then 3883.

Dropping the socket still closes every connection and frees the port. Rebind on that same number still works, using the port `local_addr` reported.

## Types

Most important first.

```rust
// from crates/freddie_event_socket/src/lib.rs
pub struct EventSocket {
    _shutdown: watch::Sender<()>,
    local_addr: SocketAddr,
}

impl EventSocket {
    /// The loopback address this socket is accepting on.
    ///
    /// Captured at bind, so `listen(0, ...)` is how a caller learns the OS-assigned port:
    /// `socket.local_addr().port()`.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}
```

`SocketAddr`, not `io::Result<SocketAddr>`. Failure to ask the OS for the bound address happens at bind, and `listen` already returns `io::Result`. An `EventSocket` that exists has an address. A method that can fail would be a lie the caller would match on forever.

The field is private. `_shutdown` stays private. Nothing else on the type changes: drop still closes accept and every live connection.

## Change 1: capture the address at bind, and stop probing in tests

Before:

```rust
// from crates/freddie_event_socket/src/lib.rs
pub struct EventSocket {
    _shutdown: watch::Sender<()>,
}

pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&str) + Send + 'static,
{
    let std_listener = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;

    let (shutdown, mut closed) = watch::channel(());
    // ...
    Ok(EventSocket {
        _shutdown: shutdown,
    })
}
```

After:

```rust
// from crates/freddie_event_socket/src/lib.rs
pub struct EventSocket {
    _shutdown: watch::Sender<()>,
    local_addr: SocketAddr,
}

impl EventSocket {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&str) + Send + 'static,
{
    let std_listener = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    std_listener.set_nonblocking(true)?;
    let local_addr = std_listener.local_addr()?;
    let listener = TcpListener::from_std(std_listener)?;

    let (shutdown, mut closed) = watch::channel(());
    // The rest of `listen` is unchanged: the forward channel, the dispatch task, the accept
    // task, `serve`.
    Ok(EventSocket {
        _shutdown: shutdown,
        local_addr,
    })
}
```

`local_addr()?` is the std call on the listener that just bound. If it fails, nothing has been spawned and `listen` returns `Err`, the same as a failed bind. Do not call it on the tokio listener after `from_std`: the value is known before the move.

The crate doc on `listen` gains one sentence: a caller that passed `0` reads the assigned port from [`EventSocket::local_addr`].

### Tests in this crate

`listen_anywhere` binds `0` and reads the socket:

```rust
// from crates/freddie_event_socket/tests/socket.rs
fn listen_anywhere<F>(on_message: F) -> (freddie_event_socket::EventSocket, u16, String)
where
    F: Fn(&str) + Send + 'static,
{
    let socket = freddie_event_socket::listen(0, on_message).expect("binding an OS-assigned port");
    let port = socket.local_addr().port();
    (socket, port, format!("ws://127.0.0.1:{port}"))
}
```

The `Send + Sync` bound on the helper becomes `Send`, matching `listen`. `collector` is still `Sync`; it still satisfies `Send`.

The probe, the `drop(probe)`, and the comment about bridging a port versus a listener all go. Every existing test that called `listen_anywhere` now actually binds port 0. `dropping_the_socket_closes_clients_and_frees_the_port` still rebinds with `listen(port, ...)` using the port `local_addr` reported.

New test, same file:

```rust
// from crates/freddie_event_socket/tests/socket.rs
#[tokio::test]
async fn listen_zero_reports_localhost_and_a_nonzero_port() {
    let (recorded, on_message) = collector();
    let socket = freddie_event_socket::listen(0, on_message).expect("binding an OS-assigned port");
    let addr = socket.local_addr();
    assert_eq!(addr.ip(), std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    assert_ne!(addr.port(), 0);

    let url = format!("ws://127.0.0.1:{}", addr.port());
    let mut ws = connect(&url).await;
    ws.send(Message::Text("via assigned port".into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;
    assert_eq!(seen(&recorded), vec!["via assigned port"]);
}
```

`expect` names an invariant the test established: the OS assigned a port on loopback.

Two `listen(0, ...)` in one test get two ports. Degenerate: the ports differ, both nonzero, both localhost. Add it:

```rust
// from crates/freddie_event_socket/tests/socket.rs
#[tokio::test]
async fn two_listen_zero_binds_are_two_ports() {
    let (_a_seen, a_cb) = collector();
    let (_b_seen, b_cb) = collector();
    let a = freddie_event_socket::listen(0, a_cb).expect("first bind");
    let b = freddie_event_socket::listen(0, b_cb).expect("second bind");
    assert_ne!(a.local_addr().port(), b.local_addr().port());
    assert_ne!(a.local_addr().port(), 0);
    assert_ne!(b.local_addr().port(), 0);
}
```

### Mercury tests

Same probe lives in `crates/mercury/tests/external.rs` as `free_port`. It becomes `listen(0, ...)` plus `local_addr().port()`:

```rust
// from crates/mercury/tests/external.rs
fn listen_for_events() -> (
    freddie_event_socket::EventSocket,
    u16,
    UnboundedReceiver<MercuryEvent>,
) {
    let (event_tx, event_rx) = unbounded_channel();
    let socket = freddie_event_socket::listen(0, move |text| {
        mercury::on_message(text, &event_tx);
    })
    .expect("binding an OS-assigned port");
    let port = socket.local_addr().port();
    (socket, port, event_rx)
}
```

`free_port` is deleted. The tests still connect to `ws://127.0.0.1:{port}`.

### Unchanged

`crates/mercury/src/daemon.rs` still `listen(port, ...)` with the clap `--port` / `MERCURY_PORT` value. It does not read `local_addr`. A failed bind is still the existing `unwrap_or_else(|e| panic!(...))`. This change does not touch that panic; it is the busy-port path `external-events.md` already shipped.

## Call sites

- `listen` writes `local_addr` on the struct it returns.
- `EventSocket::local_addr` is the only reader in production. Mercury's daemon is not a reader.
- `listen_anywhere` (this crate's tests) and `listen_for_events` (mercury tests) are the readers that land with the change.
