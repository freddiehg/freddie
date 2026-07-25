# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

```rust
after_first_peel(path).into_parent().complete()
// → Doll nest (Here / Up) with a fixed origin type
```

Peel is `laserbeam::PathMut::into_parent`. Public arms **Here** / **Up**.

This file’s types are the ones that **compile** (proved with a standalone rustc harness). No extra traits, no parallel spine type.

## Data structures that work

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}

/// complete → Here(focus). `Rest` is only in the type (unused `U` of that Here).
pub struct NoWrap<Rest>(core::marker::PhantomData<Rest>);

/// One skipped layer. `Skipped` is phantom (the PathMut type peeled past).
/// complete → Up(inner’s finish of the same focus).
pub struct WrapUp<Skipped, Inner> {
    inner: Inner,
    _skipped: core::marker::PhantomData<Skipped>,
}

/// complete → Up(focus) with no inner Here (bare terminal, e.g. &mut Root).
pub struct Terminal;

/// Focus is always a real path (`PathMut` or `&mut T`). Wrap is the Russian doll.
pub struct LeavePath<P, W> {
    focus: P,
    wrap: W,
}
```

Mental model:

```text
start (after_first_peel):  wrap = NoWrap        complete → Here(focus)
into_parent:               peel focus; wrap = WrapUp<SkippedPathMut, old_wrap>
complete:                  run wrap on focus     → nested Up(…Here(focus)) or Up(focus)
```

`NoWrap` is the “option none.” `WrapUp` is the “option some(function)” — the function is the type’s `complete` body, one layer per peel. A bool cannot carry the skipped path type.

## `complete` (full bodies)

```rust
impl<P, Rest> LeavePath<P, NoWrap<Rest>> {
    pub fn complete(self) -> Doll<P, Rest> {
        Doll::Here(self.focus)
    }
}

impl<P, Sk, Rest> LeavePath<P, WrapUp<Sk, NoWrap<Rest>>> {
    pub fn complete(self) -> Doll<Sk, Doll<P, Rest>> {
        Doll::Up(Doll::Here(self.focus))
    }
}

impl<P, Sk1, Sk2, Rest> LeavePath<P, WrapUp<Sk1, WrapUp<Sk2, NoWrap<Rest>>>> {
    pub fn complete(self) -> Doll<Sk1, Doll<Sk2, Doll<P, Rest>>> {
        Doll::Up(Doll::Up(Doll::Here(self.focus)))
    }
}

impl<P, Sk> LeavePath<P, WrapUp<Sk, Terminal>> {
    pub fn complete(self) -> Doll<Sk, P> {
        Doll::Up(self.focus)
    }
}
```

Deeper peels: one more `complete` impl per wrap depth under test (same pattern). No trait.

## `into_parent` (full bodies)

Focus must be `PathMut`. Peel with laserbeam. Grow wrap by one `WrapUp`.

```rust
use laserbeam::PathMut;

impl<N, P, R> LeavePath<PathMut<N, P>, NoWrap<Doll<P, R>>> {
    pub fn into_parent(self) -> LeavePath<P, WrapUp<PathMut<N, P>, NoWrap<R>>> {
        LeavePath {
            focus: self.focus.into_parent(),
            wrap: WrapUp {
                inner: NoWrap(core::marker::PhantomData),
                _skipped: core::marker::PhantomData,
            },
        }
    }
}

impl<N, P> LeavePath<PathMut<N, P>, NoWrap<P>> {
    /// Bare rest: Parent is the terminal path type (e.g. &mut Root).
    pub fn into_parent(self) -> LeavePath<P, WrapUp<PathMut<N, P>, Terminal>> {
        LeavePath {
            focus: self.focus.into_parent(),
            wrap: WrapUp {
                inner: Terminal,
                _skipped: core::marker::PhantomData,
            },
        }
    }
}

impl<N, P, Sk, R> LeavePath<PathMut<N, P>, WrapUp<Sk, NoWrap<Doll<P, R>>>> {
    pub fn into_parent(self) -> LeavePath<P, WrapUp<Sk, WrapUp<PathMut<N, P>, NoWrap<R>>>> {
        LeavePath {
            focus: self.focus.into_parent(),
            wrap: WrapUp {
                inner: WrapUp {
                    inner: NoWrap(core::marker::PhantomData),
                    _skipped: core::marker::PhantomData,
                },
                _skipped: core::marker::PhantomData,
            },
        }
    }
}
```

## Start leave

```rust
pub fn after_first_peel<N, P, Rest>(path: PathMut<N, P>) -> LeavePath<P, NoWrap<Rest>> {
    LeavePath {
        focus: path.into_parent(),
        wrap: NoWrap(core::marker::PhantomData),
    }
}
```

`Rest` is fixed by the origin nest (turbofish or expected type on `complete`).

## Proven values (same as rustc harness)

```rust
// AppPath = Root token (stand-in for &mut App)
// LayerPath = PathMut<Layer, AppPath>
// NavPath   = PathMut<Nav, LayerPath>
// NavOut    = Doll<LayerPath, Doll<AppPath, AppPath>>

after_first_peel::<Nav, LayerPath, Doll<AppPath, AppPath>>(nav).complete()
// Here(layer)

after_first_peel::<Nav, LayerPath, Doll<AppPath, AppPath>>(nav)
    .into_parent()
    .complete()
// Up(Here(app))

// bare Up(root):
LeavePath { focus: layer_path, wrap: NoWrap::<AppPath>::… }
    .into_parent()
    .complete()
// Up(app)
```

Three peels:

```text
after_first_peel(deep).into_parent().into_parent().complete()
// Up(Up(Here(root)))
```

## Origin nest type

Concrete alias per known path (derive later). Example:

```rust
type NavAscent<'a> = Doll<LayerPath<'a>, Doll<&'a mut App, &'a mut App>>;
```

No `AscentOut` trait required for the machine to work.

## Existing peel only

```rust
// laserbeam — already exists
impl<Node, Parent> PathMut<Node, Parent> {
    pub fn into_parent(self) -> Parent { self.parent }
}
```

## Tests (when implemented)

Assert the harness cases above on real `PathMut::from_fn` trees.

## Rules

1. `LeavePath { focus, wrap }`. Focus is `PathMut` / `&mut T` only.
2. `NoWrap` = none → `complete` is `Here(focus)`.
3. Each `into_parent` peels once and adds one `WrapUp` (function layer).
4. `complete` monomorphizes the wrap doll; no runtime `Option<fn>`.
5. Public `Doll::Here` / `Doll::Up` only.
6. Only depths with written `complete` / `into_parent` impls exist — add an impl when a test needs another peel.

## Relation to invalidation

Leave/kill uses this. Parent matches `Here` / `Up`. Claim/posts separate.
