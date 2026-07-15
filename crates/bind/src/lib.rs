//! The binding layer over a laserbeam state tree.
//!
//! A node derives [`Bind`], names its marker with `#[binds(Marker)]`, and lists
//! its bindings with `#[bind(trigger => handler, ..)]`. The derive implements
//! two halves:
//!
//! There are two halves, and only one of them ships.
//!
//! [`Dispatch`] runs the handler the active state binds for a fired event. It is what a
//! keystroke costs. [`dispatch()`] runs it from the root.
//!
//! [`EventHandler`] is THE CHECK, behind the `check` feature. It walks the same tree and
//! collects every live bind's trigger into a set, erroring on a collision. It is a test.
//! Nothing in a shipped binary calls it: the keyboard tap subscribes to event TYPES, not to
//! individual keys, so there is no trigger set to register and no reason for it to exist at
//! runtime.
//!
//! With `default-features = false` the check does not exist. [`EventHandler`],
//! [`accumulate()`], and [`BindError`] are not compiled, and `#[derive(Bind)]` emits no
//! `EventHandler` impl, because it wraps that impl in [`check_only!`].
#![allow(clippy::implicit_hasher)]

#[cfg(feature = "check")]
use std::collections::HashSet;
use std::collections::VecDeque;
use std::hash::Hash;
use std::ops::ControlFlow;

pub use bind_macro::Bind;

/// Emits its body only when the `check` feature is on.
///
/// A derive cannot see the features of the crate it expands into, so it cannot cfg the check
/// away itself. It emits `::bind::check_only! { .. }` instead, and this macro, which IS
/// compiled with `bind`'s features, keeps or drops the body.
#[cfg(feature = "check")]
#[macro_export]
macro_rules! check_only {
    ($($t:tt)*) => { $($t)* };
}

/// Drops its body: the `check` feature is off, so the check does not exist.
#[cfg(not(feature = "check"))]
#[macro_export]
macro_rules! check_only {
    ($($t:tt)*) => {};
}

/// The marker an app implements on one type to name its trigger, event, and
/// output types.
pub trait Bindings {
    /// The unified enum of every trigger the app can bind.
    ///
    /// Only the check uses it, and it cannot be cfg'd away: a consumer implements `Bindings`
    /// and cannot see `bind`'s features, so an associated type that came and went would not
    /// compile for them.
    type Trigger: Eq + Hash;
    /// The unified event the app dispatches.
    type Event;
    /// What a handler returns: the effect data for the consumer to perform.
    type Output;
}

/// The accumulate half. `#[derive(Bind)]` implements it.
///
/// It takes a path rather than `&self`, for the same reason [`Dispatch`] does: a level whose
/// child is produced by a function reaches it by CALLING that function, and the function
/// needs a path. With `&self` there is no path, so such a level's binds are invisible to the
/// trigger set, which is the one thing the trigger set exists to be complete about.
///
/// It hands the path back, again like [`Dispatch`], because a node that has descended still
/// has its own triggers to insert.
#[cfg(feature = "check")]
pub trait EventHandler<M: Bindings>: Place {
    /// Adds this node's triggers, and those of its active descendants, to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`BindError::DuplicateTrigger`] when a trigger is bound at more
    /// than one node on the active path.
    fn accumulate<'a>(
        path: Self::Path<'a>,
        out: &mut HashSet<M::Trigger>,
    ) -> Result<Self::Path<'a>, BindError>
    where
        Self: 'a;
}

/// The error [`accumulate()`] can produce.
#[cfg(feature = "check")]
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
#[cfg(feature = "check")]
pub fn insert_or_error<T: Eq + Hash>(out: &mut HashSet<T>, t: T) -> Result<(), BindError> {
    if out.insert(t) {
        Ok(())
    } else {
        Err(BindError::DuplicateTrigger)
    }
}

/// Accumulates the active trigger set for the tree at `path` (the root's `&mut Root`).
///
/// # Errors
///
/// Propagates [`BindError::DuplicateTrigger`] from [`EventHandler::accumulate`].
#[cfg(feature = "check")]
pub fn accumulate<'a, M, N>(path: N::Path<'a>) -> Result<HashSet<M::Trigger>, BindError>
where
    M: Bindings,
    N: EventHandler<M> + 'a,
{
    let mut out = HashSet::new();
    <N as EventHandler<M>>::accumulate(path, &mut out)?;
    Ok(out)
}

/// A place in the tree, and its path type. `#[derive(Bind)]` implements it for every node that
/// IS in the tree, from the node's `#[laserbeam(path = P)]` or `#[laserbeam_root]`.
///
/// This is the one associated type dispatch needs, and it lives here rather than in laserbeam
/// so that bind depends on laserbeam's TYPES (`laserbeam::Path`) but on none of its traits. A
/// DERIVED level is not a place: it has no path, so it does not implement `Place`.
pub trait Place {
    /// This node's path type. The root's is `&'a mut Self`; every other node's is its declared
    /// `laserbeam::Path` alias.
    type Path<'a>
    where
        Self: 'a;
}

