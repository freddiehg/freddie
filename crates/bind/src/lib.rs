//! The binding layer over a laserbeam state tree.
//!
//! A node derives [`Bind`], names its marker with `#[binds(Marker)]`, and lists
//! its bindings with `#[bind(trigger => handler, ..)]`. The derive implements
//! two halves:
//!
//! A trigger is usually a value, `Key::KeyR` or `Quit`. It may instead be a CLOSURE, which the
//! derive calls with the state the node is bound on, for a trigger that depends on it: a place
//! node's closure is handed `&Self::Path`, so the root's reads its fields directly and a deeper
//! node's reads through `get` and `parent`, and a derived level's is handed `&Node`. It is shared,
//! so a trigger cannot write what it reads. The two forms are told apart syntactically, since a
//! trait cannot do it: blanket impls for values and for closures overlap.
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
#![expect(clippy::implicit_hasher)]

#[cfg(feature = "check")]
use std::collections::HashSet;
use std::collections::VecDeque;
use std::hash::Hash;

pub use bind_macro::Bind;

/// One exclusive bind handler per dispatch: the first to [`try_take`](Self::try_take) wins.
///
/// A newtype over the slot rather than a bare `&mut Option<()>`, because the
/// bare reference would let a handler write the slot directly: un-claim a taken
/// key with `*slot = None`, or claim without the check. The private field
/// confines writes to `try_take`, so a claim, once taken, is taken for the rest
/// of the dispatch. The price is [`reborrow`](Self::reborrow): a bare `&mut`
/// reborrows implicitly at call sites, a newtype does not.
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}

impl<'c> Claim<'c> {
    #[must_use]
    pub const fn new(slot: &'c mut Option<()>) -> Self {
        Self { slot }
    }

    #[must_use]
    pub const fn is_taken(&self) -> bool {
        self.slot.is_some()
    }

    pub const fn try_take(&mut self) -> Option<()> {
        if self.slot.is_some() {
            None
        } else {
            *self.slot = Some(());
            Some(())
        }
    }

    /// The per-item reborrow the generated code hands to each [`AscendState`].
    ///
    /// One claim serves every scheduled item of every node on the path, and each
    /// item's [`AscendState`] takes a `Claim` by value, which its handler
    /// consumes. Dispatch holds only `&mut Claim` and cannot move out of it, and
    /// lifetime coercion cannot substitute (`Claim` is covariant in `'c`, but a
    /// shorter `Claim` is still one consumable value), so this mints a fresh
    /// `Claim` over the same slot per item: borrowck suspends the original while
    /// the mint lives, and the mint dies with its item, freeing the next.
    pub const fn reborrow(&mut self) -> Claim<'_> {
        Claim {
            slot: &mut *self.slot,
        }
    }
}

/// What every scheduled handler receives beside the event.
///
/// One lifetime: the claim rides by value, reborrowed per item, and the state is
/// what the items before this one left behind.
pub struct AscendState<'a, P: ::laserbeam::HasStop> {
    claim: Claim<'a>,
    /// Whether the descent, and every item scheduled before this one, destroyed
    /// the path this handler is bound on.
    pub state: ::laserbeam::MaybeInvalidated<P>,
}

impl<'a, P: ::laserbeam::HasStop> AscendState<'a, P> {
    #[must_use]
    pub const fn new(state: ::laserbeam::MaybeInvalidated<P>, claim: Claim<'a>) -> Self {
        Self { claim, state }
    }

    /// `Some(())`: you won the claim. `None`: someone already has it.
    pub const fn claim(&mut self) -> Option<()> {
        self.claim.try_take()
    }

    /// Stay where you are: the leave this state completes to.
    #[must_use]
    pub fn complete(self) -> ::laserbeam::Completed<P>
    where
        P: ::laserbeam::Complete<P>,
    {
        self.state.complete()
    }
}

