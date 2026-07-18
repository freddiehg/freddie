//! What mercury can be told from outside the process, over `freddie_event_socket`.
//!
//! The transport is generic and the vocabulary is mercury's, the same split `freddie_app_nav` uses:
//! the socket hands up a frame as a `&str`, and [`on_message`] decides what it means.

use tracing::warn;

/// The port mercury listens on when nothing overrides it. Hardcoded in the extension too.
///
/// Mercury's orbital period, 87.969 days, truncated to fit a `u16`. Below 49152, which is where
/// macOS starts handing out ephemeral ports (`net.inet.ip.portrange.first`): a listener up there
/// can find its port already taken by some outbound socket that grabbed it first. Unassigned in
/// `/etc/services`.
pub const DEFAULT_PORT: u16 = 8797;

/// Everything an outside process may say to mercury. A sender cannot say anything else, so remote
/// key injection and remote quit are unrepresentable rather than filtered.
///
/// `MercuryEvent` deliberately does not derive `Deserialize`: deriving it would make
/// `MercuryEvent::Key` and `MercuryEvent::Quit` constructible from the wire, and "no remote
/// keyboard, no remote kill" would be a rule some match arm enforces rather than something the
/// types say.
///
/// Empty, so every frame is refused with ``unknown variant `IncomingEvent.Tab`, there are no
/// variants``. Variants arrive with the features that want them.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum IncomingEvent {}

/// Turn one frame into an event. Runs on the socket's runtime, so it parses, dispatches, and
/// returns.
///
/// A frame that is not a valid [`IncomingEvent`] is logged and dropped: a client speaking nonsense
/// is a client bug, not a reason to tear the connection down. With [`IncomingEvent`] empty that is
/// every frame, and connecting and being ignored is what a client should see today.
pub fn on_message(text: &str) {
    match serde_json::from_str::<IncomingEvent>(text) {
        // `IncomingEvent` has no variants, so this value cannot exist and the arm is empty. The
        // first variant to land breaks this line, which is how the compiler asks where the event
        // goes; it takes an `event_tx` from then on.
        Ok(never) => match never {},
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}

#[cfg(test)]
mod tests {
    use super::IncomingEvent;

    #[test]
    fn every_frame_is_refused_while_the_vocabulary_is_empty() {
        for frame in [
            r#"{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}"#,
            r#"{"kind":"MercuryEvent.Key","value":{"key":"KeyQ"}}"#,
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
