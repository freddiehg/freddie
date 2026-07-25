# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

On existing `laserbeam::PathMut`:

```rust
leave(path).into_parent().into_parent().complete()
// → nested Doll::Here / Doll::Up
```

Peel is `PathMut::into_parent`. Public arms **Here** / **Up**.

## Public doll

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}
```

Name the whole nest by leave origin path/node as needed (`AscentOf` alias or derive `type Ascent<'a> = …`). Do not spell the nest in ordinary signatures.

## Core: Option + function per layer

No `AsHere` / `AsUp` pack types. The leave carrier holds the focus and a wrap that is **none** or **a function** (one layer at a time; nest of functions as you peel).

### Start

Path as first created for leave (after the first peel off the node, or at start of leave — same idea):

```text
focus = parent path (or current stop path)
wrap  = None
```

```rust
// complete with None → wrap in Here
fn complete(self) -> … {
    Doll::Here(self.focus)
}
```

### `into_parent`

```text
old focus → PathMut::into_parent() → new focus
wrap becomes Some(f)
```

`f` is the function for **this** skipped layer: when the leave eventually `complete`s further up, this layer contributes a `Doll::Up(…)`.

Each further `into_parent` builds another function that closes over the previous wrap — a Russian doll, **one layer at a time**.

```text
// after first into_parent
wrap = Some(f1)   // f1 wraps the eventual stop in Up for the first skipped PathMut

// after second into_parent
wrap = Some(f2)   // f2 uses f1 (or the previous Some) so complete nests Up(Up(…))
```

A boolean “are we past the first peel?” is not enough for the types: each layer has a different skipped path type and a different rest type. The wrap has to be a **function** (type-state of composable wraps) so each peel’s `Up` is well-typed.

### `complete`

```text
wrap is None  →  Here(focus)
wrap is Some(f) →  f applied so the stop focus becomes nested Up(…Here(focus)) or Up(…Up(focus))
```

One layer’s worth of work per peel; `complete` only runs the doll of functions already installed.

## Type-state (what the Option becomes in Rust)

`Option<fn…>` cannot name a different function type at each peel depth. Represent None / Some as types:

```rust
/// No wrap yet — complete is Here.
pub struct NoWrap;

/// One Up layer: skipped path type Q, then inner wrap.
pub struct UpWrap<Q, Inner> {
    inner: Inner,
    _q: core::marker::PhantomData<Q>,
}
```

```rust
pub struct LeavePath<P, W> {
    focus: P,
    wrap: W, // NoWrap or UpWrap<…, UpWrap<…, NoWrap>>
}
```

```rust
// --- complete ---

impl<P> LeavePath<P, NoWrap> {
    pub fn complete(self) -> Doll<P, /* unreachable rest — see note */> {
        // With NoWrap, Out is just "Here only" for this stop.
        // For a full origin doll type, first peel starts as LeavePath<Parent, NoWrap>
        // and complete is Doll::Here(parent) : Doll<Parent, ParentRest> only if we
        // type it as the origin Out — Rest is still in the type as the unused U parameter
        // or origin Out is exactly Doll<Parent, Rest> with Rest = thinner ascent of Parent.
        Doll::Here(self.focus)
    }
}

impl<P, Q, Inner> LeavePath<P, UpWrap<Q, Inner>>
where
    // Inner complete builds the rest from P
{
    pub fn complete(self) -> Doll<Q, /* Inner’s complete type */> {
        // Russian doll one layer:
        //   inner produces rest from focus
        //   this layer wraps: Up(rest)
        Doll::Up(/* inner.complete_with(self.focus) */)
    }
}
```

Concrete bodies (single layer each):

```rust
impl<P> LeavePath<P, NoWrap> {
    pub fn complete(self) -> /* Here-only at P; typed as origin Out when Rest known */ {
        // value:
        // Doll::Here(self.focus)
    }
}

// After one into_parent from PathMut<N, P> with NoWrap:
// LeavePath { focus: P, wrap: UpWrap<PathMut<N, P>, NoWrap> }

impl<Node, Parent> LeavePath<Parent, UpWrap<PathMut<Node, Parent>, NoWrap>> {
    pub fn complete(self) -> Doll<PathMut<Node, Parent>, Parent /* or Doll if Parent not terminal */> {
        let LeavePath { focus, wrap: UpWrap { .. } } = self;
        // stop at Parent with Here, then this layer Up:
        Doll::Up(Doll::Here(focus))
        // bare terminal Parent (e.g. &mut Root): Doll::Up(focus) using AsTerminal-style
        // — if Parent is not nested further, Up(focus) not Up(Here(focus))
    }
}
```