/// What a handler is given: a parent, plus the immutable data this level produced.
///
/// `data` is `()` for every level that is a place in the tree, and it is zero-sized, so a
/// place pays nothing for the field. A level that is NOT in the tree puts an object there.
///
/// `parent` is a [`laserbeam::Path`](::laserbeam::Path) when the level above is a place, so
/// `node.parent.get_mut()` reaches it. A `Path` ADDRESSES a place; this type CARRIES data.
/// They both sit next to a parent, and that is the whole of the resemblance.
pub struct Node<Parent, Data> {
    /// What the level above handed down.
    pub parent: Parent,
    /// The immutable data this level produced.
    pub data: Data,
}

/// How a generated impl reaches the parent's type without naming it.
///
/// A node's derive sees one struct's tokens. When it descends into a child produced by a
/// function it cannot know that function's return type, so it asks for `Self::Parent`
/// instead of writing it.
pub trait HasParent {
    /// The parent's type: a [`laserbeam::Path`](::laserbeam::Path) when the level above is a
    /// place, a [`Node`] when it is derived.
    type Parent;
    /// Consumes this node and returns the parent, moving one level up.
    fn into_parent(self) -> Self::Parent;
}

impl<Parent, Data> HasParent for Node<Parent, Data> {
    type Parent = Parent;
    fn into_parent(self) -> Parent {
        self.parent
    }
}

impl<N, P> HasParent for ::laserbeam::Path<N, P> {
    type Parent = P;
    fn into_parent(self) -> P {
        ::laserbeam::Path::into_parent(self)
    }
}

/// A trigger matches its source's event. Extracting the source event from the
/// unified event (a `TryFrom<&Event> for &SourceEvent`) is the type match; this
/// is the key match on the source event.
pub trait EventTrigger {
    /// The source event this trigger matches against.
    type Event;
    /// Whether the trigger matches `event`.
    #[must_use]
    fn is_matching(&self, event: &Self::Event) -> bool;
}

/// ONE descent, whatever the child is.
///
/// A PLACE implements it by delegating to its own [`Dispatch`] and then handing the parent
/// back. A DERIVED level implements it directly, because it has no [`Resolve`] and so cannot
/// have `Dispatch`.
///
/// It exists because a node's derive cannot name the return type of a function that produces
/// its child. It calls this on whatever that function returned, and inference finds the impl.
///
/// The place impl is emitted PER NODE by the derive, not once as a blanket
/// `impl<N, P> Descend<M> for Path<N, P>`: `Dispatch` carries `Self: 'a`, and the HRTB needed
/// to state the blanket is E0311.
pub trait Descend<M: Bindings>: HasParent + Sized {
    /// Runs the active binding for `event`, or hands the PARENT back on a miss.
    fn dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>;
}

/// A derived level's half of THE CHECK. It does not ship, for the same reason
/// [`EventHandler`] does not.
///
/// A derived level has no [`Place`], so it cannot implement
/// `EventHandler`, whose signature is written in terms of `Self::Path`. It carries its
/// triggers here instead.
#[cfg(feature = "check")]
pub trait DerivedHandler<M: Bindings>: HasParent + Sized {
    /// Adds this level's triggers to `out` and hands the PARENT back.
    ///
    /// # Errors
    ///
    /// Returns [`BindError::DuplicateTrigger`] when a trigger is already claimed.
    fn accumulate(self, out: &mut HashSet<M::Trigger>) -> Result<Self::Parent, BindError>;
}

/// The dispatch half. `#[derive(Bind)]` implements it alongside [`EventHandler`].
///
/// Each node tries its active child first, then its own binds, so a child's
/// binding takes priority over an ancestor's. [`Break`](ControlFlow::Break)
/// carries the handler's output up; [`Continue`](ControlFlow::Continue) hands the
/// node's path back so the parent can walk up (`into_parent`) and take its turn.
pub trait Dispatch<M: Bindings>: Place {
    /// Runs the active binding for `event`, or hands the path back on a miss.
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
    ) -> ControlFlow<M::Output, Self::Path<'a>>
    where
        Self: 'a;
}

/// Dispatches `event` against the tree at `path` (the root's `&mut Root`),
/// returning the handler's output, or `None` when nothing on the active path
/// binds it.
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    match <N as Dispatch<M>>::dispatch(path, event) {
        ControlFlow::Break(out) => Some(out),
        ControlFlow::Continue(_) => None,
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
    N: Dispatch<M> + for<'b> Place<Path<'b> = &'b mut N>,
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
