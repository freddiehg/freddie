//! `#[resolve_into]` on a positional field descends the same as a named one.

mod common;

use std::collections::HashSet;

use bind::Bind;
use common::{Demo, Keyboard, ignore, kb, key};
use laserbeam::PathMut;

#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
#[bind(Keyboard("esc") => ignore)]
struct TupleRoot(#[resolve_into] TupleMid);

#[derive(Bind)]
#[node(parent = TupleRootPath)]
#[binds(Demo)]
struct TupleMid(#[resolve_into] Box<TupleLeaf>);

#[derive(Bind)]
#[node(parent = TupleMidPath)]
#[binds(Demo)]
#[bind(Keyboard("g") => ignore)]
struct TupleLeaf;

type TupleRootPath<'a> = &'a mut TupleRoot;
type TupleMidPath<'a> = PathMut<TupleMid, TupleRootPath<'a>>;

// The leaf binding is reached through `root.0 -> mid.0 (Box) -> leaf`, and the root fallback fires
// when the subtree misses. `ignore` returns the fired key's length.
#[test]
fn positional_resolve_into_descends() {
    let mut root = TupleRoot(TupleMid(Box::new(TupleLeaf)));
    assert_eq!(
        bind::dispatch::<Demo, TupleRoot>(&mut root, &key("g")),
        Some(vec![1])
    );
    assert_eq!(
        bind::dispatch::<Demo, TupleRoot>(&mut root, &key("esc")),
        Some(vec![3])
    );
    assert_eq!(
        bind::dispatch::<Demo, TupleRoot>(&mut root, &key("zzz")),
        None
    );
}

// The check projects through the same `Edge`, so it collects the root's and leaf's triggers across
// the positional descent.
#[test]
fn positional_resolve_into_accumulates() {
    let mut root = TupleRoot(TupleMid(Box::new(TupleLeaf)));
    let set = bind::accumulate::<Demo, TupleRoot>(&mut root).unwrap();
    assert_eq!(set, HashSet::from([kb("esc"), kb("g")]));
}
