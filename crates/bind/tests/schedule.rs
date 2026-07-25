//! The schedule, end to end: the `A → B` demo from `invalidation.md`, plus the cases its two
//! walks do not reach.
//!
//! `A` is the root and holds the layer `B`. `B` arms a return-home timer; every key while `B` is
//! up pushes the deadline out; a leave has to cancel it, because the OS timer outlives the active
//! path and `Drop` cannot emit the cancel. One post owns the whole deadline story by matching the
//! state, and it is scheduled before the bind because it keys on what the descent did.

use bind::{AscendState, Bind, Bindings, EventTrigger, and, dispatch};
use laserbeam::{Complete, Completed, HasStop, IntoAncestor, MaybeInvalidated, PathMut};

// ---- what the app owns: events, triggers, effects ----

pub struct KeyEvent {
    pub key: &'static str,
}

/// Matches whatever key arrived, which is what a deadline post wants.
pub struct AnyKey;
impl EventTrigger for AnyKey {
    type Event = KeyEvent;
    fn is_matching(&self, _ev: &KeyEvent) -> bool {
        true
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Key(pub &'static str);
impl EventTrigger for Key {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool {
        self.0 == ev.key
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DemoTrigger {
    Key(Key),
}
impl From<Key> for DemoTrigger {
    fn from(k: Key) -> Self {
        Self::Key(k)
    }
}

pub enum DemoEvent {
    Key(KeyEvent),
}
impl<'a> TryFrom<&'a DemoEvent> for &'a KeyEvent {
    type Error = ();
    fn try_from(e: &'a DemoEvent) -> Result<Self, ()> {
        let DemoEvent::Key(k) = e;
        Ok(k)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimerId(pub u64);

impl TimerId {
    /// The next id the OS would mint. Fixed here, so a walk can name it.
    const fn fresh() -> Self {
        Self(1)
    }
}

#[derive(Debug)]
pub struct TimerGuard {
    pub id: TimerId,
}

#[derive(PartialEq, Eq, Debug)]
pub enum DemoEffect {
    ScheduleTimer(TimerId),
    CancelTimer(TimerId),
    FlashOverlay,
    /// What state a witness post was handed, so a test can see the fold.
    SawStanding,
    SawInvalidated,
}

pub struct M;
impl Bindings for M {
    type Trigger = DemoTrigger;
    type Event = DemoEvent;
    type Output = Vec<DemoEffect>;
}

const fn key(k: &'static str) -> DemoEvent {
    DemoEvent::Key(KeyEvent { key: k })
}

// ---- the demo tree ----

#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[pre_post(AnyKey => (snap_return_home, return_home_deadline))] // opt_0
#[bind(Key("esc") => flash)] // opt_1
pub struct A {
    #[resolve_into]
    pub b: B,
}

#[derive(Bind)]
#[node(parent = APath)]
#[binds(M)]
#[bind(Key("h") => go_home, Key("bump") => bump_timer)]
pub struct B {
    pub return_home: TimerGuard,
}

pub type APath<'a> = &'a mut A;
pub type BPath<'a> = PathMut<B, APath<'a>>;

/// B's bind: go home. Depth-generic and branch-free, because what it means is that the dispatch
/// ends at the root wherever the descent below it stopped.
fn go_home<'x, P>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<DemoEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<APath<'x>>,
    APath<'x>: Complete<P>,
{
    (vec![], st.state.into_ancestor::<APath<'x>>().complete())
}

/// B's other bind: replace the timer and stay. Only the pre-snap test fires it, to show that
/// what the post cancels is the id that was live BEFORE the descent ran.
fn bump_timer<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, BPath<'x>>,
) -> (Vec<DemoEffect>, Completed<BPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut b) => {
            b.get_mut().return_home = TimerGuard { id: TimerId(99) };
            (vec![], b.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}

/// A's bind.
fn flash<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, APath<'x>>,
) -> (Vec<DemoEffect>, Completed<APath<'x>>) {
    (vec![DemoEffect::FlashOverlay], st.complete())
}

/// A's pre: runs before descending into B, while the old timer id is live.
const fn snap_return_home(_ev: &KeyEvent, a: &APath<'_>) -> TimerId {
    a.b.return_home.id
}

/// A's post: the whole return-home deadline. B on the active path pushes the deadline out;
/// invalidated, the snap is all that is left of the timer.
fn return_home_deadline<'x>(
    _ev: &KeyEvent,
    snapped: TimerId,
    st: AscendState<'_, APath<'x>>,
) -> (Vec<DemoEffect>, Completed<APath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(a) => {
            let fresh = TimerId::fresh();
            a.b.return_home = TimerGuard { id: fresh };
            (
                vec![
                    DemoEffect::CancelTimer(snapped),
                    DemoEffect::ScheduleTimer(fresh),
                ],
                a.complete(),
            )
        }
        MaybeInvalidated::Invalidated(c) => (vec![DemoEffect::CancelTimer(snapped)], c),
    }
}

const fn demo(id: u64) -> A {
    A {
        b: B {
            return_home: TimerGuard { id: TimerId(id) },
        },
    }
}

// ---- the two walks ----

/// `h`: B goes home. The claim is won at B and the leave peels past A, so A's post takes its
/// invalidated arm and the snap is the only trace of the timer left to cancel. A's bind never
/// fires: `esc` did not arrive, and the claim is gone besides.
#[test]
fn key_h_leaves_and_the_post_cancels_from_its_snap() {
    let mut a = demo(7);
    assert_eq!(
        dispatch::<M, A, _>(&mut a, &key("h")),
        (vec![DemoEffect::CancelTimer(TimerId(7))], true)
    );
    assert_eq!(
        a.b.return_home.id,
        TimerId(7),
        "the invalidated arm rearms nothing"
    );
}

/// Any other key: B stays, so the post pushes the deadline out. Nothing claimed, and the effects
/// come back anyway, which is the whole reason dispatch stopped reporting them through the claim.
#[test]
fn an_unclaimed_key_still_rearms() {
    let mut a = demo(7);
    assert_eq!(
        dispatch::<M, A, _>(&mut a, &key("x")),
        (
            vec![
                DemoEffect::CancelTimer(TimerId(7)),
                DemoEffect::ScheduleTimer(TimerId(1)),
            ],
            false
        )
    );
    assert_eq!(a.b.return_home.id, TimerId(1), "the post rearmed it");
}

/// `esc`: the post runs on its own trigger, then the bind runs on its own, in source order.
#[test]
fn a_post_and_a_bind_both_run_in_source_order() {
    let mut a = demo(7);
    assert_eq!(
        dispatch::<M, A, _>(&mut a, &key("esc")),
        (
            vec![
                DemoEffect::CancelTimer(TimerId(7)),
                DemoEffect::ScheduleTimer(TimerId(1)),
                DemoEffect::FlashOverlay,
            ],
            true
        )
    );
}

/// The pre reads the state as it stood on the way down. `bump` replaces the timer during the
/// descent, and the post still cancels the id the pre took before it.
#[test]
fn a_pre_snaps_before_the_descent_mutates() {
    let mut a = demo(7);
    let (effects, claimed) = dispatch::<M, A, _>(&mut a, &key("bump"));
    assert!(claimed);
    assert_eq!(
        effects,
        vec![
            DemoEffect::CancelTimer(TimerId(7)),
            DemoEffect::ScheduleTimer(TimerId(1)),
        ],
        "the snap is the id from before the descent, not the 99 it wrote"
    );
    assert_eq!(a.b.return_home.id, TimerId(1));
}

// ---- the claim trap door ----

/// A key bound at two depths. Nothing bans it statically: the claim resolves it, deepest first.
#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[bind(Key("t") => trap_root)]
pub struct Trap {
    pub open: bool,
    #[resolve_into]
    pub child: TrapChild,
}

#[derive(Bind)]
#[node(parent = TrapPath)]
#[binds(M)]
#[bind(|p: &TrapChildPath| p.parent().open.then_some(Key("t")) => trap_child)]
pub struct TrapChild;

pub type TrapPath<'a> = &'a mut Trap;
pub type TrapChildPath<'a> = PathMut<TrapChild, TrapPath<'a>>;

fn trap_root<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, TrapPath<'x>>,
) -> (Vec<DemoEffect>, Completed<TrapPath<'x>>) {
    (vec![DemoEffect::FlashOverlay], st.complete())
}

fn trap_child<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, TrapChildPath<'x>>,
) -> (Vec<DemoEffect>, Completed<TrapChildPath<'x>>) {
    (vec![DemoEffect::SawStanding], st.complete())
}

#[test]
fn the_deepest_binding_takes_the_claim_and_the_ancestors_is_skipped() {
    let mut trap = Trap {
        open: true,
        child: TrapChild,
    };
    assert_eq!(
        dispatch::<M, Trap, _>(&mut trap, &key("t")),
        (vec![DemoEffect::SawStanding], true),
        "the child claimed, so the root's bind completed where it stood"
    );

    // With the child's trigger absent, the same key reaches the root's bind instead.
    let mut trap = Trap {
        open: false,
        child: TrapChild,
    };
    assert_eq!(
        dispatch::<M, Trap, _>(&mut trap, &key("t")),
        (vec![DemoEffect::FlashOverlay], true)
    );
}

// ---- three levels: forwarding, and the fold after each item ----

#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[post(AnyKey => witness)] // opt_0: sees what the descent did
#[post(AnyKey => witness)] // opt_1: sees what opt_0 left
pub struct Top {
    #[resolve_into]
    pub mid: Mid,
}

/// The middle node binds nothing, so a leave from the leaf passes straight through its
/// `state.complete()` on the way to `Top`.
#[derive(Bind)]
#[node(parent = TopPath)]
#[binds(M)]
pub struct Mid {
    #[resolve_into]
    pub leaf: Leaf,
}

#[derive(Bind)]
#[node(parent = MidPath)]
#[binds(M)]
#[bind(
    Key("go") => leaf_home,
    // One gesture, composed from its units at the bind site.
    Key("pair") => and!(emits_flash, emits_cancel),
    Key("nest") => and!(emits_flash, emits_cancel, emits_flash),
    Key("leave-then-look") => and!(leaf_home, witness_leaf),
)]
pub struct Leaf;

