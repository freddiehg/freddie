# `Resolved` is dead weight

Not done. Independent of `resolution.md`, and made worse by it.

## What it is

```rust
pub trait Resolve {
    type Path<'a>;                                          // this node's path type
    type Resolved<'a>;                                      // the enum of every leaf
    fn resolve<'a>(path: Self::Path<'a>) -> Self::Resolved<'a>;
}
```

Two independent things. `Path` is a type alias. `Resolved` plus `resolve()` is a walk: descend from here to the ACTIVE LEAF and hand back that leaf's path.

## Nobody uses the second one

`Dispatch` uses `<Self as Resolve>::Path<'a>` in every signature. Load-bearing.

`Resolved` and `resolve()` appear zero times in `crates/bind` and zero times in `crates/bind_macro`. The only callers of `resolve()` in the repo are `laserbeam_macro`, which generates it, and laserbeam's own tests.

`crates/mercury` declares `enum Resolved<'a>` with a variant per leaf and passes `resolved = Resolved` on all ten nodes, and never constructs or matches a single variant.

## Why the binding layer cannot want it

`resolve()` answers "where is the leaf". `Dispatch` never asks that. It descends one level at a time and runs each level's binds on the way back up, because a bind can live at any level, not only the leaf. The terminus is the one thing it does not need.

## What it costs

Every node's `#[laserbeam(..)]` carries `resolved = Resolved`.

The user hand-writes `enum Resolved<'a>` with a variant per leaf path, and adds a variant whenever a leaf is added.

`laserbeam_macro` emits a `resolve()` match cascade per node that nothing calls.

## Why `resolution.md` makes it worse

`Resolved` is an enum of `Path`s. A derived level has no `Path`; it has a `Node`.

So either derived levels are absent from `Resolved`, which makes `resolve()` silently wrong about where the leaf is, or `Resolved` becomes a mixed `Path`/`Node` enum maintained for a method with no callers.

## The change

Split the trait. Keep `Path`, drop `Resolved` and `resolve()`.

```rust
pub trait Resolve {
    type Path<'a> where Self: 'a;
}
```

`laserbeam_macro` stops emitting `resolve()`. Mercury deletes `enum Resolved` and ten `resolved = Resolved`.

## What to check first

Whether laserbeam's tests are the only consumers, or whether `resolve()` is a public API worth keeping for a user who is not `bind`. If it is worth keeping, it belongs behind its own trait, so a node that never resolves does not have to name a `Resolved` it never returns.
