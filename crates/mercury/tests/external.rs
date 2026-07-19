//! The event socket wired to mercury's vocabulary, the way `run` wires it.
//!
//! `main` composes `freddie_event_socket::listen(port, mercury::on_message)`. These drive that same
//! composition over a real connection, on an OS-assigned port so a running mercury is never
//! disturbed and two test binaries never collide.

use std::time::Duration;

use futures_util::SinkExt;
use tokio_tungstenite::tungstenite::Message;

const SETTLE: Duration = Duration::from_millis(250);

fn free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = probe.local_addr().expect("a bound address").port();
    drop(probe);
    port
}

/// Every frame is refused while `IncomingEvent` is empty, and refusing one neither panics the
/// socket's runtime nor closes the connection. A client may keep talking; nothing listens yet.
#[tokio::test]
async fn frames_are_refused_without_disturbing_the_connection() {
    let port = free_port();
    let _socket =
        freddie_event_socket::listen(port, mercury::on_message).expect("binding a free port");

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
        .await
        .expect("connecting");

    for frame in [
        r#"{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}"#,
        "not json at all",
    ] {
        ws.send(Message::Text(frame.to_owned()))
            .await
            .expect("sending");
    }
    tokio::time::sleep(SETTLE).await;

    // Still open: one more frame goes out without error.
    ws.send(Message::Text("still connected".to_owned()))
        .await
        .expect("the connection survived two refusals");
}

/// A web page cannot reach mercury's vocabulary at all, through this composition.
#[tokio::test]
async fn a_web_page_cannot_connect() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let port = free_port();
    let _socket =
        freddie_event_socket::listen(port, mercury::on_message).expect("binding a free port");

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