Bare root (`&mut T`) vs nested parent: one layer wraps `Up(Here(p))` when the stop still uses Here; `Up(p)` when the stop **is** the terminal path (no further Here). Same as before; choose by whether `Parent` is `PathMut` or `&mut T` when writing the `into_parent` impl.

### `into_parent` installs the next function

```rust
impl<Node, Parent> LeavePath<PathMut<Node, Parent>, NoWrap> {
    pub fn into_parent(self) -> LeavePath<Parent, UpWrap<PathMut<Node, Parent>, NoWrap>> {
        LeavePath {
            focus: self.focus.into_parent(), // laserbeam
            wrap: UpWrap {
                inner: NoWrap,
                _q: core::marker::PhantomData,
            },
        }
    }
}

impl<Node, Parent, Q, Inner> LeavePath<PathMut<Node, Parent>, UpWrap<Q, Inner>> {
    pub fn into_parent(self) -> LeavePath<Parent, UpWrap<Q, UpWrap<PathMut<Node, Parent>, Inner>>> {
        // or UpWrap { inner: old wrap, … } so the doll of functions grows inward
        LeavePath {
            focus: self.focus.into_parent(),
            wrap: UpWrap {
                inner: UpWrap {
                    inner: self.wrap.inner, // previous doll
                    _q: core::marker::PhantomData::<PathMut<Node, Parent>>,
                },
                _q: self.wrap._q, // keep outer skipped types — structure as needed
            },
        }
    }
}
```

Exact `UpWrap` nesting order must match `complete` application order so types equal the known origin nest (`Doll<Layer, Doll<App, App>>` etc.). One layer per `into_parent`.

### Start of leave

```rust
/// First peel off this node’s PathMut; wrap = None (NoWrap).
pub fn after_first_peel<Node, Parent>(
    path: PathMut<Node, Parent>,
) -> LeavePath<Parent, NoWrap> {
    LeavePath {
        focus: path.into_parent(),
        wrap: NoWrap,
    }
}
```

```text
after_first_peel(nav).complete()
  → Here(layer)

after_first_peel(nav).into_parent().complete()
  → Up(…)   // function installed by into_parent ran
```

## Existing path types only

```rust
// laserbeam
PathMut::into_parent(self) -> Parent

// bind Place paths (examples)
type AppPath<'a> = &'a mut App;
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;
```

No parallel spine type.

## Origin nest type (no trait)

At each known path alias, the leave return type is a concrete `Doll` nest (derive or hand alias). Example:

```rust
// leave started at NavPath
type NavAscent<'a> = Doll<LayerPath<'a>, Doll<AppPath<'a>, AppPath<'a>>>;
// or bare terminal: Doll<LayerPath<'a>, AppPath<'a>>
```

`complete()`’s return type is that alias. Fixed by the path you started from + how many peels, not by a trait.

## Tests (when implemented)

On real `from_fn` `PathMut` trees.

| Call | Value |
| --- | --- |
| `after_first_peel(nav).complete()` | `Here(layer)` |
| `after_first_peel(nav).into_parent().complete()` | `Up(…)` at app |
| type | equals the path’s leave alias |

## Rules

1. Start: `wrap = None` (`NoWrap`); `complete` → `Here(focus)`.
2. `into_parent`: laserbeam peel; install/compose one wrap function (`UpWrap`) for the skipped layer.
3. `complete` with wraps: Russian doll of those functions, one layer each.
4. Functions (type-state), not a boolean — types differ per layer.
5. Public `Doll::Here` / `Doll::Up` only.
6. Focus is always real `PathMut` / `&mut T`.

## Relation to invalidation

Leave/kill uses this. Parent matches `Here` / `Up`. Claim/posts separate.
