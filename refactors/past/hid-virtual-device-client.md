# freddie_virtual_hid: the Karabiner virtual-device client

A safe-Rust client that posts keyboard reports to Karabiner's `Karabiner-VirtualHIDDevice-Daemon` over its unix socket. No unsafe, no C++: the protocol is bytes on a `SOCK_STREAM` socket. This is the output half of the HID backend. It knows nothing about seizing, the session, or `freddie_keyboard`; it takes a desired keyboard state and makes the virtual device report it.

Verified against `pqrs-org/Karabiner-DriverKit-VirtualHIDDevice` package 8.2.0, driver 1.8.0, client protocol 7.

## Crate

```toml
# crates/freddie_virtual_hid/Cargo.toml
[package]
name = "freddie_virtual_hid"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
freddie_keys = { path = "../freddie_keys", version = "0.0.1" }
tracing = "0.1"

[lints]
workspace = true
```

Under `forbid(unsafe_code)`; it only reads and writes a socket. `std::os::unix::net::UnixStream` is the whole transport.

## The socket

```rust
/// Where the Karabiner daemon binds. Fixed, root-only. From the driver's
/// `virtual_hid_device_service/constants.hpp`.
const SOCKET_PATH: &str =
    "/Library/Application Support/org.pqrs/tmp/rootonly/karabiner_virtual_hid_device_service.sock";
```

The parent directory is `0700 root`, so the connecting process must be root. `freddie_hidd` is, so this is not a problem the client solves; it surfaces a connect failure as an error and lets the daemon decide.

## The two frame layers

Every message is an outer transport frame; a `request`/`response` frame carries an 8-byte id; the request body is an inner service payload. The outer integers are big-endian, the inner payload is native little-endian. This split is not ours to change: it is what the daemon's `cpp-unix_domain_stream` and `virtual_hid_device_service` layers require.

### Outer transport frame

```
[ body_size : u32 BE ]      // counts message_type through the end of body
[ message_type : u8 ]
[ body ... ]
```

```rust
/// The transport message type. `cpp-unix_domain_stream/impl/protocol.hpp`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MessageType {
    Heartbeat = 0,
    UserData = 1,
    HealthCheck = 2,
    HealthCheckResponse = 3,
    Request = 4,
    Response = 5,
}
```

A `Request` or `Response` body begins with the id:

```
[ body_size : u32 BE ]      // = 1 (type) + 8 (id) + payload.len()
[ message_type : u8 = 4|5 ]
[ request_id : u64 BE ]
[ payload ... ]
```

`request_id` is a per-connection counter (`++next`), unique among outstanding requests. It has no other meaning; a `Response` echoes the id of the `Request` it answers.

Reads and writes of these frames are the only IO. A `FrameCodec` over the `UnixStream`:

```rust
/// One transport frame, decoded. `payload` excludes the id even for request/response;
/// the id is lifted into `id`.
struct Frame {
    message_type: MessageType,
    id: Option<u64>,       // Some for Request/Response, None otherwise
    payload: Vec<u8>,
}

impl Frame {
    /// Read one frame, blocking. `Err` on EOF or a malformed length.
    fn read(sock: &mut impl std::io::Read) -> std::io::Result<Frame>;
    /// Write this frame. `body_size` is computed here.
    fn write(&self, sock: &mut impl std::io::Write) -> std::io::Result<()>;
}
```

`max_message_size` on the daemon is 1024, so a frame body over that is a protocol error we never produce (a keyboard payload is 70 bytes).

### Inner service payload

The body of a `Request` we send is:

```
[ client_protocol_version : u16 LE = 7 ]
[ request : u8 ]
[ request-specific struct, packed, native LE ]
```

