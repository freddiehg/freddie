//! Multiple children: several `#[child]` fields and `#[derived_children]` fns live at once on
//! one place node, visited in declared order — fields first, then derived — with each descent
//! going through `MaybeInvalidated::descend`, so a leave that stopped at the branch point still
//! descends the remaining siblings while the branch point stays invalidated, and a leave that
//! went above skips them.
//!
//! Handlers return fixed codes rather than key lengths, one per handler, so a test reads off
//! exactly which ran and in what order.

mod common;

use bind::{AscendState, Bind, dispatch, if_not_invalidated};
use common::{Demo, KeyEvent, Keyboard, key};
use laserbeam::{Completed, CompletesTo, MaybeInvalidated, PathMut};

// Grove -> Fork -> { Left, Right }: a mid-tree branch point with two field children.
#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
pub struct Grove {
    #[child]
    pub fork: Fork,
}

#[derive(Bind)]
#[node(parent_path = GrovePath)]
#[binds(Demo)]
#[bind(
    Keyboard("both") => if_not_invalidated(fork_both),
    Keyboard("post-leave") => if_not_invalidated(fork_after_post),
)]
pub struct Fork {
    #[child]
    pub left: Left,
    #[child]
    pub right: Right,
}

#[derive(Bind)]
#[node(parent_path = ForkPath)]
#[binds(Demo)]
#[bind(
    Keyboard("l") => if_not_invalidated(left_key),
    Keyboard("both") => if_not_invalidated(left_both),
    Keyboard("up") => if_not_invalidated(left_to_fork),
    Keyboard("top") => if_not_invalidated(left_to_grove),
)]
#[post(Keyboard("post-leave") => left_post_leaves)]
pub struct Left;

#[derive(Bind)]
#[node(parent_path = ForkPath)]
#[binds(Demo)]
#[bind(
    Keyboard("r") => if_not_invalidated(right_key),
    Keyboard("both") => if_not_invalidated(right_both),
)]
#[post(
    Keyboard("up") => right_post,
    Keyboard("top") => right_post,
    Keyboard("post-leave") => right_post,
)]
pub struct Right;

pub type GrovePath<'a> = &'a mut Grove;
pub type ForkPath<'a> = PathMut<Fork, GrovePath<'a>>;
pub type LeftPath<'a> = PathMut<Left, ForkPath<'a>>;
pub type RightPath<'a> = PathMut<Right, ForkPath<'a>>;

fn left_key<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: LeftPath<'x>,
) -> (Vec<usize>, Completed<LeftPath<'x>>) {
    (vec![1], p.complete())
}

fn right_key<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: RightPath<'x>,
) -> (Vec<usize>, Completed<RightPath<'x>>) {
    (vec![2], p.complete())
}

fn left_both<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: LeftPath<'x>,
) -> (Vec<usize>, Completed<LeftPath<'x>>) {
    (vec![11], p.complete())
}

fn right_both<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: RightPath<'x>,
) -> (Vec<usize>, Completed<RightPath<'x>>) {
    (vec![21], p.complete())
}

fn fork_both<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: ForkPath<'x>,
) -> (Vec<usize>, Completed<ForkPath<'x>>) {
    (vec![31], p.complete())
}

/// A leave to the branch point from a BIND row: the claim is taken and the leave stops at Fork.
fn left_to_fork<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: LeftPath<'x>,
) -> (Vec<usize>, Completed<LeftPath<'x>>) {
    (vec![12], p.into_parent().complete())
}

/// A leave past the branch point: to the root, so the siblings not yet visited are skipped.
fn left_to_grove<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: LeftPath<'x>,
) -> (Vec<usize>, Completed<LeftPath<'x>>) {
    let root: GrovePath<'x> = p.into_ancestor();
    (vec![13], root.complete())
}

/// A leave to the branch point from a POST: no claim is taken, so the branch point's own row
/// would win the claim — the invalidation is what has to gate it.
fn left_post_leaves<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, LeftPath<'x>>,
) -> (Vec<usize>, Completed<LeftPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(left) => (vec![14], left.into_parent().complete()),
        MaybeInvalidated::Invalidated(c) => (vec![14], c),
    }
}

/// The sibling's schedule: runs whenever Right's subtree is descended, whatever Left did.
fn right_post<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, RightPath<'x>>,
) -> (Vec<usize>, Completed<RightPath<'x>>) {
    (vec![22], st.complete())
}

/// The branch point's own row for the post-leave key: the join must keep it gated.
fn fork_after_post<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: ForkPath<'x>,
) -> (Vec<usize>, Completed<ForkPath<'x>>) {
    (vec![32], p.complete())
}

fn grove() -> Grove {
    Grove {
        fork: Fork {
            left: Left,
            right: Right,
        },
    }
}

#[test]
fn each_branch_owns_its_keys() {
    let mut g = grove();
    assert_eq!(dispatch::<Demo, Grove, _>(&mut g, &key("l")), vec![1]);
    assert_eq!(dispatch::<Demo, Grove, _>(&mut g, &key("r")), vec![2]);
}

