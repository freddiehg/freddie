# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

On `laserbeam::PathMut`. Leave holds a function `f` such that:

```text
complete = f(Here(focus))
```

Start: `f = identity`. Each `into_parent`: peel focus, set `f := |x| f(Up(x))` (one `Up`/`Err` layer). Nested Ups fall out of composition.

Public arms: **Here** / **Up**. Same Either as `Result` Ok/Err.

## Origin type includes the start path

Leave started at **C** (C still in the nest):

```text
Out = Doll<CPath, Doll<BPath, APath>>
// or Result<CPath, Result<BPath, APath>>

// or with explicit never further past A:
// Result<CPath, Result<BPath, Result<APath, !>>>
```

Not “first peel drops C then Out starts at B.” C is the outermost Here.

## Line of thinking (corrected)

Every leave path has `f` mapping **a one-layer doll at the current focus** into **origin Out**.

```text
complete(focus) = f(Here(focus))
```

### At C — identity

```text
f_C = id
complete = id(Here(c)) = Here(c)
// Result: Ok(c_path)
```

### C → B — give B the function `Up` / `Err`

```text
f_B = |x| f_C(Up(x)) = Up    // since f_C = id
complete = f_B(Here(b)) = Up(Here(b))
// Result: Err(Ok(b_path))
```

### B → A — compose another `Up` / `Err`

```text
f_A = |x| f_B(Up(x)) = Up ∘ Up
complete = f_A(Here(a)) = Up(Up(Here(a)))
// Result: Err(Err(Ok(a)))  if Out = Result<C, Result<B, Result<A, !>>>
// Result: Err(Err(a))      if Out = Result<C, Result<B, A>>  (A terminal in Err)
```

So “A is given `Err(Err(…))`” means **`f` is the composition of two lifts**, and complete always does `f(Here(a))` first.

One-level lift type (Result language):

```text
lift : Result<BPath, T> → Result<CPath, Result<BPath, T>>   // = Err
// general:  LocalDoll → OuterDoll  = Up / Err
```

Compose with the stop at the new focus:

```text
// at B after peel from C:
//   local stop: BPath → Result<BPath, T> = Ok / Here
//   f_B = lift_C ∘ Ok = Err ∘ Ok  →  |b| Err(Ok(b))

// at A:
//   f_A = lift_C ∘ lift_B ∘ Ok → |a| Err(Err(Ok(a))) or Err(Err(a))
```

## Either

Yes. Each layer is stop vs went further:

| Doll | Result |
| --- | --- |
| Here(p) | Ok(p) |
| Up(rest) | Err(rest) |

`Ok` / Here is **not** `!` at C or B: you can stop there. `!` only if you add an explicit “cannot go past A” layer (`Result<APath, !>`), optional.

## Data structures

```rust
pub enum Doll<H, U> {
    Here(H), // Ok
    Up(U),   // Err
}

/// f = identity. complete = Here(focus).
pub struct Id<Rest>(core::marker::PhantomData<Rest>);

/// f = |x| outer(Up(x)). Skipped path type is phantom on the layer.
pub struct ComposeUp<Skipped, Outer> {
    outer: Outer,
    _skipped: core::marker::PhantomData<Skipped>,
}

pub struct LeavePath<P, F> {
    focus: P,
    /// Type-state for f: LocalDoll-at-P → Out (via complete = f(Here(focus))).
    f: F,
}
```

### `complete` = `f(Here(focus))`

```rust
impl<P, Rest> LeavePath<P, Id<Rest>> {
    pub fn complete(self) -> Doll<P, Rest> {
        // f = id
        Doll::Here(self.focus)
    }
}

impl<P, Sk, Rest> LeavePath<P, ComposeUp<Sk, Id<Rest>>> {
    pub fn complete(self) -> Doll<Sk, Doll<P, Rest>> {
        // f = Up ∘ id  →  Up(Here(focus))
        Doll::Up(Doll::Here(self.focus))
    }
}

impl<P, Sk1, Sk2, Rest> LeavePath<P, ComposeUp<Sk1, ComposeUp<Sk2, Id<Rest>>>> {
    pub fn complete(self) -> Doll<Sk1, Doll<Sk2, Doll<P, Rest>>> {
        // f = Up ∘ Up ∘ id
        Doll::Up(Doll::Up(Doll::Here(self.focus)))
    }
}
```

