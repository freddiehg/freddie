# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

On `laserbeam::PathMut`. Leave holds a function `f` (type-state). Start: `f = identity`. Each `into_parent`: peel focus, set `f := |x| f(Up(x))` (one `Up`/`Err` layer).

```text
non-root focus:  complete = f(Here(focus))   // Here then apply lifts
root focus:      complete = f_bare(focus)    // last Up holds &mut Root; no Here layer
```

Root bottoms the nest as bare `APath` — no `Result<APath, !>`.

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

## Result / Doll spelling (A = root)

Two concrete shapes. Prefer **no `!`**.

### Preferred: root path is bare terminal (no `!`)

Spine: C → B → A with `APath = &mut Root` (real root `Place::Path`).

```text
Out = Doll<CPath, Doll<BPath, APath>>
// = Result<CPath, Result<BPath, APath>>
```

| stop | value | Result |
| --- | --- | --- |
| C | `Here(c)` | `Ok(c)` |
| B | `Up(Here(b))` | `Err(Ok(b))` |
| A (root) | `Up(Up(a))` | `Err(Err(a))` |

Innermost type is **`APath`**, not `Doll<APath, something>`. So at root you never need `Here(a)` / `Ok(a)` as a layer — the last `Up`/`Err` **holds the root path**.

That is the concrete meaning of “avoid the bang”: bottom out the nest on the root path type.

```text
complete at C:  f = id                 →  Here(c)
complete at B:  f = Up                 →  Up(Here(b))
complete at A:  f = Up∘Up              →  Up(Up(a))     // not Up(Up(Here(a)))
```

So `complete = f(Here(focus))` holds for **non-root** foci. At root focus, `complete = f_root(focus)` where the last lift is bare `Up(path)` (Terminal), not `Up(Here(path))`.

```rust
// Non-root stop (still a PathMut in the nest as Here)
impl LeavePath</* BPath */, ComposeUp</*C*/, Id</* rest */>>> {
    fn complete(self) -> Doll<CPath, Doll<BPath, APath>> {
        Doll::Up(Doll::Here(self.focus)) // Up(Here(b))
    }
}

// Root stop — derive/impl for focus = APath = &mut Root only
impl LeavePath<APath, ComposeUp<CPath, ComposeUp<BPath, Terminal>>> {
    fn complete(self) -> Doll<CPath, Doll<BPath, APath>> {
        Doll::Up(Doll::Up(self.focus)) // Up(Up(a)) — bare a
    }
}
```

`into_parent` into root installs `Terminal` instead of `Id`/`NoWrap` as the innermost wrap:

```rust
// peel B → A when A is &mut Root (bare rest of B’s doll layer)
// Before: LeavePath<BPath, ComposeUp<C, Id<APath>>>  or NoWrap with Rest = APath
// After:  LeavePath<APath, ComposeUp<C, ComposeUp<B, Terminal>>>

impl LeavePath<PathMut<BNode, APath>, ComposeUp<CPath, Id<APath>>> {
    fn into_parent(self) -> LeavePath<APath, ComposeUp<CPath, ComposeUp<BPath, Terminal>>> {
        LeavePath {
            focus: self.focus.into_parent(), // &mut Root
            f: /* compose Terminal as innermost */,
        }
    }
}
```

How we know “parent is root”: `Parent = &mut T` (or whatever the root `Place::Path` is), not `PathMut<_, _>`. One `into_parent` impl for `PathMut<_, &mut T>` ends in `Terminal`; impls for `PathMut<_, PathMut<_, _>>` keep nesting `Id`/`Here`.

### Optional: `Result<APath, !>`

```text
Out = Result<CPath, Result<BPath, Result<APath, !>>>
```

Then every layer is uniform `complete = f(Ok(focus))` / `f(Here(focus))`, and at A:

```text
Ok(a) : Result<APath, !>   // Err is uninhabited
f(Ok(a)) = Err(Err(Ok(a)))
```

Works, but:

- Rust `!` / `Infallible` in public aliases is noisy.
- Root `&mut T` is not naturally `Result<&mut T, !>`.

Prefer bare terminal unless a later generic forces uniform layer shape. Derive for root already treats root specially (`Path = &mut Self`, no `from_fn` parent); bare `APath` in the nest matches that.

### Root as leave origin

Leave **started at root** (dispatch only on `&mut Root`): nest is not a Doll — Out is `()` or unused; root only peels into children via `from_fn`, does not `leave_at(root)` through the same machine. Child returns `Doll<ChildPath, RootPath>` / `Result<ChildPath, RootPath>`; root matches `Here`/`Up` and is done.

## Existing peel

```rust
PathMut::into_parent(self) -> Parent
```

## Tests (when implemented)

| Start | complete | value |
| --- | --- | --- |
| `leave_at(c)` | | `Here(c)` |
| `leave_at(c).into_parent()` | | `Up(Here(b))` |
| `leave_at(c).into_parent().into_parent()` | | `Up(Up(a))` bare root |
| type of last | | `Doll<CPath, Doll<BPath, APath>>` with `APath = &mut Root` |

## Rules

1. `complete = f(Here(focus))` on non-root foci; at root focus `complete = … Up(…(focus))` bare (Terminal).
2. Start: `f = id`.
3. `into_parent`: laserbeam peel; `f := |x| f(Up(x))` (one layer); peel to root installs Terminal innermost.
4. Out = `Doll<C, Doll<B, A>>` with A root path type — **no `!`**.
5. Either per non-terminal layer; public Here/Up.

## Relation to invalidation

Child leave returns origin `Out` for the child’s path type. Parent matches `Here` / `Up`. Claim/posts separate.