```rust
/// Current client protocol the daemon accepts. It reports `DriverVersionMismatched`
/// on any other value, so this is checked against the driver at runtime, not assumed.
const CLIENT_PROTOCOL_VERSION: u16 = 7;

/// The service request. `virtual_hid_device_service/request.hpp`. Ordinal values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Request {
    VirtualHidKeyboardInitialize = 0,
    VirtualHidKeyboardTerminate = 1,
    VirtualHidKeyboardReset = 2,
    PostKeyboardInputReport = 6,
    // pointing and the other post_* variants exist but are unused here
}
```

## The keyboard input report

The struct the daemon copies into the driver. Packed, 67 bytes, little-endian. From `virtual_hid_device_driver/hid_report/keyboard_input.hpp`.

```
u8  report_id   = 1
u8  modifiers               // bitmask below
u8  reserved    = 0
u16 keys[32]                // HID Keyboard/Keypad (page 0x07) usage ids, LE; 0 = empty
```

Note the keys are `u16` and there are 32 slots, not the 6-byte boot array. Modifier bits (`hid_report/modifier.hpp`):

```rust
/// Karabiner's modifier bitmask byte. Distinct from `freddie_keys::ModifierFlags`:
/// this one splits left/right and is a wire detail of this crate.
mod modifier {
    pub const LEFT_CONTROL: u8 = 0x01;
    pub const LEFT_SHIFT: u8 = 0x02;
    pub const LEFT_OPTION: u8 = 0x04;
    pub const LEFT_COMMAND: u8 = 0x08;
    pub const RIGHT_CONTROL: u8 = 0x10;
    pub const RIGHT_SHIFT: u8 = 0x20;
    pub const RIGHT_OPTION: u8 = 0x40;
    pub const RIGHT_COMMAND: u8 = 0x80;
}
```

Reports are absolute state, not events. A report is the full set of keys and modifiers currently held. Key-down is a report that includes the key; key-up is a later report with it removed; releasing everything is an all-zero report. So the client owns a current-state struct and re-serializes it on every change.

```rust
/// The virtual keyboard's current held state. The client's single source of truth for
/// what to serialize. Makes an invalid report unrepresentable: modifiers is a validated
/// bitmask, keys is a set with the driver's capacity.
struct KeyboardState {
    modifiers: u8,
    keys: KeyUsageSet,   // holds up to 32 distinct usage ids, insertion filling low slots
}

/// A HID Keyboard/Keypad usage id (page 0x07). Newtype so a raw u16 cannot be mistaken
/// for a CGKeyCode or a freddie_keys code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Usage(pub u16);

impl KeyboardState {
    fn press(&mut self, key: HidKey);     // set a modifier bit, or insert a usage
    fn release(&mut self, key: HidKey);   // clear a modifier bit, or remove a usage
    fn clear(&mut self);                  // all up

    /// The 67-byte packed report for the current state.
    fn report_bytes(&self) -> [u8; 67];
}

/// What the client is asked to hold down or release: either a modifier (which side is
/// explicit, because the wire bitmask is sided) or an ordinary key usage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HidKey {
    Modifier(ModifierBit),  // one of the eight bits above
    Key(Usage),
}
```

`report_bytes` is the only place the byte layout is written. `KeyUsageSet` fills the first empty slot on insert and zeroes matching slots on remove, matching the driver's `insert`/`erase`.

Worked example, shift+e held (from the verified capture): inner payload

```
07 00            version 7
06               PostKeyboardInputReport
01               report_id
02               modifiers = LEFT_SHIFT
00               reserved
08 00 00 00 ...  keys[0] = 0x0008 (e), rest 0     (64 bytes)
```

wrapped as a `Request` frame (`body_size = 1 + 8 + 70 = 79`).

## Initialize parameters

`VirtualHidKeyboardInitialize` carries a 24-byte parameters struct (three `u64` LE), `virtual_hid_device_service/parameters.hpp`:

```rust
/// The virtual keyboard's identity. Defaults match Karabiner's.
struct KeyboardParameters {
    vendor_id: u64,     // 0x16c0
    product_id: u64,    // 0x27db
    country_code: u64,  // 0
}
```

Payload is `[07 00][00][parameters]`.