/// The claim gate, shape-preserving: `handler` runs iff the claim is won, and
/// otherwise the state completes where it stands.
///
/// `#[bind]` desugars through this, and nothing else does: a post is scheduled by
/// its trigger and runs whether or not anything claimed. Winning the claim says
/// nothing about whether the path survived, which is why the state a handler
/// matches on is the same either way.
pub fn exclusive<Ev, Snap, P, E, H>(
    handler: H,
) -> impl for<'a> FnOnce(Ev, Snap, AscendState<'a, P>) -> (Vec<E>, ::laserbeam::Completed<P>)
where
    P: ::laserbeam::HasStop + ::laserbeam::Complete<P>,
    H: for<'a> FnOnce(Ev, Snap, AscendState<'a, P>) -> (Vec<E>, ::laserbeam::Completed<P>),
{
    move |ev, snap, mut st| match st.claim() {
        Some(()) => handler(ev, snap, st),
        None => (Vec::new(), st.complete()),
    }
}

/// Runs `a` then `b` as one handler: one claim, effects in order, `b` receiving
/// the state `a` left behind.
///
/// The schedule's fold, at expression level, so one user action composes from
/// units at its bind site: `#[bind(K => and!(tap_cmd_l, enter_typing))]`. It
/// nests, and it claims nothing itself — `#[bind]` wraps the outermost
/// expression in [`exclusive`], so the whole composition takes the one claim and
/// no unit takes its own.
///
/// Both units are handed the same event and the same snap, hence the `Copy`
/// bounds; in bind position the snap is `()`.
pub fn and<Ev, Snap, P, E, A, B>(
    a: A,
    b: B,
) -> impl for<'x> FnOnce(Ev, Snap, AscendState<'x, P>) -> (Vec<E>, ::laserbeam::Completed<P>)
where
    Ev: Copy,
    Snap: Copy,
    P: ::laserbeam::HasStop,
    A: for<'x> FnOnce(Ev, Snap, AscendState<'x, P>) -> (Vec<E>, ::laserbeam::Completed<P>),
    B: for<'x> FnOnce(Ev, Snap, AscendState<'x, P>) -> (Vec<E>, ::laserbeam::Completed<P>),
{
    move |ev, snap, st| {
        let AscendState { mut claim, state } = st;
        let (mut effs, completed) = a(ev, snap, AscendState::new(state, claim.reborrow()));
        let state = completed.to_maybe_invalidated();
        let (e, completed) = b(ev, snap, AscendState::new(state, claim));
        effs.extend(e);
        (effs, completed)
    }
}

/// `and!(a, b, c)` is `and(a, and(b, c))`: the flat spelling of a gesture's unit list.
///
/// A macro over the fn rather than a collection, so each unit keeps its own type: closures and
/// parameterized units like `tmux_window(1)` survive, where a slice would force one element type
/// and `&dyn` would buy vtables.
#[macro_export]
macro_rules! and {
    ($h:expr) => { $h };
    ($h:expr, $($rest:expr),+ $(,)?) => {
        $crate::and($h, $crate::and!($($rest),+))
    };
}

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
    /// What dispatch returns: the effect data for the consumer to perform.
    ///
    /// A handler returns any `IntoIterator` this collects from, which is what lets one handler
    /// produce a whole `Vec<Effect>` and the next produce a single effect. The consumer owns
    /// the `IntoIterator` impls, so what a handler may return is its choice rather than this
    /// crate's.
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
/// so that bind depends on laserbeam's TYPES (`laserbeam::PathMut`) but on none of its traits. A
/// DERIVED level is not a place: it has no path, so it does not implement `Place`.
pub trait Place {
    /// This node's path type. The root's is `&'a mut Self`; every other node's is its declared
    /// `laserbeam::PathMut` alias.
    type Path<'a>
    where
        Self: 'a;
}

/// What a handler is given: a parent, plus the immutable data this level produced.
///
/// `data` is `()` for every level that is a place in the tree, and it is zero-sized, so a
/// place pays nothing for the field. A level that is NOT in the tree puts an object there.
///
/// `parent` is a [`laserbeam::PathMut`](::laserbeam::PathMut) when the level above is a place, so
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
    /// The parent's type: a [`laserbeam::PathMut`](::laserbeam::PathMut) when the level above is a
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

impl<N, P> HasParent for ::laserbeam::PathMut<N, P> {
    type Parent = P;
    fn into_parent(self) -> P {
        Self::into_parent(self)
    }
}

