//! A handler position accepts any EXPRESSION, not just a fn path. `trigger => make(n)`, where
//! `make` returns a handler closure, works because the derive splices the handler as `#handler(ev,
//! node)` — so `make(n)` is called, then its result is called with `(ev, node)`. This is what an
//! `exclusive(h)`-style wrapper relies on, so it is pinned here.

mod common;

use bind::{Bind, Node};
use common::{Demo, KeyEvent, Keyboard, key};

/// A higher-order handler: `plus(n)` returns a handler reporting the fired key's length plus `n`.
fn plus(n: usize) -> impl Fn(&KeyEvent, Node<&mut ExprRoot, ()>) -> [usize; 1] {
    move |ev, _node| [ev.key.len() + n]
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
        bind::dispatch::<Demo, ExprRoot>(&mut root, &key("x")),
        Some(vec![11])
    );
    assert_eq!(bind::dispatch::<Demo, ExprRoot>(&mut root, &key("y")), None);
}
