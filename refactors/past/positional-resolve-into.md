# `#[resolve_into]` on a tuple-struct field

Not done.

## What it enables

A wrapper node reads best as a tuple: `AndReturnHome(#[resolve_into] NavLayer)`, one field that is the wrapped inner. Today the derive only finds `#[resolve_into]` on a NAMED field, so a tuple struct that carries the attribute is silently treated as a leaf: the descent into the inner is dropped, with no error. This change makes the derive project through a positional field exactly as it does a named one.

It is additive. Nothing in the tree is a tuple struct with `#[resolve_into]` today, so enabling it changes no existing dispatch. It is the prefactor the wrapper work builds on.

## The gap

`find_resolve_into` scans `Fields::Named` only:

```rust
pub fn find_resolve_into(fields: &Fields) -> syn::Result<Option<ResolveInto>> {
    let mut found: Option<ResolveInto> = None;
    if let Fields::Named(named) = fields {
        for f in &named.named {
            ...
            let Some(ident) = f.ident.clone() else { continue };
            found = Some((ident, f.ty.clone(), parent_route(&f.attrs)?));
        }
    }
    Ok(found)
}
```

The result carries the field's `Ident`:

```rust
pub type ResolveInto = (Ident, Type, Option<Path>);
```

and the projection reaches the child through it, `o.#field` / `np.get_mut().#field`. A tuple field has no `Ident`; it is reached by position, `o.0`. The fix is to carry a `syn::Member` instead, which is `Named(Ident)` or `Unnamed(Index)` and renders as `field` or `0` in either projection with no change to the projection code.

## The change: `derive_support`

All of it is in `crates/derive_support/src/lib.rs`.

The imports gain `Member` and `Index`:

```rust
use syn::{Fields, Ident, Index, Member, Path, Type};
```

`ResolveInto` carries a `Member`, before:

```rust
pub type ResolveInto = (Ident, Type, Option<Path>);
```

after:

```rust
pub type ResolveInto = (Member, Type, Option<Path>);
```

`find_resolve_into` scans every field, not only named ones, and builds the `Member` from the field's `ident` or its position. Before:

```rust
pub fn find_resolve_into(fields: &Fields) -> syn::Result<Option<ResolveInto>> {
    let mut found: Option<ResolveInto> = None;
    if let Fields::Named(named) = fields {
        for f in &named.named {
            if !f.attrs.iter().any(|a| a.path().is_ident("resolve_into")) {
                continue;
            }
            if found.is_some() {
                return Err(syn::Error::new(
                    f.span(),
                    "at most one `#[resolve_into]` field per struct",
                ));
            }
            let Some(ident) = f.ident.clone() else {
                continue;
            };
            found = Some((ident, f.ty.clone(), parent_route(&f.attrs)?));
        }
    }
    Ok(found)
}
```

after (`Fields::iter` yields every field for named and unnamed alike, and nothing for a unit struct, so the one loop covers all three shapes):

```rust
pub fn find_resolve_into(fields: &Fields) -> syn::Result<Option<ResolveInto>> {
    let mut found: Option<ResolveInto> = None;
    for (i, f) in fields.iter().enumerate() {
        if !f.attrs.iter().any(|a| a.path().is_ident("resolve_into")) {
            continue;
        }
        if found.is_some() {
            return Err(syn::Error::new(
                f.span(),
                "at most one `#[resolve_into]` field per struct",
            ));
        }
        let member = f.ident.clone().map_or_else(
            || Member::Unnamed(Index::from(i)),
            Member::Named,
        );
        found = Some((member, f.ty.clone(), parent_route(&f.attrs)?));
    }
    Ok(found)
}
```

`Via::Field` carries a `Member`, before:

```rust
pub enum Via<'a> {
    /// A struct `#[resolve_into]` field.
    Field(&'a Ident),
    /// A single-field enum variant `Parent::Variant(Child)`.
    Variant(&'a Ident),
}
```

after:

```rust
pub enum Via<'a> {
    /// A struct `#[resolve_into]` field, named (`.field`) or positional (`.0`).
    Field(&'a Member),
    /// A single-field enum variant `Parent::Variant(Child)`.
    Variant(&'a Ident),
}
```

`single_parent_projection` and `multi_parent_projection` are unchanged: they already interpolate `#field` into `o.#field`, `np.get_mut().#field`, and `pp.#field`, and `Member`'s `ToTokens` renders `field` or `0` there. `Via::Variant` still carries an `Ident`, because a variant name is always a real identifier.

## `bind_macro` is untouched

`crates/bind_macro/src/lib.rs` destructures the `find_resolve_into` result and passes the field straight into `Via::Field`:

```rust
Data::Struct(s) => match find_resolve_into(&s.fields)? {
    None => ...,
    Some((field, child_ty, route)) => {
        let (child, boxed) = unbox(&child_ty);
        let edge = Edge { parent: name, is_root: root, route: route.as_ref(), boxed, via: Via::Field(&field) };
        ...
    }
}
```

`field` is now a `Member` rather than an `Ident`, and `Via::Field(&field)` takes it as-is. There is no other use of it, so both the dispatch and accumulate arms compile unchanged.

## The test: `bind`

The change touches both generated halves: `dispatch_body` and `accumulate_body` each call `find_resolve_into` and build the `Edge` projection. So the fixture is exercised by a dispatch test and an accumulate test, and both live in one new file rather than in the shared `common/mod.rs`: the tuple tree is specific to this prefactor, not part of the full tree the two suites share, and both `bind::dispatch` and `bind::accumulate` are callable from a single test binary.

The fixture mirrors the named tree's three positions: positional `#[resolve_into]` at the root (`.0`), at a non-root node (`.0` through `get_mut`), and through a `Box`. It reuses `common`'s marker, handler, and helpers.

New file `crates/bind/tests/tuple.rs`:

```rust
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
    assert_eq!(bind::dispatch::<Demo, TupleRoot>(&mut root, &key("g")), Some(vec![1]));
    assert_eq!(bind::dispatch::<Demo, TupleRoot>(&mut root, &key("esc")), Some(vec![3]));
    assert_eq!(bind::dispatch::<Demo, TupleRoot>(&mut root, &key("zzz")), None);
}

// The check projects through the same `Edge`, so it collects the root's and leaf's triggers across
// the positional descent.
#[test]
fn positional_resolve_into_accumulates() {
    let mut root = TupleRoot(TupleMid(Box::new(TupleLeaf)));
    let set = bind::accumulate::<Demo, TupleRoot>(&mut root).unwrap();
    assert_eq!(set, HashSet::from([kb("esc"), kb("g")]));
}
```

`common`'s `ignore`, `Keyboard`, `Demo`, `key`, and `kb` are already `pub`; `bind::accumulate` is behind the `check` feature, which the test target already enables (the existing `accumulate.rs` calls it). `TupleLeaf` has no children, so no `TupleLeafPath` alias is defined.

## Status

Small and self-contained: one type, one loop, one enum variant's field type in `derive_support`, and a mirrored test tree. Blocked on nothing.