Same monomorphization as before; meaning is composition of `Up` after `Here(focus)`.

### `into_parent` = peel + `f := |x| f(Up(x))`

```rust
use laserbeam::PathMut;

// At C-equivalent: focus PathMut<C, B>, f = Id<Doll<B, Rest>>
// After peel: focus B, f = ComposeUp<PathMut<C,B>, Id<Rest>>
//   complete = Up(Here(b)) : Doll<PathMut<C,B>, Doll<B, Rest>>
//
// Wait — Out for leave at C is Doll<CPath, …> where CPath = PathMut<C,B>.
// Here(c) uses CPath. After peel, Up(Here(b)) has type Doll<CPath, Doll<B, Rest>>. Yes.

impl<N, P, Rest> LeavePath<PathMut<N, P>, Id<Doll<P, Rest>>> {
    pub fn into_parent(self) -> LeavePath<P, ComposeUp<PathMut<N, P>, Id<Rest>>> {
        LeavePath {
            focus: self.focus.into_parent(),
            f: ComposeUp {
                outer: Id(core::marker::PhantomData),
                _skipped: core::marker::PhantomData,
            },
        }
    }
}

// Already composed once; another peel composes another Up inside / outside
impl<N, P, Sk, Rest> LeavePath<PathMut<N, P>, ComposeUp<Sk, Id<Doll<P, Rest>>>> {
    pub fn into_parent(
        self,
    ) -> LeavePath<P, ComposeUp<Sk, ComposeUp<PathMut<N, P>, Id<Rest>>>> {
        LeavePath {
            focus: self.focus.into_parent(),
            f: ComposeUp {
                outer: ComposeUp {
                    outer: Id(core::marker::PhantomData),
                    _skipped: core::marker::PhantomData,
                },
                _skipped: core::marker::PhantomData, // Sk preserved — structure as in working harness
            },
        }
    }
}
```

### Start at C (identity, **no** peel yet)

```rust
pub fn leave_at<N, P, Rest>(path: PathMut<N, P>) -> LeavePath<PathMut<N, P>, Id<Doll<P, Rest>>> {
    LeavePath {
        focus: path,
        f: Id(core::marker::PhantomData),
    }
}

// complete → Here(c) : Doll<CPath, Doll<P, Rest>>
// into_parent → at P with f = Up∘id → complete Up(Here(p))
```

If the first API peels immediately (`after_first_peel`), that is `leave_at(path).into_parent()` and Out for that leave no longer offers `Here(c)` — only use `leave_at` when the origin nest includes C.

## Result spelling

```text
Result<CPath, Result<BPath, APath>>
  Ok(c)           = Here(c)
  Err(Ok(b))      = Up(Here(b))
  Err(Err(a))     = Up(Up(a))           // A terminal in Err

Result<CPath, Result<BPath, Result<APath, !>>>
  Err(Err(Ok(a))) = Up(Up(Here(a)))
  Err(Err(Err(_))) impossible
```

## Existing peel

```rust
PathMut::into_parent(self) -> Parent
```

## Tests (when implemented)

| Start | complete | value |
| --- | --- | --- |
| `leave_at(c)` | | `Here(c)` |
| `leave_at(c).into_parent()` | | `Up(Here(b))` |
| `leave_at(c).into_parent().into_parent()` | | `Up(Up(…))` |

## Rules

1. `complete = f(Here(focus))` always.
2. Start: `f = id`.
3. `into_parent`: laserbeam peel; `f := |x| f(Up(x))` (one layer).
4. Either per layer; public Here/Up; Ok is not `!` where you can stop.
5. Origin nest includes the start path type as outermost Here.

## Relation to invalidation

Child leave returns origin `Out` for the child’s path type. Parent matches `Here` / `Up`. Claim/posts separate.
