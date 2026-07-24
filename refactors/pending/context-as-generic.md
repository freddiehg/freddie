# Context as a generic

Not done. Stub. Companion to `invalidation.md`.

`invalidation.md` currently fixes post context as a concrete bag:

```rust
struct Context {
    structure: Structure, // Valid | Invalidated
    claimed: bool,
}
```

That is odd: two unrelated facts glued under one name because the ascent happens to need both today. The next field (fallback policy, depth of reshape, …) would make the bag worse. This doc is the alternative: **context is a type parameter of the dispatch machine**, not a single struct in `bind`.

## Shape

```rust
// Framework (sketch): the ascent threads C, and every post receives C.
pub trait Dispatch<M: Bindings, C = ()>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        ctx: &mut C,
    ) -> Self::Path<'a>
    where
        Self: 'a;
}

// Post signature:
//   fn post(t: T, node: Node<P, ()>, ctx: C) -> PostOut<P>
// or by value snapshot if C: Copy
//   fn post(t: T, node: Node<P, ()>, ctx: C) -> PostOut<P>
```

`C` is chosen by the app (or by a layer of the stack), not by `bind`. `bind` only requires whatever bounds posts need to run (`Copy`, or a small trait).

## What mercury would supply

```rust
// mercury-specific — not in bind
#[derive(Clone, Copy)]
pub struct MercuryContext {
    structure: Structure,
    claimed: bool,
}

impl MercuryContext {
    pub fn structure(self) -> Structure { self.structure }
    pub fn claimed(self) -> bool { self.claimed }
    pub fn valid(self) -> bool { matches!(self.structure, Structure::Valid) }
}
```

`exclusive` and rearm's `only_if_valid` bound on `MercuryContext` (or on traits it implements), not on a universal `bind::Context`.

## Why generic

- `structure` and `claimed` are independent; cohabiting one named type is accidental.
- Fallbacks, logging, or other ascent facts may need different carriers in different apps.
- `bind` stays ignorant of mercury policy; freddie crates do not smuggle app semantics into the path machinery.
- Default `C = ()` keeps a tree that needs no ascent facts trivial.

## Open

- Snapshot (`C: Copy`) vs `&C` / `&mut C` for posts that update context mid-ascent.
- Whether `structure` (field survival) is still computed by laserbeam/`into_parent` and *injected into* `C` via a trait (`C::with_structure(Structure)`), so the framework owns invalidation but not claim.
- How `PostOut.claim` becomes a method on `C` or a separate channel when claim is app-defined.
- Interaction with the prefactor (threaded batch only, `C = ()`).

No implementation plan here. When `invalidation.md` is implemented, either keep the concrete bag as a first cut or land `C` first if the bag already feels wrong at the type boundary.
