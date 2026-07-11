//! The binding layer over a laserbeam state tree.
//!
//! A node derives [`Bind`], names its marker with `#[binds(Marker)]`, and lists
//! its bindings with `#[bind(trigger => handler, ..)]`. The derive implements
//! two halves:
//!
//! - [`EventHandler`], whose [`accumulate`](EventHandler::accumulate) gathers the
//!   active trigger set (what the app registers with the OS). [`accumulate()`]
//!   runs it from the root.
//! - [`Dispatch`], which runs the handler the active state binds for a fired
//!   event. [`dispatch()`] runs it from the root, picking the highest-priority
//!   binding across the active path rather than the leafmost by position.
#![allow(clippy::implicit_hasher)]

use std::collections::{HashSet, VecDeque};
use std::hash::Hash;
use std::ops::ControlFlow;

pub use bind_macro::Bind;

/// The marker an app implements on one type to name its trigger, event, and
/// output types.
pub trait Bindings {
    /// The unified enum of every trigger the app can register.
    type Trigger: Eq + Hash;
    /// The unified event the app dispatches.
    type Event;
    /// What a handler returns: the effect data for the consumer to perform.
    type Output;
}

/// The accumulate half. `#[derive(Bind)]` implements it.
pub trait EventHandler<M: Bindings> {
    /// Adds this node's triggers, and those of its active descendants, to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`BindError::DuplicateTrigger`] when a trigger is bound at more
    /// than one node on the active path.
    fn accumulate(&self, out: &mut HashSet<M::Trigger>) -> Result<(), BindError>;
}

/// The error [`accumulate()`] can produce.
#[derive(Debug, PartialEq, Eq)]
pub enum BindError {
    /// A trigger was bound at more than one node on the active path.
    DuplicateTrigger,
}

/// Inserts `t` into `out`, failing when it is already present.
///
/// # Errors
///
/// Returns [`BindError::DuplicateTrigger`] when `t` is already in `out`.
pub fn insert_or_error<T: Eq + Hash>(out: &mut HashSet<T>, t: T) -> Result<(), BindError> {
    if out.insert(t) {
        Ok(())
    } else {
        Err(BindError::DuplicateTrigger)
    }
}

/// Accumulates the active trigger set for the tree rooted at `root`.
///
/// # Errors
///
/// Propagates [`BindError::DuplicateTrigger`] from [`EventHandler::accumulate`].
pub fn accumulate<M, N>(root: &N) -> Result<HashSet<M::Trigger>, BindError>
where
    M: Bindings,
    N: EventHandler<M>,
{
    let mut out = HashSet::new();
    root.accumulate(&mut out)?;
    Ok(out)
}

/// How much a binding wants an event, when it wants it at all.
///
/// Higher wins; ties break leafward, so a more specific node overrides a more
/// general one. Negative is allowed and is how a wildcard sits below the specific
/// keys it overlaps.
pub type Priority = i32;

/// The result of testing one trigger against one event.
///
/// A wildcard like a catch-all key returns `Handle` at a low priority, and a
/// specific key at a higher one, so the specific key wins even when the wildcard
/// is nearer the leaf. That is the whole reason this is not a `bool`: leaf-wins is
/// the wrong rule when the leaf's binding is a wildcard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Match {
    /// The trigger does not apply to this event.
    DontHandle,
    /// The trigger handles this event, at this priority.
    Handle(Priority),
}

/// A trigger matches its source's event. Extracting the source event from the
/// unified event (a `TryFrom<&Event> for &SourceEvent`) is the type match; this
/// is the key match on the source event.
pub trait EventTrigger {
    /// The source event this trigger matches against.
    type Event;
    /// Whether the trigger handles `event`, and at what priority.
    #[must_use]
    fn try_match(&self, event: &Self::Event) -> Match;
}

/// The dispatch half. `#[derive(Bind)]` implements it alongside [`EventHandler`].
///
/// Dispatch is two passes over the active path. [`winner`](Self::winner) reads the
/// tree and returns the highest priority any active binding gives the event.
/// [`dispatch_at`](Self::dispatch_at) then runs the binding that handles at exactly
/// that priority, trying the active child first so that among equal priorities the
/// leafward one wins.
pub trait Dispatch<M: Bindings>: ::laserbeam::Resolve {
    /// The highest priority any binding on the active path gives `event`, or `None`
    /// if none handles it. Reads the tree without mutating it, and hands the path
    /// back so the caller can dispatch with it.
    fn winner<'a>(path: Self::Path<'a>, event: &M::Event) -> (Option<Priority>, Self::Path<'a>)
    where
        Self: 'a;

    /// Runs the active binding that handles `event` at exactly `target`, or hands
    /// the path back on a miss. The active child is tried first, so ties break
    /// leafward.
    fn dispatch_at<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        target: Priority,
    ) -> ControlFlow<M::Output, Self::Path<'a>>
    where
        Self: 'a;
}

