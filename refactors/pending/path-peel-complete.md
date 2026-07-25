# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

On existing `laserbeam::PathMut`:

```rust
after_first_peel(path).into_parent().complete()
// → nested Doll::Here / Doll::Up of parent path types
```

Public arms: **Here** / **Up**. Peel: `PathMut::into_parent`. Root path: `&mut T`.

Deliverable when implemented: unit tests on real `PathMut` nests.

## Existing machinery

```rust
// laserbeam
PathMut<Node, Parent>::into_parent(self) -> Parent

// bind
HasParent for PathMut  // into_parent
Place::Path<'a>        // root: &mut Self; child: PathMut<Self, Parent::Path<'a>>
```

## Public doll

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}
```

```text
Here(path)  — stop here
Up(rest)    — jumped past; rest is thinner nest
```

```text
// leave started at NavPath = PathMut<Nav, LayerPath>
Doll<LayerPath, Doll<AppPath, AppPath>>
```

Name by leave origin (path and/or node as needed). Do not write the nest in signatures unless a test asserts it.

```rust
// Per-node or per-path alias — written by derive or by hand for a known Place::Path
type NavAscent<'a> = Doll<LayerPath<'a>, Doll<AppPath<'a>, AppPath<'a>>>;
```

No `AscentOut` trait. The nest is a normal type alias (or opaque newtype around that alias). The derive already knows the parent chain when it emits `Place::Path`; it emits the matching alias the same way.

## Why not a trait

A trait (`AscentOut`) would only be a type-level function `Path → Nest`. Rust does that with associated types, but:

- Every real call site already has a **concrete** path type (`NavPath`, `InnerPath`, …).
- The derive already expands parent links for `from_fn`.
- Agents.md: avoid traits that exist only to carry one associated type / fold boilerplate.

So: **spell the nest (or an alias) where the path type is known.** No open type family on all paths until something truly generic needs it.

`Pack` / `PeelPack` as traits are also optional. Below they are **structs + inherent methods** so each pack shape has a real `pack` / `peel` body without a trait object or trait bound soup. If monomorphized free functions read clearer later, use those.

## Pack (structs, inherent methods)

```rust
use core::marker::PhantomData;
use laserbeam::PathMut;

/// Stop here.
pub struct AsHere<E>(PhantomData<E>);

impl<E> AsHere<E> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }

    pub fn pack<P>(self, path: P) -> Doll<P, E> {
        Doll::Here(path)
    }
}

/// This layer skipped.
pub struct AsUp<Q, Inner>(Inner, PhantomData<Q>);

impl<Q, Inner> AsUp<Q, Inner> {
    pub const fn new(inner: Inner) -> Self {
        Self(inner, PhantomData)
    }
}

impl<Q, Inner, P, InnerOut> AsUp<Q, Inner>
where
    // Inner packs P → InnerOut (each pack type has its own pack method)
{
    // concrete impls below per Inner shape
}

impl<Q, P, E> AsUp<Q, AsHere<E>> {
    pub fn pack(self, path: P) -> Doll<Q, Doll<P, E>> {
        Doll::Up(self.0.pack(path))
    }
}

impl<Q, P> AsUp<Q, AsTerminal> {
    pub fn pack(self, path: P) -> Doll<Q, P> {
        Doll::Up(self.0.pack(path))
    }
}

// Nested AsUp: AsUp<Q, AsUp<…>> — add impls as tests need, or one generic
// once the pattern is fixed. Prefer generating/impl blocks over a Pack trait.

pub struct AsTerminal;

impl AsTerminal {
    pub fn pack<P>(self, path: P) -> P {
        path
    }
}
```

### Peel (pack rewrite after `PathMut::into_parent`)

Inherent on each pack that sits on a `PathMut`:

```rust
impl<Node, Parent, E> AsHere<Doll<Parent, E>> {
    /// Focus was PathMut<Node, Parent>; after peel, stop further at Parent uses AsHere<E>.
    pub fn peel(self) -> AsUp<PathMut<Node, Parent>, AsHere<E>> {
        AsUp::new(AsHere::new())
    }
}

impl<Node, Parent> AsHere<Parent> {
    /// Bare parent rest (e.g. Parent = &mut Root).
    pub fn peel(self) -> AsUp<PathMut<Node, Parent>, AsTerminal> {
        AsUp::new(AsTerminal)
    }
}

impl<Node, Parent, Q, E> AsUp<Q, AsHere<Doll<Parent, E>>> {
    pub fn peel(self) -> AsUp<Q, AsUp<PathMut<Node, Parent>, AsHere<E>>> {
        AsUp::new(self.0.peel())
    }
}

impl<Node, Parent, Q> AsUp<Q, AsHere<Parent>> {
    pub fn peel(self) -> AsUp<Q, AsUp<PathMut<Node, Parent>, AsTerminal>> {
        AsUp::new(self.0.peel())
    }
}

// Deeper AsUp nests: same idea — peel the inner pack, wrap AsUp<Q, _>.
```

Need a peel for `AsUp<Q, AsUp<…>>` when tests go three deep; write the impl next to the test that needs it (or a small macro). Still no trait.

## LeavePath

```rust
pub struct LeavePath<P, Pk> {
    focus: P,
    pack: Pk,
}

