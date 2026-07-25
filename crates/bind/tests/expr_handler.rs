//! A handler position accepts any EXPRESSION, not just a fn path. `trigger => make(n)`, where
//! `make` returns a handler closure, works because the derive splices the rhs into call position
//! — so `make(n)` is called, then its result is called with the event, the snap, and the state.
//! This is what `exclusive` itself relies on, and what a `#[pre_post]` rhs like
//! `exclusive(handler)` is, so it is pinned here.

mod common;

use bind::{AscendState, Bind};
use common::{Demo, KeyEvent, Keyboard, key};
use laserbeam::Completed;

/// A higher-order handler: `plus(n)` returns a handler reporting the fired key's length plus `n`.
fn plus(
    n: usize,
) -> impl for<'a, 'b> Fn(
    &KeyEvent,
    (),
    AscendState<'a, &'b mut ExprRoot>,
) -> (Vec<usize>, Completed<&'b mut ExprRoot>) {
    move |ev, (), st| (vec![ev.key.len() + n], st.complete())
}

#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
#[bind(Keyboard("x") => plus(10))] // an expression, not a path
struct ExprRoot;

#[test]
fn expression_handler_is_called() {
    let mut root = ExprRoot;
    // "x" has length 1, plus 10.
    assert_eq!(
        bind::dispatch::<Demo, ExprRoot, _>(&mut root, &key("x")),
        (vec![11], true)
    );
    assert_eq!(
        bind::dispatch::<Demo, ExprRoot, _>(&mut root, &key("y")),
        (vec![], false)
    );
}
