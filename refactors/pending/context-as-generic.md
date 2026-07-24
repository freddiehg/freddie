# Context as a generic

Not done. Stub. Companion to `invalidation.md`.

`invalidation.md` currently fixes post context as a concrete bag:

```rust
enum Structure { Valid, Invalidated }
struct Claimed;

struct Context {
    structure: Structure,
    claim: Option<Claimed>,
}
// ctx.claim() -> Option<Claimed>
```

That is odd: two unrelated facts glued under one name because the ascent happens to need both today. The next field (fallback policy, depth of reshape, …) would make the bag worse. This doc is the alternative: **context is a type parameter of the dispatch machine**, not a single struct in `bind`.

## Shape

```rust
// Framework (sketch): the ascent mutates one C; every post receives &mut C.
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
//   fn post(t: T, node: Node<P, ()>, ctx: &mut C) -> (Vec<Effect>, P)
```

`C` is chosen by the app (or by a layer of the stack), not by `bind`. Posts mutate `C` in place (claim, structure, whatever the app puts there). No parallel flags beside `C`.

## What mercury would supply

```rust
// mercury-specific — not in bind
#[derive(Clone, Copy)]
pub enum Structure { Valid, Invalidated }

#[derive(Clone, Copy)]
pub struct Claimed;

#[derive(Clone, Copy)]
pub struct MercuryContext {
    structure: Structure,
    claim: Option<Claimed>,
}

impl MercuryContext {
    pub fn structure(&self) -> Structure { self.structure }
    pub fn claim(&mut self) -> Option<Claimed> { self.claim }
    pub fn set_structure(&mut self, s: Structure) { self.structure = s; }
    pub fn set_claim(&mut self, c: Claimed) { self.claim = Some(c); }
}
```

`exclusive` and rearm's `only_if_valid` bound on `MercuryContext` (or on traits it implements), not on a universal `bind::Context`.

## Why generic

- `structure` and `claim` are independent; cohabiting one named type is accidental.
- Fallbacks, logging, or other ascent facts may need different carriers in different apps.
- `bind` stays ignorant of mercury policy; freddie crates do not smuggle app semantics into the path machinery.
- Default `C = ()` keeps a tree that needs no ascent facts trivial.

## Open

- Whether `structure` (field survival) is still computed by laserbeam/`into_parent` and written into `C` via a trait (`C::set_structure`), so the framework owns invalidation timing but not claim policy.
- How exclusive claim becomes a method on `C` when claim is app-defined (`C: Claimable`).
- Interaction with the prefactor (threaded batch only, `C = ()`).

No implementation plan here. When `invalidation.md` is implemented, either keep the concrete bag as a first cut or land `C` first if the bag already feels wrong at the type boundary.
