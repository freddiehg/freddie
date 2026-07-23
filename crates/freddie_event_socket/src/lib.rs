//! A loopback WebSocket that hands each text frame to a callback.
//!
//! A source in the mold of `freddie_app_nav`: [`listen`] binds the port and calls back per frame,
//! the caller decides what a frame means, and dropping the returned [`EventSocket`] closes
//! everything. It knows nothing about any particular event type, so every binary in the family can
//! take an event socket from the same call.
//!
//! Web pages are refused at the handshake. A browser attaches `Origin` to a WebSocket handshake and
//! leaves the decision to the server, and a `WebSocket` handshake is exempt from the same-origin
//! policy, so
//! without this check any page in any open tab could drive the socket.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};

use futures_util::StreamExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::accept_hdr_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tracing::{debug, warn};

/// A frame past this closes the connection that sent it. Nothing that belongs on this socket is
/// large, and a client must not be able to make the process allocate without bound.
const MAX_FRAME_BYTES: usize = 64 * 1024;

/// The listener. Dropping it stops accepting and closes every live connection.
///
/// It owns the only [`watch::Sender`], and every task holds a receiver, so the drop is what each of
/// them is waiting on. There is nothing to abort by hand.
pub struct EventSocket {
    _shutdown: watch::Sender<()>,
}

/// Bind `127.0.0.1:port` and call `on_message` for each text frame any client sends.
///
/// `on_message` runs on the socket's runtime, so it must not block: send on a channel and return,
/// the way `freddie_app_nav::watch`'s callback does. Every connection forwards its frames to one
/// task that owns `on_message`, so the calls are serialized and it need not be `Sync`.
///
/// The bind is synchronous, through `std`, so a busy port is an `Err` from this call rather than a
/// failure inside a spawned task that the caller would have to go looking for.
///
/// # Errors
///
/// If the port is taken, or loopback cannot be bound.
///
/// # Panics
///
/// If called outside a tokio runtime.
pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&str) + Send + 'static,
{
    let std_listener = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;

    let (shutdown, mut closed) = watch::channel(());
    // Every connection forwards its frames here, and one task owns `on_message` and drains them. The
    // sender is `Send` and `Clone`, the receiver stays on the one task; that is the shape the shared
    // `Arc<F>` was standing in for, and it drops the `Sync` bound. Unbounded because the drain is the
    // non-blocking `on_message` and keeps up; a frame is capped at `MAX_FRAME_BYTES` either way.
    let (forward, mut frames) = mpsc::unbounded_channel::<String>();

    tokio::spawn(async move {
        while let Some(frame) = frames.recv().await {
            on_message(&frame);
        }
        debug!("the event socket's dispatch ended");
    });

    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                () = dropped(&mut closed) => break,
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, peer)) => {
                    debug!(%peer, "accepted");
                    tokio::spawn(serve(stream, forward.clone(), closed.clone()));
                }
                // A refused connection is that client's problem; the listener keeps accepting.
                Err(e) => debug!(error = %e, "accept failed"),
            }
        }
        debug!("the event socket closed");
    });

    Ok(EventSocket {
        _shutdown: shutdown,
    })
}

/// Resolves once the [`EventSocket`] has been dropped, taking the only sender with it.
async fn dropped(closed: &mut watch::Receiver<()>) {
    while closed.changed().await.is_ok() {}
}

/// One connection: handshake, then forward every text frame to the dispatch task until it ends or
/// the socket is dropped.
async fn serve(
    stream: TcpStream,
    forward: mpsc::UnboundedSender<String>,
    mut closed: watch::Receiver<()>,
) {
    let config = WebSocketConfig {
        max_message_size: Some(MAX_FRAME_BYTES),
        ..WebSocketConfig::default()
    };
    let mut ws = match accept_hdr_async_with_config(stream, check_origin, Some(config)).await {
        Ok(ws) => ws,
        Err(e) => {
            debug!(error = %e, "handshake failed");
            return;
        }
    };

    loop {
        let frame = tokio::select! {
            // The socket was dropped. Say goodbye properly: dropping `ws` here instead would reset
            // the connection, and the client would see a protocol error rather than a close.
            () = dropped(&mut closed) => {
                if let Err(e) = ws.close(None).await {
                    debug!(error = %e, "could not close cleanly");
                }
                break;
            }
            frame = ws.next() => frame,
        };
        match frame {
            // The receiver is dropped only after the socket is gone, which the `dropped` arm above
            // catches first; a send that still loses that race is a frame arriving as we close, and
            // dropping it is what closing means.
            Some(Ok(Message::Text(text))) => {
                let _ = forward.send(text.as_str().to_owned());
            }
            Some(Ok(Message::Binary(_))) => debug!("dropping a binary frame"),
            // Ping and Close are tungstenite's to answer, and it has already queued the reply.
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                debug!(error = %e, "connection ended");
                break;
            }
            None => break,
        }
    }
}

/// The handshake gate: refuse a web page, admit everything else.
///
/// The large `Err` is `http::Response`, and the signature is tungstenite's `Callback`, so there is
/// nothing here to box: it is handed to `accept_hdr_async_with_config` and never returned upward.
#[expect(clippy::result_large_err)]
fn check_origin(request: &Request, response: Response) -> Result<Response, ErrorResponse> {
    let origin = match request.headers().get(http::header::ORIGIN) {
        None => None,
        Some(value) => match value.to_str() {
            Ok(origin) => Some(origin),
            // An origin that is not text is one this cannot clear, so it does not get to connect.
            Err(_) => return Err(refuse()),
        },
    };
    if origin_allowed(origin) {
        Ok(response)
    } else {
        warn!(?origin, "refusing a handshake from a web page");
        Err(refuse())
    }
}

fn refuse() -> ErrorResponse {
    let mut response = ErrorResponse::new(Some("origin not allowed".to_owned()));
    *response.status_mut() = http::StatusCode::FORBIDDEN;
    response
}

/// Whether a handshake carrying this `Origin` may connect.
///
/// Native clients send none, so absent connects. A page's `http`/`https` origin does not, and that
/// includes a page served from loopback, which is still a page. Anything else, in practice
/// `chrome-extension://<id>`, connects; the id is not matched, because an unpacked development
/// build's id follows from where it was loaded and a packed build's differs again.
fn origin_allowed(origin: Option<&str>) -> bool {
    origin.is_none_or(|origin| !origin.starts_with("http://") && !origin.starts_with("https://"))
}

#[cfg(test)]
mod tests {
    use super::origin_allowed;

    #[test]
    fn web_origins_are_refused_and_others_are_not() {
        for allowed in [None, Some("chrome-extension://abcdef"), Some("file://")] {
            assert!(origin_allowed(allowed), "{allowed:?} should connect");
        }
        for refused in [
            "https://evil.com",
            "http://evil.com",
            // A page served from loopback is still a page.
            "http://localhost:3000",
            "http://127.0.0.1:8797",
        ] {
            assert!(
                !origin_allowed(Some(refused)),
                "{refused} should not connect"
            );
        }
    }
}
