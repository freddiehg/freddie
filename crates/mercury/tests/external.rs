//! The event socket wired to mercury's vocabulary, the way `run` wires it.
//!
//! `main` composes `freddie_event_socket::listen(port, mercury::on_message)`. These drive that same
//! composition over a real connection, on an OS-assigned port so a running mercury is never
//! disturbed and two test binaries never collide.

use std::time::Duration;

use futures_util::SinkExt;
use mercury::MercuryEvent;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;

const SETTLE: Duration = Duration::from_millis(250);

fn free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = probe.local_addr().expect("a bound address").port();
    drop(probe);
    port
}

/// A listener wired to mercury's vocabulary, and the channel the events land on.
fn listen_for_events() -> (
    freddie_event_socket::EventSocket,
    u16,
    UnboundedReceiver<MercuryEvent>,
) {
    let port = free_port();
    let (event_tx, event_rx) = unbounded_channel();
    let socket = freddie_event_socket::listen(port, move |text| {
        mercury::on_message(text, &event_tx);
    })
    .expect("binding a free port");
    (socket, port, event_rx)
}

/// A tab frame becomes the event the model dispatches.
#[tokio::test]
async fn a_tab_frame_arrives_as_an_event() {
    let (_socket, port, mut event_rx) = listen_for_events();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
        .await
        .expect("connecting");

    ws.send(Message::Text(
        r#"{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}"#.to_owned(),
    ))
    .await
    .expect("sending");
    tokio::time::sleep(SETTLE).await;

    match event_rx.try_recv().expect("an event arrived") {
        MercuryEvent::Tab(ev) => assert_eq!(ev.url, "https://claude.ai/new"),
        other => panic!("expected a tab event, got {other:?}"),
    }
}

/// A frame outside the vocabulary is dropped, and dropping one neither panics the socket's runtime
/// nor closes the connection the client is still using.
#[tokio::test]
async fn an_unknown_frame_is_dropped_without_disturbing_the_connection() {
    let (_socket, port, mut event_rx) = listen_for_events();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
        .await
        .expect("connecting");

    for frame in [
        r#"{"kind":"MercuryEvent.Key","value":{"key":"KeyQ"}}"#,
        "not json at all",
    ] {
        ws.send(Message::Text(frame.to_owned()))
            .await
            .expect("sending");
    }
    tokio::time::sleep(SETTLE).await;
    assert!(event_rx.try_recv().is_err(), "nothing was dispatched");

    // Still open, and still delivering.
    ws.send(Message::Text(
        r#"{"kind":"IncomingEvent.Tab","value":{"url":"https://example.com/"}}"#.to_owned(),
    ))
    .await
    .expect("the connection survived two bad frames");
    tokio::time::sleep(SETTLE).await;
    assert!(
        event_rx.try_recv().is_ok(),
        "the next good frame still arrived"
    );
}

/// A web page cannot reach mercury's vocabulary at all, through this composition.
#[tokio::test]
async fn a_web_page_cannot_connect() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let (_socket, port, _event_rx) = listen_for_events();

    let mut request = format!("ws://127.0.0.1:{port}")
        .into_client_request()
        .expect("a valid request");
    request.headers_mut().insert(
        "origin",
        "https://evil.com".parse().expect("a valid header value"),
    );

    let refused = tokio_tungstenite::connect_async(request)
        .await
        .expect_err("a page is refused");
    assert!(
        format!("{refused}").contains("403"),
        "expected a 403, got {refused}"
    );
}

/// The default port is what the extension hardcodes, so a change here has to be a change there.
#[test]
fn the_default_port_is_the_one_the_extension_uses() {
    assert_eq!(mercury::DEFAULT_PORT, 3883);
}