/// The place path at the bottom of a parent chain: what a level ASCENDS at.
///
/// A place is its own; a [`Node`] flattens to its parent's, however many derived levels are
/// stacked. That is what lets one handler shape serve both: ascent holds a path into the tree,
/// never a `Node`, whose `data` is rebuilt from the tree on every dispatch and dies with it.
pub trait HasPlace {
    /// The place path this chain bottoms out at.
    type Place;
    /// Consumes this chain and returns that path, dropping every derived level's data.
    fn into_place(self) -> Self::Place;
}

impl<R> HasPlace for &mut R {
    type Place = Self;
    fn into_place(self) -> Self {
        self
    }
}

impl<N, P> HasPlace for ::laserbeam::PathMut<N, P> {
    type Place = Self;
    fn into_place(self) -> Self {
        self
    }
}

impl<Parent: HasPlace, Data> HasPlace for Node<Parent, Data> {
    type Place = Parent::Place;
    fn into_place(self) -> Parent::Place {
        self.parent.into_place()
    }
}

/// Calls a closure trigger with the state its binding is bound on.
///
/// The macro emits this rather than `(#closure)(state)`, because a closure parameter is not
/// inferred from an immediate call: it takes its type from an EXPECTED type, which this function's
/// signature supplies. Without it every state-reading binding would have to annotate its own
/// parameter with a path type it should not have to name.
pub fn call_with<S: ?Sized, T>(state: &S, f: impl FnOnce(&S) -> T) -> T {
    f(state)
}

/// A trigger matches its source's event. Extracting the source event from the
/// unified event (a `TryFrom<&Event> for &SourceEvent`) is the type match; this
/// is the key match on the source event.
pub trait EventTrigger {
    /// The source event this trigger matches against.
    type Event;
    #[must_use]
    fn is_matching(&self, event: &Self::Event) -> bool;
}

/// A trigger that may be absent, which matches nothing when it is.
///
/// A trigger read from state has to produce a value even when the state holds none, and this is
/// that value: `None` answers no to every event, so a binding reads
/// `some_state.timer().map(TimerGuard::trigger)` and nothing branches on absence.
///
/// One impl for one type constructor, so nothing overlaps. It does claim `Option` for every
/// consumer: no crate can implement `EventTrigger` for an `Option` of its own trigger afterwards,
/// since coherence will not reason about whether the inner type qualifies. The meaning imposed
/// here is the only sensible one.
impl<T: EventTrigger> EventTrigger for Option<T> {
    type Event = T::Event;
    fn is_matching(&self, event: &T::Event) -> bool {
        self.as_ref().is_some_and(|t| t.is_matching(event))
    }
}

/// Implements [`EventTrigger`] for a type that is its own event and matches by [`PartialEq`].
///
/// For unit signals (`Quit`) equality is always true (one value). For tag enums
/// (`DeviceClass`) variants discriminate so a filter only matches its own class.
///
/// `$t` must implement [`PartialEq`].
#[macro_export]
macro_rules! self_trigger {
    ($t:ty) => {
        impl $crate::EventTrigger for $t {
            type Event = Self;
            fn is_matching(&self, event: &Self) -> bool {
                self == event
            }
        }
    };
}