impl<P, Pk> LeavePath<P, Pk> {
    pub fn focus(&self) -> &P {
        &self.focus
    }

    pub fn focus_mut(&mut self) -> &mut P {
        &mut self.focus
    }
}

// complete: only where pack has pack(P) → Out
impl<P, E> LeavePath<P, AsHere<E>> {
    pub fn complete(self) -> Doll<P, E> {
        let LeavePath { focus, pack } = self;
        pack.pack(focus)
    }
}

impl<Q, E, P> LeavePath<P, AsUp<Q, AsHere<E>>> {
    pub fn complete(self) -> Doll<Q, Doll<P, E>> {
        let LeavePath { focus, pack } = self;
        pack.pack(focus)
    }
}

impl<Q, P> LeavePath<P, AsUp<Q, AsTerminal>> {
    pub fn complete(self) -> Doll<Q, P> {
        let LeavePath { focus, pack } = self;
        pack.pack(focus)
    }
}

// into_parent: focus PathMut, pack peels
impl<Node, Parent, E> LeavePath<PathMut<Node, Parent>, AsHere<Doll<Parent, E>>> {
    pub fn into_parent(self) -> LeavePath<Parent, AsUp<PathMut<Node, Parent>, AsHere<E>>> {
        let LeavePath { focus, pack } = self;
        LeavePath {
            focus: focus.into_parent(),
            pack: pack.peel(),
        }
    }
}

impl<Node, Parent> LeavePath<PathMut<Node, Parent>, AsHere<Parent>> {
    pub fn into_parent(self) -> LeavePath<Parent, AsUp<PathMut<Node, Parent>, AsTerminal>> {
        let LeavePath { focus, pack } = self;
        LeavePath {
            focus: focus.into_parent(),
            pack: pack.peel(),
        }
    }
}

impl<Node, Parent, Q, E> LeavePath<PathMut<Node, Parent>, AsUp<Q, AsHere<Doll<Parent, E>>>> {
    pub fn into_parent(
        self,
    ) -> LeavePath<Parent, AsUp<Q, AsUp<PathMut<Node, Parent>, AsHere<E>>>> {
        let LeavePath { focus, pack } = self;
        LeavePath {
            focus: focus.into_parent(),
            pack: pack.peel(),
        }
    }
}

// …
```

More `into_parent` / `complete` impls as depth requires. Verbose, but each is a real function. A trait would hide the same cases behind bounds that still need those impls.

### First peel

```rust
/// First peel. `Rest` is the thinner doll type for `Parent` (alias or nested Doll).
pub fn after_first_peel<Node, Parent, Rest>(
    path: PathMut<Node, Parent>,
) -> LeavePath<Parent, AsHere<Rest>> {
    LeavePath {
        focus: path.into_parent(),
        pack: AsHere::new(),
    }
}
```

Caller/return type fixes `Rest` (e.g. `after_first_peel::<_, _, Doll<AppPath, AppPath>>(nav)` or turbofish inferred from `let out: NavAscent = …`).

Or a thin helper per known alias:

```rust
fn after_first_peel_nav<'a>(
    path: NavPath<'a>,
) -> LeavePath<LayerPath<'a>, AsHere<Doll<AppPath<'a>, AppPath<'a>>>> {
    after_first_peel(path)
}
```

Derive emits the latter shape for each node when invalidation lands.

## Worked values

```rust
// NavPath = PathMut<Nav, LayerPath>
// LayerPath = PathMut<Layer, AppPath>
// AppPath = &mut App

// one peel
after_first_peel::<Nav, LayerPath, Doll<AppPath, AppPath>>(nav).complete()
// → Doll::Here(layer)

// two peels
after_first_peel::<Nav, LayerPath, Doll<AppPath, AppPath>>(nav)
    .into_parent()
    .complete()
// → Doll::Up(Doll::Here(app))   // if Rest nest uses AsHere on &mut App
// or Doll::Up(app) if Rest = AppPath bare and peel uses AsTerminal
```

## Tests (when implemented)

On real `PathMut` from bind fixtures (`from_fn` Nav/Layer/App).

| Case | Assert |
| --- | --- |
| one peel | `Doll::Here(layer)` |
| two peels | `Doll::Up(…)` at app |
| assign to path-specific alias | type checks |

No `AscentOut` / `Pack` / `PeelPack` traits in the test API.

## Ordered changes

### 1 — `Doll`

### 2 — `AsHere` / `AsUp` / `AsTerminal` inherent `pack` / `peel`

### 3 — `LeavePath` + `after_first_peel` + `complete` / `into_parent` impls for depths under test

### 4 — unit tests on `PathMut` trees

### 5 — invalidation uses the same; derive emits nest aliases / turbofish Rest

## Rules

1. Peel is `PathMut::into_parent`.
2. Public arms are `Doll::Here` / `Doll::Up`.
3. Nest type is a concrete alias at each known path, not a trait associated type (unless a later generic force it).
4. Pack peels are inherent methods on pack structs.
5. Deliverable is tests; no implementation until design is settled.

## Relation to invalidation

Leave/kill: `after_first_peel` → `into_parent`* → `complete`. Parent matches `Doll::Here` / `Doll::Up`. Claim/posts separate.
