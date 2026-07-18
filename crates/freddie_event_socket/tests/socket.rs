//! The socket against a real client.
//!
//! Every test binds port 0 and reads back what the OS assigned, so concurrent test binaries never
//! collide and none of them can take the port a running mercury is using.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::SinkExt;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Long enough for a frame to cross loopback and reach the callback. The tests assert on what has
/// arrived by then, so this trades a few milliseconds for not needing a channel in every one.
const SETTLE: Duration = Duration::from_millis(250);

/// A callback that records what it was handed, and the handle to read it back.
fn collector() -> (
    Arc<Mutex<Vec<String>>>,
    impl Fn(&str) + Send + Sync + 'static,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    (seen, move |text: &str| {
        sink.lock()
            .expect("no test panics while holding this")
            .push(text.to_owned());
    })
}

fn seen(recorded: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    recorded
        .lock()
        .expect("no test panics while holding this")
        .clone()
}

/// A listener on an OS-assigned port, and the URL to reach it.
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

async fn connect(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("connecting");
    ws
}

async fn connect_with_origin(
    url: &str,
    origin: &str,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let mut request = url.into_client_request().expect("a valid request");
    request
        .headers_mut()
        .insert("origin", origin.parse().expect("a valid header value"));
    tokio_tungstenite::connect_async(request).await.map(|_| ())
}

#[tokio::test]
async fn a_text_frame_arrives_intact() {
    let (recorded, on_message) = collector();
    let (_socket, _port, url) = listen_anywhere(on_message);

    let mut ws = connect(&url).await;
    // Multibyte, because the frame crosses as bytes and `as_str` has to put it back together.
    ws.send(Message::Text("héllo ✓ мир".into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;

    assert_eq!(seen(&recorded), vec!["héllo ✓ мир"]);
}

#[tokio::test]
async fn two_connections_both_deliver() {
    let (recorded, on_message) = collector();
    let (_socket, _port, url) = listen_anywhere(on_message);

    let mut first = connect(&url).await;
    let mut second = connect(&url).await;
    first
        .send(Message::Text("from first".into()))
        .await
        .expect("sending");
    second
        .send(Message::Text("from second".into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;

    let mut arrived = seen(&recorded);
    arrived.sort();
    assert_eq!(arrived, vec!["from first", "from second"]);

    // One going away leaves the other delivering.
    drop(first);
    second
        .send(Message::Text("still here".into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;
    assert!(seen(&recorded).contains(&"still here".to_owned()));
}

#[tokio::test]
async fn a_web_page_is_refused_and_delivers_nothing() {
    let (recorded, on_message) = collector();
    let (_socket, _port, url) = listen_anywhere(on_message);

    for origin in [
        "https://evil.com",
        // A page served from loopback is still a page.
        "http://localhost:3000",
    ] {
        let refused = connect_with_origin(&url, origin)
            .await
            .expect_err("refused");
        assert!(
            format!("{refused}").contains("403"),
            "{origin} should be refused with 403, got {refused}"
        );
    }

    connect_with_origin(&url, "chrome-extension://abcdef")
        .await
        .expect("the extension connects");

    tokio::time::sleep(SETTLE).await;
    assert!(
        seen(&recorded).is_empty(),
        "a refused handshake delivers nothing"
    );
}

#[tokio::test]
async fn a_binary_frame_is_dropped_and_the_connection_survives() {
    let (recorded, on_message) = collector();
    let (_socket, _port, url) = listen_anywhere(on_message);

    let mut ws = connect(&url).await;
    ws.send(Message::Binary(vec![0, 1, 2].into()))
        .await
        .expect("sending");
    ws.send(Message::Text("after the binary".into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;

    assert_eq!(seen(&recorded), vec!["after the binary"]);
}

#[tokio::test]
async fn an_oversized_frame_closes_only_its_own_connection() {
    let (recorded, on_message) = collector();
    let (_socket, _port, url) = listen_anywhere(on_message);

    let mut ws = connect(&url).await;
    ws.send(Message::Text("x".repeat(70 * 1024).into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;
    assert!(
        seen(&recorded).is_empty(),
        "the oversized frame never arrives"
    );

    let mut second = connect(&url).await;
    second
        .send(Message::Text("after".into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        seen(&recorded),
        vec!["after"],
        "the listener kept accepting"
    );
}

#[tokio::test]
async fn dropping_the_socket_closes_clients_and_frees_the_port() {
    let (recorded, on_message) = collector();
    let (socket, port, url) = listen_anywhere(on_message);

    let mut ws = connect(&url).await;
    ws.send(Message::Text("before".into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;
    assert_eq!(seen(&recorded), vec!["before"]);

    drop(socket);
    tokio::time::sleep(SETTLE).await;

    // The client's stream ends rather than hanging.
    let ended = tokio::time::timeout(SETTLE, futures_util::StreamExt::next(&mut ws)).await;
    assert!(
        matches!(ended, Ok(None) | Ok(Some(Ok(Message::Close(_))))),
        "the client saw it close: {ended:?}"
    );

    // And the port is free immediately, with no lingering listener holding it.
    let (again, on_message) = collector();
    let socket = freddie_event_socket::listen(port, on_message).expect("rebinding the same port");
    let mut ws = connect(&url).await;
    ws.send(Message::Text("after".into()))
        .await
        .expect("sending");
    tokio::time::sleep(SETTLE).await;
    assert_eq!(seen(&again), vec!["after"]);
    drop(socket);
}
