//! What a scheduled handler is handed: the state the items before it left, and
//! the one claim they all share.
//!
//! The generated code that builds these is change 4's; these are the pieces on
//! their own, over the shared `App -> Layer -> Nav` tree.

mod common;

use bind::{AscendState, Claim, exclusive};
use common::{App, KeyEvent, Layer, LayerPath, Nav, NavPath};
use laserbeam::{Complete, Completed, MaybeInvalidated, PathMut, Stop};

const fn tree(nav_hits: u32) -> App {
    App {
        hits: 0,
        layer: Layer::Nav(Nav { hits: nav_hits }),
    }
}

fn layer_path(app: &mut App) -> LayerPath<'_> {
    PathMut::from_fn(app, |a| &mut a.layer, |a| &a.layer)
}

fn nav_path(app: &mut App) -> NavPath<'_> {
    PathMut::from_fn(
        layer_path(app),
        |lp| match lp.get_mut() {
            Layer::Nav(n) => n,
            Layer::Typing(_) => unreachable!("the test tree is Nav"),
        },
        |lp| match lp.get() {
            Layer::Nav(n) => n,
            Layer::Typing(_) => unreachable!("the test tree is Nav"),
        },
    )
}

const KEY: KeyEvent = KeyEvent { key: "g" };

/// Counts the key and stays where it is.
fn count<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, NavPath<'x>>,
) -> (Vec<usize>, Completed<NavPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut nav) => {
            nav.get_mut().hits += 1;
            (vec![ev.key.len()], nav.complete())
        }
        MaybeInvalidated::Invalidated(completed) => (vec![ev.key.len()], completed),
    }
}

#[test]
fn an_exclusive_handler_runs_when_the_claim_is_free() {
    let mut app = tree(0);
    let mut slot = None;
    let mut claim = Claim::new(&mut slot);
    {
        let state = MaybeInvalidated::NotInvalidated(nav_path(&mut app));
        let (effs, completed) =
            exclusive(count)(&KEY, (), AscendState::new(state, claim.reborrow()));
        assert_eq!(effs, vec![1]);
        let Stop::Here(nav) = completed.into_inner() else {
            panic!("the handler stayed where it was");
        };
        assert_eq!(nav.get().hits, 1);
    }
    assert!(claim.is_taken());
}

/// The trap door: one exclusive per dispatch, so a second sees the claim gone and
/// completes the state it was handed without running its handler.
#[test]
fn an_exclusive_handler_is_skipped_once_the_claim_is_taken() {
    let mut app = tree(0);
    let mut slot = None;
    let mut claim = Claim::new(&mut slot);
    {
        let first = MaybeInvalidated::NotInvalidated(nav_path(&mut app));
        let (_, completed) = exclusive(count)(&KEY, (), AscendState::new(first, claim.reborrow()));
        drop(completed);
    }
    {
        let second = MaybeInvalidated::NotInvalidated(nav_path(&mut app));
        let (effs, completed) =
            exclusive(count)(&KEY, (), AscendState::new(second, claim.reborrow()));
        assert!(effs.is_empty(), "the second handler never ran");
        let Stop::Here(nav) = completed.into_inner() else {
            panic!("a skipped handler completes where the state stands");
        };
        assert_eq!(nav.get().hits, 1, "only the first handler counted");
    }
}

/// A skipped handler forwards an invalidated state rather than resurrecting a
/// path: what it completes to is the leave it was handed.
#[test]
fn a_skipped_handler_forwards_the_leave_it_was_handed() {
    let mut app = tree(0);
    let mut slot = Some(());
    let mut claim = Claim::new(&mut slot);
    {
        let left: Completed<NavPath<'_>> =
            nav_path(&mut app).into_parent().into_parent().complete();
        let state = MaybeInvalidated::Invalidated(left);
        let (effs, completed) =
            exclusive(count)(&KEY, (), AscendState::new(state, claim.reborrow()));
        assert!(effs.is_empty());
        let Stop::Up(rest) = completed.into_inner() else {
            panic!("the leave still points above nav");
        };
        let Stop::Up(root) = rest.into_inner() else {
            panic!("the leave still points above the layer");
        };
        root.hits = 9;
    }
    assert_eq!(app.hits, 9);
}

/// A handler that is scheduled without the gate runs whether or not the claim is
/// gone, which is what a post does.
#[test]
fn a_handler_scheduled_without_the_gate_runs_with_the_claim_taken() {
    let mut app = tree(0);
    let mut slot = Some(());
    let mut claim = Claim::new(&mut slot);
    {
        let state = MaybeInvalidated::NotInvalidated(nav_path(&mut app));
        let (effs, completed) = count(&KEY, (), AscendState::new(state, claim.reborrow()));
        assert_eq!(effs, vec![1]);
        drop(completed);
    }
    let Layer::Nav(nav) = &app.layer else {
        unreachable!("the test tree is Nav")
    };
    assert_eq!(nav.hits, 1);
}
