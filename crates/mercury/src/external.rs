//! What mercury can be told from outside the process, over `freddie_event_socket`.
//!
//! The transport is generic and the vocabulary is mercury's, the same split `freddie_app_nav` uses:
//! the socket hands up a frame as a `&str`, and [`on_message`] decides what it means.

use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, warn};

use crate::{MercuryEvent, tab};

/// The port mercury listens on when nothing overrides it. Hardcoded in the extension too.
///
/// Mercury's melting point, -38.83 °C: the one fact about the element, which is the metal that is
/// liquid at room temperature. Below 49152, which is where macOS starts handing out ephemeral ports
/// (`net.inet.ip.portrange.first`): a listener up there can find its port already taken by some
/// outbound socket that grabbed it first.
///
/// IANA has it registered to VRPN (VR Peripheral Network), which nothing here runs. Registration is
/// advisory, and a collision would show up as the bind failing at startup, which is fatal and says
/// so.
pub const DEFAULT_PORT: u16 = 3883;

/// Everything an outside process may say to mercury. A sender cannot say anything else, so remote
/// key injection and remote quit are unrepresentable rather than filtered.
///
/// `MercuryEvent` deliberately does not derive `Deserialize`: deriving it would make
/// `MercuryEvent::Key` and `MercuryEvent::Quit` constructible from the wire, and "no remote
/// keyboard, no remote kill" would be a rule some match arm enforces rather than something the
/// types say.
///
#[derive(serde::Deserialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
#[serde(tag = "kind", content = "value")]
pub enum IncomingEvent {
    /// The front browser tab's URL changed.
    #[serde(rename = "IncomingEvent.Tab")]
    Tab(TabMessage),
}

#[derive(serde::Deserialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct TabMessage {
    pub url: String,
}

/// Turn one frame into an event and send it. Runs on the socket's runtime, so it parses, sends,
/// and returns.
///
/// A frame that is not a valid [`IncomingEvent`] is logged and dropped: a client speaking nonsense
/// is a client bug, not a reason to tear the connection down.
pub fn on_message(text: &str, event_tx: &UnboundedSender<MercuryEvent>) {
    match serde_json::from_str::<IncomingEvent>(text) {
        Ok(IncomingEvent::Tab(TabMessage { url })) => {
            debug!(%url, "tab");
            // A closed channel means the event loop has ended, which is the way out running.
            let _ = event_tx.send(tab(url));
        }
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}

#[cfg(test)]
mod tests {
    use super::IncomingEvent;

    #[test]
    fn a_tab_frame_carries_its_url() {
        let frame = r#"{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}"#;
        let IncomingEvent::Tab(tab) =
            serde_json::from_str::<IncomingEvent>(frame).expect("a tab frame deserializes");
        assert_eq!(tab.url, "https://claude.ai/new");
    }

    #[test]
    fn nothing_outside_the_vocabulary_deserializes() {
        for frame in [
            // The key vocabulary is mercury's own and stays unreachable from the wire.
            r#"{"kind":"MercuryEvent.Key","value":{"key":"KeyQ"}}"#,
            r#"{"kind":"IncomingEvent.Quit","value":null}"#,
            // A tab frame with no url at all.
            r#"{"kind":"IncomingEvent.Tab","value":{}}"#,
            "{}",
            "not json at all",
        ] {
            assert!(
                serde_json::from_str::<IncomingEvent>(frame).is_err(),
                "{frame} should not deserialize"
            );
        }
    }
}
