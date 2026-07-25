# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

Leave on real `laserbeam::PathMut`. Each leave path holds a **function** (type-state) `Focus → OriginOut`. One peel installs one layer of that function. `complete` runs it.

Public arms: **Here** / **Up** (`Doll`). Same Either as `Result`’s Ok/Err; names match the meaning.

## Either / Result

One layer is an Either:

```text
Here(path)  — stop at this path   (Result::Ok)
Up(rest)    — went further        (Result::Err)
```

You need that choice at every layer where you might stop **or** skip. Nested:

```text
Doll<BPath, Doll<APath, RootPath>>
// = Result<BPath, Result<APath, RootPath>> if you prefer Result
```

**Ok is not `!`.** After peeling C → B, stopping at B is `Here(b)` — inhabited. `!` on Ok would mean “can only Up,” i.e. forced peel; that is not the first stop.

Bare terminal: last Up can hold the root path directly (`Up(root)`) instead of `Up(Here(root))` if the rest type is the path itself, not another Doll.

## Function at every leave path

Leave started at **C**. Origin type (C already peeled once by `after_first_peel`):

```text
Out = Doll<BPath, Doll<APath, R>>
```

Every `LeavePath` at focus `F` holds
`finish: F → Out`
(as type-state: the wrap type **is** that function).

### At B (just after C → B)

```text
finish_B : BPath → Out
finish_B = Here
// value: |b| Doll::Here(b)
```

### One-level lift (skip a path type)

Skipping **B** means: take whatever doll you would have produced **from the parent side of B**, and wrap `Up`:

```text
lift_B : Doll<APath, R> → Doll<BPath, Doll<APath, R>>
lift_B = Up
// value: |rest| Doll::Up(rest)
// Result language: Result<A, R> → Result<B, Result<A, R>> = Err
```

(Your `Result<BPath, T> → Result<CPath, Result<BPath, T>>` is the same shape one level higher — lift past C. After `after_first_peel`, C is already gone from `Out`; lifts start at B.)

### At A (after B → A): compose

Stop at A:

```text
here_A : APath → Doll<APath, R>
here_A = Here
```

Skip B then stop at A:

```text
finish_A = lift_B ∘ here_A
         : APath → Out
// |a| Up(Here(a))
// Result: |a| Err(Ok(a))
```

That is why A is “given `Err(Ok(…))`” / `Up(Here(…))`: not a hand-built nest, **composition of one lift with Here**.

### Further peel A → R (terminal bare R)

```text
here_R : R → R                 // terminal: path is the rest
// or Here if R still wrapped in Doll

lift_A : R → Doll<APath, R>    // |r| Up(r)   bare
// or |r| Up(Here(r))

finish_R = lift_B ∘ lift_A
         : R → Out
// |r| Up(Up(r))   or Up(Up(Here(r)))
// Result: |r| Err(Err(r)) or Err(Err(Ok(r)))
```

Again: **only compose one `Up`/`Err` per peel.**

## Line of thinking (completed)

```text
Out fixed at leave origin (after first peel from C):
  Out = Doll<B, Doll<A, R>>

At B:    finish = Here                     : B → Out

Peel B→A installs lift_B = Up              : Doll_at_A → Doll<B, Doll_at_A>
         finish_A = lift_B ∘ Here          : A → Out
                                           = |a| Up(Here(a))

Peel A→R installs lift_A = Up              : R → Doll<A, R>  (or Up∘Here)
         finish_R = lift_B ∘ lift_A        : R → Out
                                           = |r| Up(Up(…))
```

Each `into_parent`:

1. `focus = focus.into_parent()` (laserbeam).
2. `finish_new = lift_skipped ∘ finish_old_at_new_focus`
   where `finish_old_at_new_focus` is “Here at new focus” when the old finish was Here at the child path — i.e. replace “Here at child” with “Here at parent” then apply lifts for every skipped layer already in the wrap, **or** equivalently store only the composed `finish: Focus → Out` rebuilt as `Up ∘ old_finish_reinterpreted`.

Simplest value representation that matches this:

```text
// wrap type-state IS the composed finish
NoWrap        ≅ Here           : F → Doll<F, Rest>
WrapUp<Sk, W> ≅ Up ∘ W         : F → Doll<Sk, W’s out from F>
```

Same as the compiling harness: `complete` monomorphizes the composition; each `into_parent` adds one `Up` to the composition.

## Data structures (minimal, compile-shaped)

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}

/// finish = Here. Rest only in the type.
pub struct NoWrap<Rest>(core::marker::PhantomData<Rest>);

/// finish = Up ∘ inner.finish. Skipped = path type peeled past (phantom).
pub struct WrapUp<Skipped, Inner> {
    inner: Inner,
    _skipped: core::marker::PhantomData<Skipped>,
}

pub struct Terminal; // bare: finish = Up(focus), no inner Here

pub struct LeavePath<P, W> {
    focus: P, // PathMut or &mut T
    wrap: W,  // NoWrap | WrapUp<…>  ≅ Option-none | composed Up functions
}
```

```rust
// complete ≅ run finish
impl<P, Rest> LeavePath<P, NoWrap<Rest>> {
    pub fn complete(self) -> Doll<P, Rest> {
        Doll::Here(self.focus) // Here
    }
}

impl<P, Sk, Rest> LeavePath<P, WrapUp<Sk, NoWrap<Rest>>> {
    pub fn complete(self) -> Doll<Sk, Doll<P, Rest>> {
        Doll::Up(Doll::Here(self.focus)) // lift ∘ Here
    }
}

impl<P, Sk> LeavePath<P, WrapUp<Sk, Terminal>> {
    pub fn complete(self) -> Doll<Sk, P> {
        Doll::Up(self.focus) // lift bare
    }
}

// into_parent ≅ peel + compose one lift
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

// deeper: compose another WrapUp inside (same pattern as harness)
```

```rust
pub fn after_first_peel<N, P, Rest>(path: PathMut<N, P>) -> LeavePath<P, NoWrap<Rest>> {
    LeavePath {
        focus: path.into_parent(),
        wrap: NoWrap(core::marker::PhantomData),
    }
}
```

## Result vs Doll

| | |
| --- | --- |
| Need Either? | Yes — stop vs went further |
| `Result`? | Fine as private layout (`Ok`/`Err` = Here/Up) |
| Public names | `Here` / `Up` |
| Ok = `!`? | No at layers you can stop; first stop after peel is `Here(path)` |

## Existing peel

```rust
PathMut::into_parent(self) -> Parent  // laserbeam only
```

## Tests (when implemented)

| Call | Value (Doll) |
| --- | --- |
| `after_first_peel(nav).complete()` | `Here(layer)` |
| `.into_parent().complete()` | `Up(Here(app))` |
| bare `.into_parent().complete()` | `Up(app)` |

## Rules

1. Origin `Out` fixed for the leave; every focus holds `finish: Focus → Out` (as wrap type-state).
2. One peel = laserbeam `into_parent` + compose one `Up` lift.
3. `complete` = run `finish` = nested `Up`∘…∘`Here` (or bare `Up`).
4. Either at each layer; public `Here`/`Up`.
5. Real `PathMut` / `&mut T` only.

## Relation to invalidation

Parent matches `Here` / `Up` on the child’s `Out`. Claim/posts separate.
