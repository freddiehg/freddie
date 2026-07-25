# Derived levels: ascend at the place

Not done. Depends on `invalidation.md` changes 0–4 (landed). This doc holds the model and the new bind items (change 1 below); the codegen flip, the handler migration, and the tests are invalidation.md's change 5 and are written out there, beside the rest of that change's before/afters. Not yet compile-checked.

## Model

A derived level keeps its two sides distinct:

- Descent, unchanged: `f(&parent)` produces `Data`, the derive builds `Node { parent, data }`, and triggers and pres read `&Node` — own data, ancestor data, and the tree through the path, all shared.
- Ascent: the place path at the bottom of the `Node` chain. A derived level's scheduled items receive `AscendState<Place>` and return `Completed<Place>`; the `Node` is consumed by the descent and never comes back. Data a handler needs on the way up is snapped by its pre.

Place per level: mercury's `AppData` / `ChromeApp` / `GhosttyApp` under `Node<AppLayerPath, _>` ascend at `AppLayerPath` (site mirrors it); the test tree's `AppData` under `Node<ShellPath, AppData>` ascends at `ShellPath`, and the nested `TabData` under `Node<AppNode, TabData>` ascends at `ShellPath` too.

The state a derived item receives means what it means everywhere: has the place beneath me been destroyed. The derived data is not state — it is rebuilt from the tree on every dispatch — so it rides in the snap, never in `MaybeInvalidated`. Whether the level existed at all is likewise a pre's question (call `app_data(&path)`, or read the root fields it reads), not the fold's.

Because `Place` is a real place path, one handler shape serves places and derived levels: a generic unit handler bound as `P: HasAncestor<MercuryPath<'a>> + HasStop + Complete<P>` binds at a derived leaf exactly as at a layer, which is what `mercury-post-patterns.md`'s Chrome sketches assume. `Node<P, D>` never appears in an ascent signature.

A `#[resolve_into]` field on a derived level is rejected at derive time (previously it was accepted and silently ignored): everything below the last real place is derived, its `data` dies with the dispatch, and folding a place child hung there would need `Node: Above`, which this design refuses. `refactors/past/derived-child-persistence.md` records the stance; the rejection itself is a change-5 macro delta in invalidation.md.

## bind additions (change 1)

The projection from a parent chain to its place path. A path is its own place; a derived node's is its parent's:

```rust
pub trait HasPlace {
    type Place;
    fn into_place(self) -> Self::Place;
}

impl<'a, R> HasPlace for &'a mut R {
    type Place = &'a mut R;
    fn into_place(self) -> Self {
        self
    }
}

impl<N, P> HasPlace for ::laserbeam::PathMut<N, P> {
    type Place = ::laserbeam::PathMut<N, P>;
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
```

The derived-level dispatch contract. It replaces `DispatchIntoParent`, which change 5 deletes (until then the two coexist; nothing consumes this one yet):

```rust
/// Consumes a derived node, dispatches at that level, and surfaces at the
/// place path beneath it.
pub trait DispatchIntoPlace<M: Bindings>: HasPlace + Sized
where
    Self::Place: ::laserbeam::HasStop,
{
    fn dispatch_into_place(
        self,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'_>,
    ) -> ::laserbeam::Completed<Self::Place>;
}
```

`HasParent` stays: pres walk `node.parent`, and the route fold uses it. The check halves (`DerivedHandler`, `EventHandler`) are untouched: feature-gated, ignored by this design, and they read only triggers, which still take `&Node`.

## Tests (change 1)

A `#[cfg(test)]` module in `bind/src/lib.rs`, one test per `HasPlace` shape. Each asserts through the returned place that it still addresses the tree, so the identity impls and the recursion are all exercised as values, not just as types. `DispatchIntoPlace` gets no change-1 test: nothing implements it until invalidation change 5, whose tests (the `derived.rs` suite, the derived-leave walk, the `Modes` tree, the trybuild rejection) are listed in invalidation.md's Tests section.

```rust
#[cfg(test)]
mod has_place_tests {
    use super::{HasPlace, Node};
    use laserbeam::PathMut;

    struct Root {
        layer: u32,
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
        let path: PathMut<u32, &mut Root> =
            PathMut::from_fn(&mut root, |r| &mut r.layer, |r| &r.layer);
        let mut place: PathMut<u32, &mut Root> = HasPlace::into_place(path);
        *place.get_mut() = 9;
        assert_eq!(root.layer, 9);
    }

    #[test]
    fn a_node_flattens_to_its_parent_path() {
        let mut root = Root { layer: 7 };
        let path: PathMut<u32, &mut Root> =
            PathMut::from_fn(&mut root, |r| &mut r.layer, |r| &r.layer);
        let node = Node {
            parent: path,
            data: "derived",
        };
        let mut place: PathMut<u32, &mut Root> = HasPlace::into_place(node);
        *place.get_mut() = 10;
        assert_eq!(root.layer, 10);
    }

    #[test]
    fn two_node_layers_flatten_to_the_same_place() {
        let mut root = Root { layer: 7 };
        let path: PathMut<u32, &mut Root> =
            PathMut::from_fn(&mut root, |r| &mut r.layer, |r| &r.layer);
        let node = Node {
            parent: Node {
                parent: path,
                data: "outer",
            },
            data: 3_u8,
        };
        let mut place: PathMut<u32, &mut Root> = HasPlace::into_place(node);
        *place.get_mut() = 11;
        assert_eq!(root.layer, 11);
    }
}
```

## Ordered changes

### 1 — bind: `HasPlace` + `DispatchIntoPlace`, pure additions

With the tests above. Nothing else consumes the new items until invalidation change 5.

### 2 — everything else is invalidation change 5

The derived codegen (before/afters in invalidation.md's change-5 section), the `DispatchIntoParent` deletion, the `#[resolve_into]` rejection, the handler migration in mercury and the bind tests, and the derived walks land there, in the one workspace-global change.

## Rules

1. Descent reads the `Node`; ascent holds the place. Data rides in snaps.
2. `DispatchIntoPlace` exists only for `Node`s; no place emits an into-parent dispatch impl.
3. One handler signature everywhere; `Node<P, D>` never appears in an ascent signature; bounds go on the place path.
4. A derived edge folds exactly like a place edge: `Completed<Place>` through `to_maybe_invalidated`.
5. Whether a derived level exists is a pre's question, never the fold's.
6. A derived level has no `#[resolve_into]` child; persistent state lives in the tree at a real place the derived level reads.