pub type TopPath<'a> = &'a mut Top;
pub type MidPath<'a> = PathMut<Mid, TopPath<'a>>;
pub type LeafPath<'a> = PathMut<Leaf, MidPath<'a>>;

/// Two effect-only units, distinguishable in the order they ran.
fn emits_flash<P: HasStop + Complete<P>>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<DemoEffect>, Completed<P>) {
    (vec![DemoEffect::FlashOverlay], st.complete())
}

fn emits_cancel<P: HasStop + Complete<P>>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<DemoEffect>, Completed<P>) {
    (vec![DemoEffect::CancelTimer(TimerId(0))], st.complete())
}

/// Reports the state it was handed at the LEAF, which is how a second unit says what the first
/// one did to the path they share.
fn witness_leaf<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, LeafPath<'x>>,
) -> (Vec<DemoEffect>, Completed<LeafPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(leaf) => (vec![DemoEffect::SawStanding], leaf.complete()),
        MaybeInvalidated::Invalidated(c) => (vec![DemoEffect::SawInvalidated], c),
    }
}

/// The leaf's leave, the same shape `go_home` has on the demo tree: it ends at the root, so it
/// does not match the state at all.
fn leaf_home<'x, P>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<DemoEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<TopPath<'x>>,
    TopPath<'x>: Complete<P>,
{
    (vec![], st.state.into_ancestor::<TopPath<'x>>().complete())
}

/// Reports which branch it was handed, and leaves the state exactly as it found it.
fn witness<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, TopPath<'x>>,
) -> (Vec<DemoEffect>, Completed<TopPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(top) => (vec![DemoEffect::SawStanding], top.complete()),
        MaybeInvalidated::Invalidated(c) => (vec![DemoEffect::SawInvalidated], c),
    }
}