/// Dispatches `event` against the tree at `path` (the root's `&mut Root`),
/// returning the handler's output, or `None` when nothing on the active path
/// handles it.
///
/// Finds the winning priority across the whole active path, then runs the binding
/// that handles at it. So a specific binding beats an overlapping wildcard wherever
/// each sits in the tree, rather than the leaf winning by position alone.
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    let (winner, path) = <N as Dispatch<M>>::winner(path, event);
    let target = winner?;
    match <N as Dispatch<M>>::dispatch_at(path, event, target) {
        ControlFlow::Break(out) => Some(out),
        // Unreachable: `winner` found `target`, so a binding handles at it.
        ControlFlow::Continue(_) => None,
    }
}

/// The higher of two optional priorities.
#[must_use]
pub fn merge(a: Option<Priority>, b: Match) -> Option<Priority> {
    match b {
        Match::DontHandle => a,
        Match::Handle(p) => Some(a.map_or(p, |a| a.max(p))),
    }
}

// The real event loop is bespoke: its queue and its wait-when-empty differ per
// consumer (a run loop, a channel), so each writes its own; `dispatch` and
// `accumulate` are the pieces. `SimpleRunner` below is not that loop. It is a
// synchronous driver for tests: process one queued event at a time, and queue
// more (a handler's follow-ups) as you go.

/// A synchronous event runner for tests.
///
/// Queue events, process them one at a time with [`next`](Self::next), and queue
/// more between or during steps (for a handler's follow-up events). It drains
/// rather than waits: an empty queue returns `None`, not a block. The real loop
/// is the consumer's; this one exists to drive the tree in a test.
pub struct SimpleRunner<'a, M: Bindings, N> {
    root: &'a mut N,
    queue: VecDeque<M::Event>,
}

impl<'a, M, N> SimpleRunner<'a, M, N>
where
    M: Bindings,
    N: Dispatch<M> + for<'b> ::laserbeam::Resolve<Path<'b> = &'b mut N>,
{
    /// A runner over the tree rooted at `root`, with an empty queue.
    pub const fn new(root: &'a mut N) -> Self {
        Self {
            root,
            queue: VecDeque::new(),
        }
    }

    /// Queues an event to be processed by a later [`next`](Self::next).
    pub fn queue_event(&mut self, event: M::Event) {
        self.queue.push_back(event);
    }

    /// Processes exactly one queued event. The outer `None` means the queue was
    /// empty; the inner is what [`dispatch`] returned for the event (`None` when
    /// no binding matched, `Some` with the output otherwise).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Option<M::Output>> {
        let event = self.queue.pop_front()?;
        Some(dispatch::<M, N>(&mut *self.root, &event))
    }

    /// Queues `event` and processes one event, returning its output (`None` when
    /// no binding matched). There is no empty case: the queue is non-empty after
    /// queueing, so there is always an event to process.
    ///
    /// The event processed is the front of the queue, which is `event` only when
    /// the queue was empty; if earlier follow-ups are still queued, one of them
    /// runs first.
    ///
    /// # Panics
    ///
    /// Never: the queue is non-empty after queueing; the `expect` asserts it.
    pub fn process_event(&mut self, event: M::Event) -> Option<M::Output> {
        // Field ops inlined rather than calling `queue_event`/`next`, which the
        // impl's HRTB bound would otherwise force to `'static`.
        self.queue.push_back(event);
        let event = self
            .queue
            .pop_front()
            .expect("the queue is non-empty: an event was just queued");
        dispatch::<M, N>(&mut *self.root, &event)
    }
}

impl<M: Bindings, N> SimpleRunner<'_, M, N> {
    /// The number of queued events not yet processed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{Match, merge};

    #[test]
    fn merge_ignores_a_non_match() {
        assert_eq!(merge(None, Match::DontHandle), None);
        assert_eq!(merge(Some(3), Match::DontHandle), Some(3));
    }

    #[test]
    fn merge_takes_a_match_when_there_was_nothing() {
        assert_eq!(merge(None, Match::Handle(2)), Some(2));
    }

    #[test]
    fn merge_keeps_the_higher_priority() {
        assert_eq!(merge(Some(5), Match::Handle(2)), Some(5));
        assert_eq!(merge(Some(2), Match::Handle(5)), Some(5));
    }

    // None is below every Some, so a real match always beats no candidate, which is
    // what `winner` relies on when folding child priorities with `Ord::max`.
    #[test]
    fn no_candidate_is_below_any_candidate() {
        assert!(None < Some(i32::MIN));
        assert_eq!(Ord::max(None, Some(-1)), Some(-1));
    }
}
