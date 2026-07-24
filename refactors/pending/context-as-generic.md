# Context as a generic

Not done. Stub. Companion to `invalidation.md`.

`invalidation.md` currently fixes post context as a concrete bag:

```rust
// invalidation_depth: u32  — step_up each into_parent; invalidate(d) raises it
// claim: Option<Claimed>   — claim() try-take
// validity() derived from depth
```

That is odd: two unrelated facts glued under one name because the ascent happens to need both today. The next field (fallback policy, …) would make the bag worse. This doc is the alternative: **context is a type parameter of the dispatch machine**, not a single struct in `bind`.

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

`C` is chosen by the app (or by a layer of the stack), not by `bind`. Posts mutate `C` in place. No parallel flags beside `C`.

## What mercury would supply

```rust
// mercury-specific — not in bind
pub struct Claimed;

pub struct MercuryContext {
    invalidation_depth: u32,
    claim: Option<Claimed>,
}

impl MercuryContext {
    pub fn validity(&self) -> Validity { /* depth == 0 → Valid else Invalidated */ }
    pub fn invalidate(&mut self, depth: u32) { /* max with current */ }
    pub fn step_up(&mut self) { /* saturating_sub(1) on into_parent */ }
    pub fn claim(&mut self) -> Option<Claimed> { /* try-take */ }
}
```

`exclusive` and rearm's `only_if_valid` bound on `MercuryContext` (or on traits it implements), not on a universal `bind::Context`.

## Why generic

- invalidation depth and claim are independent; cohabiting one named type is accidental.
- Fallbacks, logging, or other ascent facts may need different carriers in different apps.
- `bind` stays ignorant of mercury policy; freddie crates do not smuggle app semantics into the path machinery.
- Default `C = ()` keeps a tree that needs no ascent facts trivial.

## Open

- Whether `invalidate` / `step_up` are framework-owned methods injected via a trait on `C`, so the derive never hand-rolls depth math.
- How exclusive try-take (`claim(&mut self) -> Option<Claimed>`) becomes a method on app-defined `C`.
- Interaction with the prefactor (threaded batch only, `C = ()`).

No implementation plan here. When `invalidation.md` is implemented, either keep the concrete bag as a first cut or land `C` first if the bag already feels wrong at the type boundary.