#[test]
fn a_leave_forwards_through_a_node_that_binds_nothing() {
    let mut top = Top {
        mid: Mid { leaf: Leaf },
    };
    // The leaf leaves for the root. `Mid` has nothing scheduled, so its state completes
    // untouched, and `Top`'s first post sees the leave. The fold of what that post returned
    // re-establishes the root as standing, which the second post reports.
    assert_eq!(
        dispatch::<M, Top, _>(&mut top, &key("go")),
        (
            vec![DemoEffect::SawInvalidated, DemoEffect::SawStanding],
            true
        )
    );
}

#[test]
fn posts_run_without_a_claim_on_the_standing_branch() {
    let mut top = Top {
        mid: Mid { leaf: Leaf },
    };
    assert_eq!(
        dispatch::<M, Top, _>(&mut top, &key("nothing-binds-this")),
        (
            vec![DemoEffect::SawStanding, DemoEffect::SawStanding],
            false
        )
    );
}

// ---- and: one claim, effects in order, the second unit on the first's state ----

/// The units' effects land in call order, ahead of the posts that run above them, and the whole
/// composition takes the one claim rather than one per unit.
#[test]
fn and_concatenates_effects_in_order() {
    let mut top = Top {
        mid: Mid { leaf: Leaf },
    };
    assert_eq!(
        dispatch::<M, Top, _>(&mut top, &key("pair")),
        (
            vec![
                DemoEffect::FlashOverlay,
                DemoEffect::CancelTimer(TimerId(0)),
                // Top's two posts, which never claimed and ran anyway.
                DemoEffect::SawStanding,
                DemoEffect::SawStanding,
            ],
            true
        )
    );
}

/// `a` leaves; `b` is handed what `a` left rather than the path `a` was handed, so it takes its
/// invalidated arm and forwards the leave. The enclosing dispatch then folds that leave at each
/// node above, which `Top`'s posts report: gone, then standing again at the root.
#[test]
fn the_second_unit_sees_the_firsts_leave() {
    let mut top = Top {
        mid: Mid { leaf: Leaf },
    };
    assert_eq!(
        dispatch::<M, Top, _>(&mut top, &key("leave-then-look")),
        (
            vec![
                DemoEffect::SawInvalidated,
                DemoEffect::SawInvalidated,
                DemoEffect::SawStanding,
            ],
            true
        )
    );
}

/// The flat spelling runs all three, in order, under the one claim, identically to the
/// hand-nested `and(a, and(b, c))` it expands to.
#[test]
fn and_nests() {
    let mut top = Top {
        mid: Mid { leaf: Leaf },
    };
    assert_eq!(
        dispatch::<M, Top, _>(&mut top, &key("nest")),
        (
            vec![
                DemoEffect::FlashOverlay,
                DemoEffect::CancelTimer(TimerId(0)),
                DemoEffect::FlashOverlay,
                DemoEffect::SawStanding,
                DemoEffect::SawStanding,
            ],
            true
        )
    );
}