## The handshake, heartbeat, and drain

The daemon does not have the device ready at connect. After `Initialize` it pushes status as server-initiated `Request` frames whose payload is a run of `[response_type : u8][value : u8]` pairs. From `response.hpp`:

```rust
/// Status the daemon pushes. `virtual_hid_device_service/response.hpp`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Response {
    None = 0,
    DriverActivated = 1,
    DriverConnected = 2,
    DriverVersionMismatched = 3,
    VirtualHidKeyboardReady = 4,
    VirtualHidPointingReady = 5,
}
```

Two rules the daemon enforces, and the client must honor or be dropped:

- Answer every server-initiated `Request`. Each status push is a `Request` (type 4) with an id; reply with a `Response` (type 5) carrying the same id and an empty payload. Miss it and the daemon's request manager times out at 15 s and closes the connection.
- Heartbeat. Send a `Heartbeat` frame (type 0, body just the type byte — the five bytes `00 00 00 01 00`) every ~3 s when otherwise idle. The daemon drops a peer it has heard nothing from for 30 s. Any frame refreshes the deadline, so posting reports counts, but idle time must be filled.

The client must also read and discard the `Response` the daemon returns for each report `Request`, so the daemon's send buffer never stalls, and reply to a `HealthCheck` (type 2) with `HealthCheckResponse` (type 3) if one ever arrives.

## The reader thread and the public API

One background thread owns the socket reads: it blocks on `Frame::read`, answers status requests, records readiness, and answers health checks. Writes (reports, heartbeats) happen from the caller's thread; the `UnixStream` is `try_clone`d so the reader and the writer hold independent halves, and a single writer `Mutex` serializes frames. A heartbeat is a timer, not a poll: the writer wakes on either an inbound command or the 3 s deadline via a `recv_timeout` on the command channel.

```rust
/// A live connection to the Karabiner daemon, with the virtual keyboard initialized and
/// ready. Dropping it terminates the virtual keyboard and closes the socket.
pub struct VirtualKeyboard { /* writer half, reader join handle, state */ }

/// Why a virtual keyboard could not be brought up.
pub enum ConnectError {
    /// The daemon socket is absent or refused: the driver is not installed or its daemon
    /// is not running.
    NotRunning(std::io::Error),
    /// The daemon reported a protocol mismatch: its driver expects a version other than
    /// `CLIENT_PROTOCOL_VERSION`.
    VersionMismatch,
    /// Ready was not reported within the timeout.
    NotReady,
}

impl VirtualKeyboard {
    /// Connect, initialize, and block until `VirtualHidKeyboardReady(1)` or the timeout.
    /// Spawns the reader thread and starts the heartbeat.
    pub fn connect(params: KeyboardParameters, ready_timeout: Duration)
        -> Result<Self, ConnectError>;

    /// Set the full held state and post it. The caller (the daemon) owns the mapping from
    /// its notion of held keys to `HidKey`s; this posts exactly what it is given.
    pub fn set_state(&self, held: &[HidKey]) -> std::io::Result<()>;
}
```

`set_state` takes the whole held set rather than press/release deltas, because the wire is absolute state and making the caller describe deltas would put the state machine in two places. The daemon holds one `KeyboardState`, mutates it per emitted event, and calls `set_state` with the result.

## Tests

Pure-logic, no daemon:

- `report_bytes` for a known held state equals the verified byte capture (shift+e above, and a plain `a`, and all-up).
- The `Frame` codec round-trips: encode a `Request` with an id and a payload, decode it, get the same id and bytes; `body_size` is `1 + 8 + payload.len()`.
- `KeyUsageSet` fills low slots and removes correctly, and overflow past 32 is refused rather than silently dropped.
- Endianness: `body_size`/`request_id` serialize MSB-first; `client_protocol_version` and `keys[]` serialize LSB-first.

The live path (connect, ready, type a character) is a manual demo binary, not a unit test, because it needs the installed driver and a running daemon.