#[test]
fn declaration_order_is_claim_order() {
    let mut g = grove();
    assert_eq!(dispatch::<Demo, Grove, _>(&mut g, &key("both")), vec![11]);
}

#[test]
fn a_leave_to_the_branch_point_still_descends_the_sibling() {
    let mut g = grove();
    assert_eq!(dispatch::<Demo, Grove, _>(&mut g, &key("up")), vec![12, 22]);
}

#[test]
fn a_stopped_here_leave_gates_the_branch_points_own_row() {
    let mut g = grove();
    // Left's post leaves to Fork without a claim, Right's post still runs, and Fork's own row
    // wins the free claim but finds its path invalidated, so code 32 must be absent.
    assert_eq!(
        dispatch::<Demo, Grove, _>(&mut g, &key("post-leave")),
        vec![14, 22]
    );
}

#[test]
fn a_leave_above_the_branch_point_skips_the_siblings_not_yet_visited() {
    let mut g = grove();
    // Left leaves to the root, so Right's whole schedule — its post included — never runs.
    assert_eq!(dispatch::<Demo, Grove, _>(&mut g, &key("top")), vec![13]);
}

// Host: one field child and two derived children on one place node.
#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
#[derived_children(first_data, second_data)]
pub struct Host {
    pub first: Option<u32>,
    pub second: Option<u32>,
    #[child]
    pub slot: Slot,
}

#[derive(Bind)]
#[node(parent_path = HostPath)]
#[binds(Demo)]
#[bind(
    Keyboard("m") => if_not_invalidated(slot_key),
    Keyboard("shared") => if_not_invalidated(slot_shared),
)]
pub struct Slot;

pub type HostPath<'a> = &'a mut Host;
pub type SlotPath<'a> = PathMut<Slot, HostPath<'a>>;

#[derive(Bind)]
#[derived_node(parent_path = HostPath)]
#[binds(Demo)]
#[bind(
    Keyboard("one") => first_key,
    Keyboard("shared") => first_shared,
)]
pub struct FirstData {
    pub n: u32,
}

#[derive(Bind)]
#[derived_node(parent_path = HostPath)]
#[binds(Demo)]
#[bind(Keyboard("two") => second_key)]
pub struct SecondData {
    pub n: u32,
}

fn first_data(host: &HostPath) -> Option<FirstData> {
    host.first.map(|n| FirstData { n })
}

fn second_data(host: &HostPath) -> Option<SecondData> {
    host.second.map(|n| SecondData { n })
}

fn slot_key<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: SlotPath<'x>,
) -> (Vec<usize>, Completed<SlotPath<'x>>) {
    (vec![40], p.complete())
}

fn slot_shared<'x>(
    _ev: &KeyEvent,
    _snap: (),
    p: SlotPath<'x>,
) -> (Vec<usize>, Completed<SlotPath<'x>>) {
    (vec![43], p.complete())
}

fn first_key<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, HostPath<'x>>,
) -> (Vec<usize>, Completed<HostPath<'x>>) {
    (vec![41], st.complete())
}

fn first_shared<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, HostPath<'x>>,
) -> (Vec<usize>, Completed<HostPath<'x>>) {
    (vec![44], st.complete())
}

fn second_key<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, HostPath<'x>>,
) -> (Vec<usize>, Completed<HostPath<'x>>) {
    (vec![42], st.complete())
}

#[test]
fn field_and_derived_children_coexist() {
    let mut h = Host {
        first: Some(1),
        second: Some(2),
        slot: Slot,
    };
    assert_eq!(dispatch::<Demo, Host, _>(&mut h, &key("m")), vec![40]);
    assert_eq!(dispatch::<Demo, Host, _>(&mut h, &key("one")), vec![41]);
    assert_eq!(dispatch::<Demo, Host, _>(&mut h, &key("two")), vec![42]);
}

#[test]
fn a_missing_derived_child_is_skipped_and_its_siblings_still_run() {
    let mut h = Host {
        first: None,
        second: Some(2),
        slot: Slot,
    };
    assert_eq!(
        dispatch::<Demo, Host, _>(&mut h, &key("one")),
        Vec::<usize>::new()
    );
    assert_eq!(dispatch::<Demo, Host, _>(&mut h, &key("two")), vec![42]);
}

#[test]
fn a_field_child_outranks_a_derived_child_for_a_shared_trigger() {
    let mut h = Host {
        first: Some(1),
        second: Some(2),
        slot: Slot,
    };
    assert_eq!(dispatch::<Demo, Host, _>(&mut h, &key("shared")), vec![43]);
}

/// The check does not walk a branch point: any tree containing one errors at the call.
#[cfg(feature = "check")]
#[test]
fn accumulate_errors_at_a_branch_point() {
    let mut g = grove();
    assert_eq!(
        bind::accumulate::<Demo, Grove>(&mut g),
        Err(bind::BindError::MultiChildNode)
    );
}