/// Consumes a derived node, dispatches at that level, and surfaces at the place path beneath it.
///
/// The derived counterpart of [`Dispatch`], and what replaces [`DispatchIntoParent`]: a derived
/// level's scheduled items ascend at [`HasPlace::Place`], so its caller folds what comes back
/// exactly as it folds a place child's leave.
///
/// It exists because a derived-child caller cannot name the child's type. It calls this in
/// method position on whatever the child function returned, and inference finds the impl.
pub trait DispatchIntoPlace<M: Bindings>: HasPlace + Sized
where
    Self::Place: ::laserbeam::HasStop,
{
    /// Runs this level's scheduled items for `event` into `effs` under `claim`, and returns
    /// the leave they completed to at the place beneath.
    fn dispatch_into_place(
        self,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'_>,
    ) -> ::laserbeam::Completed<Self::Place>;
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
/// Each node descends into its active child first, then runs its own scheduled items, so a
/// child's binding takes priority over an ancestor's. Effects collect into `effs`; the first
/// exclusive handler takes `claim`.
///
/// What comes back says where the leave this dispatch produced stopped: at this path, or
/// somewhere above it. Every item runs either way, so nothing here is control flow.
pub trait Dispatch<M: Bindings>: Place {
    /// Runs this node's scheduled items for `event` into `effs` under `claim`, and returns the
    /// leave they completed to.
    fn dispatch<'a, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> ::laserbeam::Completed<Self::Path<'a>>
    where
        Self: 'a,
        Self::Path<'a>: ::laserbeam::HasStop;
}

/// Dispatches `event` against the tree at `path` (the root's `&mut Root`),
/// returning what it produced and whether anything claimed the event.
///
/// The effects are always the caller's to perform: a scheduled item runs on its
/// trigger, not on the claim, so an event nothing claimed can still have produced
/// some. The bool is the only answer to "was this handled", which is what a
/// consumer that decides whether to pass the event on asks.
pub fn dispatch<'a, M, N, E>(path: N::Path<'a>, event: &M::Event) -> (Vec<E>, bool)
where
    M: Bindings<Output = Vec<E>>,
    N: Dispatch<M> + 'a,
    N::Path<'a>: ::laserbeam::HasStop,
{
    let mut effs: Vec<E> = Vec::new();
    let mut claim_slot = None;
    let mut claim = Claim::new(&mut claim_slot);
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
    (effs, claim.is_taken())
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

impl<'a, M, N, E> SimpleRunner<'a, M, N>
where
    M: Bindings<Output = Vec<E>>,
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

    /// Processes exactly one queued event. `None` means the queue was empty;
    /// otherwise it is what [`dispatch`] returned for the event: its effects, and
    /// whether anything claimed it.
    #[expect(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<(Vec<E>, bool)> {
        let event = self.queue.pop_front()?;
        Some(dispatch::<M, N, E>(&mut *self.root, &event))
    }

    /// Queues `event` and processes one event, returning its effects and whether
    /// anything claimed it. There is no empty case: the queue is non-empty after
    /// queueing, so there is always an event to process.
    ///
    /// The event processed is the front of the queue, which is `event` only when
    /// the queue was empty; if earlier follow-ups are still queued, one of them
    /// runs first.
    ///
    /// # Panics
    ///
    /// Never: the queue is non-empty after queueing; the `expect` asserts it.
    pub fn process_event(&mut self, event: M::Event) -> (Vec<E>, bool) {
        // Field ops inlined rather than calling `queue_event`/`next`, which the
        // impl's HRTB bound would otherwise force to `'static`.
        self.queue.push_back(event);
        let event = self
            .queue
            .pop_front()
            .expect("the queue is non-empty: an event was just queued");
        dispatch::<M, N, E>(&mut *self.root, &event)
    }
}

impl<M: Bindings, N> SimpleRunner<'_, M, N> {
    /// The number of queued events not yet processed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod has_place_tests {
    use super::{HasPlace, Node};
    use laserbeam::PathMut;

    struct Root {
        layer: u32,
    }

    fn path(root: &mut Root) -> PathMut<u32, &mut Root> {
        PathMut::from_fn(root, |r| &mut r.layer, |r| &r.layer)
    }

    #[test]
    fn a_root_path_is_its_own_place() {
        let mut root = Root { layer: 7 };
        let place: &mut Root = HasPlace::into_place(&mut root);
        place.layer = 8;
        assert_eq!(root.layer, 8);
    }

    #[test]
    fn a_path_mut_is_its_own_place() {
        let mut root = Root { layer: 7 };
        {
            let mut place: PathMut<u32, &mut Root> = HasPlace::into_place(path(&mut root));
            *place.get_mut() = 9;
        }
        assert_eq!(root.layer, 9);
    }

    #[test]
    fn a_node_flattens_to_its_parent_path() {
        let mut root = Root { layer: 7 };
        {
            let node = Node {
                parent: path(&mut root),
                data: "derived",
            };
            let mut place: PathMut<u32, &mut Root> = HasPlace::into_place(node);
            *place.get_mut() = 10;
        }
        assert_eq!(root.layer, 10);
    }

    #[test]
    fn two_node_layers_flatten_to_the_same_place() {
        let mut root = Root { layer: 7 };
        {
            let node = Node {
                parent: Node {
                    parent: path(&mut root),
                    data: "outer",
                },
                data: 3_u8,
            };
            let mut place: PathMut<u32, &mut Root> = HasPlace::into_place(node);
            *place.get_mut() = 11;
        }
        assert_eq!(root.layer, 11);
    }
}
