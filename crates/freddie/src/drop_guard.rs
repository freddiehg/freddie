//! A guard whose drop cancels a paired async job.

use tokio::sync::oneshot;

/// The cancelling half of a drop pair.
///
/// The owning node holds it; dropping it (a transition that replaces the node, or a clobber that
/// overwrites it) closes the channel and wakes the paired receiver, so whatever waits on that
/// receiver tears down at once.
///
/// A pure RAII primitive: it knows nothing about testing equality. A consumer that needs a
/// comparable effect wraps its own half in `AlwaysEqual` (see [`TimerEffect`](crate::TimerEffect)).
#[must_use = "dropping the guard cancels immediately"]
pub struct DropGuard(
    // Held only to be dropped: dropping the sender wakes the paired receiver. Never read.
    #[expect(dead_code)] oneshot::Sender<()>,
);

impl std::fmt::Debug for DropGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DropGuard")
    }
}

/// Build a linked guard/receiver pair. The guard goes in the node; the receiver rides an effect to
/// whatever performs the cancellable job.
pub fn drop_guard() -> (DropGuard, oneshot::Receiver<()>) {
    let (sender, receiver) = oneshot::channel();
    (DropGuard(sender), receiver)
}
