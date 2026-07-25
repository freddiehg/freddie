# PathMut peel + complete (Here / Up)

Not done. Prefactor for `invalidation.md`. Depends on `ascend-to-ancestor.md` (uses `path_nest!`).

Everything below is the implementation, compile-checked against the real `laserbeam::PathMut`: the traits, the newtype, the distance-indexed impls at depth 12, every peel value, the parent-composition seam, and the negatives (over-peel, off-chain complete, `Ascent` of a root path).

## What

The leave API is on the path values themselves. No user-facing constructor:

```text
path.complete()                                // Here(path)
path.into_parent().complete()                  // Up(Here(parent))
path.into_parent().into_parent().complete()    // Up(Up(root))
```

Peel is laserbeam `PathMut::into_parent`. `complete` is laserbeam's `Complete<O>` trait method, `O` the origin path, and it returns the newtype `Ascent<O>`. A handler receives a path and returns `path.complete()` or `path.into_parent().complete()`; its signature names the origin path, not a nest:

```rust
fn handler<'a>(path: NavPath<'a>) -> Ascent<NavPath<'a>>
```

So there are no new type aliases to write or to derive: `Ascent<NavPath<'a>>` is built from the path alias consumers already have. The bind derive is untouched; the whole feature is laserbeam-only.

Runtime peel depth unifies to one return type, and an `Ascent` can only be produced by `complete` (its constructor is private).

## The types

```text
Ascent<P>     — a completed leave from origin P; wraps P's Stop
Stop<H, U>    — Here(H): stopped at this path | Up(U): went above
Above::Up     — what an ascent hands upward past a child of this path:
                the root path itself, or Ascent<this path>
HasStop::Stop — a non-root path's stop layer: Stop<Self, parent's Up>
```

Expanded for the test tree (`App → Layer → Nav`):

```text
Ascent<NavPath>   wraps  Stop<NavPath, Ascent<LayerPath>>
Ascent<LayerPath> wraps  Stop<LayerPath, AppPath>          // bare root in Up
Ascent<AppPath>   does not typecheck: &mut App has no HasStop impl
```

The `Up` payload of a child's ascent IS the parent's own ascent type, so a parent forwards it unchanged (see Parent match).

## laserbeam additions (all of them)

Below the ancestor machinery, after `path_nest!`:

```rust
/// Where an ascent stopped: at this path, or somewhere further up.
///
/// No derives: paths are neither `Debug` nor `PartialEq`; consumers
/// destructure.
pub enum Stop<H, U> {
    Here(H),
    Up(U),
}

/// What an ascent hands upward once it has peeled past a child of this path:
/// the root path itself, or this path's own ascent.
pub trait Above {
    type Up;
}

impl<'a, R> Above for &'a mut R {
    type Up = &'a mut R;
}

impl<N, P: Above> Above for PathMut<N, P> {
    type Up = Ascent<PathMut<N, P>>;
}

/// A non-root path's stop layer: stopped here, or went above.
pub trait HasStop {
    type Stop;
}

impl<N, P: Above> HasStop for PathMut<N, P> {
    type Stop = Stop<PathMut<N, P>, P::Up>;
}

/// A completed leave from origin `P`: where the peeling stopped.
///
/// Only `complete` constructs one; `new` is private.
pub struct Ascent<P: HasStop> {
    stop: P::Stop,
}

impl<P: HasStop> Ascent<P> {
    fn new(stop: P::Stop) -> Self {
        Self { stop }
    }

    pub fn into_inner(self) -> P::Stop {
        self.stop
    }
}

/// Complete a leave from origin `O` at this path: wrap the focus into
/// `Ascent<O>` at its chain position.
///
/// `O` is a type parameter, not an associated type, because one focus
/// completes into every ascent whose chain contains it (a `LayerPath` into
/// `Ascent<NavPath>`, `Ascent<TypingPath>`, `Ascent<LayerPath>`). The call
/// site's expected type pins `O`: the dispatch return type, or an annotation.
///
/// Impls are indexed by peel distance, like `HasAncestor`: one impl for zero
/// peels at any depth, then per distance one impl for a focus still on the
/// chain and one for a focus at the root. Unifying two distances needs a type
/// that contains itself, which the occurs check rejects, so no phantom index
/// is needed. Off-chain completes have no impl and do not compile.
pub trait Complete<O: HasStop> {
    fn complete(self) -> Ascent<O>;
}

/// One `Ascent::new(Stop::Up(..))` per peeled-past type parameter.
macro_rules! up_wrap {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        Ascent::new(Stop::Up(up_wrap!($e $(, $rest)*)))
    };
}

/// Stopping at the origin: zero peels, every depth, one impl.
impl<N, P: Above> Complete<PathMut<N, P>> for PathMut<N, P> {
    fn complete(self) -> Ascent<PathMut<N, P>> {
        Ascent::new(Stop::Here(self))
    }
}

/// Two `Complete` impls per peel distance: focus still a path, and focus at
/// the root. The origin in the trait parameter is the focus wrapped in one
/// `PathMut` per skipped layer.
macro_rules! complete_impls {
    ([$($done:ident),*]) => {};
    ([$($done:ident),*], $head:ident $(, $rest:ident)*) => {
        impl<$($done,)* $head, N, P: Above> Complete<path_nest!(PathMut<N, P>, $($done,)* $head)>
            for PathMut<N, P>
        {
            fn complete(self) -> Ascent<path_nest!(PathMut<N, P>, $($done,)* $head)> {
                up_wrap!(Ascent::new(Stop::Here(self)), $($done,)* $head)
            }
        }

        impl<'a, R, $($done,)* $head> Complete<path_nest!(&'a mut R, $($done,)* $head)>
            for &'a mut R
        {
            fn complete(self) -> Ascent<path_nest!(&'a mut R, $($done,)* $head)> {
                up_wrap!(self, $($done,)* $head)
            }
        }

        complete_impls!([$($done,)* $head] $(, $rest)*);
    };
}

complete_impls!([], N1, N2, N3, N4, N5, N6, N7, N8, N9, N10, N11, N12);
```

Impl count: 1 + 2×12 = 25, once, for every tree.

What the distance-1 pair expands to, for reference:

```rust
impl<N1, N, P: Above> Complete<PathMut<N1, PathMut<N, P>>> for PathMut<N, P> {
    fn complete(self) -> Ascent<PathMut<N1, PathMut<N, P>>> {
        Ascent::new(Stop::Up(Ascent::new(Stop::Here(self))))
    }
}

impl<'a, R, N1> Complete<PathMut<N1, &'a mut R>> for &'a mut R {
    fn complete(self) -> Ascent<PathMut<N1, &'a mut R>> {
        Ascent::new(Stop::Up(self))
    }
}
```

## Call sites

```text
// in Nav dispatch (return type Ascent<NavPath<'a>>)
path.complete()                                // Here(nav)
path.into_parent().complete()                  // Up(Here(layer))
path.into_parent().into_parent().complete()    // Up(Up(app))

// in Layer dispatch (return type Ascent<LayerPath<'a>>)
path.complete()                                // Here(layer)
path.into_parent().complete()                  // Up(app)
```

`Complete` must be in scope at the call site; generated dispatch (invalidation) imports it. A `complete` outside return position needs an annotation.

## Parent match

One `into_inner` per level; the `Up` payload is already the parent's own ascent:

```rust
match nav_ascent.into_inner() {
    Stop::Here(nav_path) => {
        // posts at nav; then e.g. nav_path.into_parent().complete()
    }
    Stop::Up(rest) => rest, // rest: Ascent<LayerPath> — return it unchanged
}

match layer_ascent.into_inner() {
    Stop::Here(layer_path) => { /* … */ }
    Stop::Up(app) => { /* app: &mut App */ }
}
```

Posts when receiving `Up`: `invalidation.md`.

## Route-enum parents

A path whose parent is a route enum (`TitlePath = PathMut<Title, TitleParent>`) has no `Above` impl on its parent, so `Ascent<TitlePath>` and every `complete` involving it do not compile. Multi-parent ascent is out, the same stance `HasAncestor` documents.

## Tests

laserbeam side, beside `ancestor_tests`, on a local three-level tree; plus the shape pin:

```rust
fn shapes<'a>(nav: Ascent<NavPath<'a>>) {
    let stop: Stop<NavPath<'a>, Ascent<LayerPath<'a>>> = nav.into_inner();
    if let Stop::Up(rest) = stop {
        let _: Stop<LayerPath<'a>, AppPath<'a>> = rest.into_inner();
    }
}
```

```rust
#[test]
fn complete_at_nav() {
    let mut app = tree(/* nav.hits = 7 */);
    let out: Ascent<NavPath<'_>> = nav_path(&mut app).complete();
    let Stop::Here(mut nav) = out.into_inner() else {
        panic!("expected Here");
    };
    assert_eq!(nav.get().hits, 7);
    nav.get_mut().hits = 8;
    assert_eq!(app.layer.nav.hits, 8);
}

#[test]
fn one_peel() {
    let mut app = tree(/* nav.hits = 7 */);
    let out: Ascent<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
    let Stop::Up(rest) = out.into_inner() else {
        panic!("expected Up");
    };
    let Stop::Here(layer) = rest.into_inner() else {
        panic!("expected Up(Here(layer))");
    };
    assert_eq!(layer.get().nav.hits, 7);
}

#[test]
fn two_peels() {
    let mut app = tree(/* app.hits = 0 */);
    let out: Ascent<NavPath<'_>> = nav_path(&mut app).into_parent().into_parent().complete();
    let Stop::Up(rest) = out.into_inner() else {
        panic!("expected Up");
    };
    let Stop::Up(root) = rest.into_inner() else {
        panic!("expected Up(Up(app))");
    };
    root.hits = 3;
    assert_eq!(app.hits, 3);
}

#[test]
fn layer_origin_bare_root() {
    let mut app = tree(/* app.hits = 0 */);
    let out: Ascent<LayerPath<'_>> = layer_path(&mut app).into_parent().complete();
    let Stop::Up(root) = out.into_inner() else {
        panic!("expected Up(app)");
    };
    root.hits = 1;
    assert_eq!(app.hits, 1);
}

#[test]
fn all_peel_depths_unify() {
    fn all_depths(nav: NavPath<'_>, branch: u8) -> Ascent<NavPath<'_>> {
        match branch {
            0 => nav.complete(),
            1 => nav.into_parent().complete(),
            _ => nav.into_parent().into_parent().complete(),
        }
    }
    // one match per branch, asserting the arm as above
}

// The composition seam invalidation builds on:
fn parent_returns_up_payload<'a>(child: Ascent<NavPath<'a>>) -> Ascent<LayerPath<'a>> {
    match child.into_inner() {
        Stop::Here(nav) => nav.into_parent().complete(),
        Stop::Up(rest) => rest,
    }
}
```

bind side: the same five, on the real `App` tree from `tests/common`, via the paths the macro builds.

trybuild — must not compile:

```rust
nav.into_parent().into_parent().into_parent();  // no into_parent on &mut App
let _: Ascent<LayerPath<'_>> = nav.complete();  // off-chain: no impl (E0277)
fn f(a: Ascent<AppPath<'_>>) {}                 // root path: no HasStop
fn g(a: Ascent<TitlePath<'_>>) {}               // route parent: no Above
```

## Ordered changes

### 1 — laserbeam: `Stop`, `Above`, `HasStop`, `Ascent`, `Complete`, `up_wrap!`, `complete_impls!` + depth-12 invocation; unit tests + shape pin

### 2 — bind: the five tests on the real `App` tree; trybuild negatives

### 3 — invalidation: dispatch returns `Ascent<Self::Path>`; parent matches `into_inner`; posts on `Up`

## Rules

1. `Place::Path` only: `&mut Root` or `PathMut<Self, ParentPath>`.
2. `Ascent<P>` is the one leave type; its inner `Stop` nest is `Stop<P, parent's Up>` with the root path bare at the bottom; `Ascent` of a root path does not typecheck.
3. Peel is laserbeam `PathMut::into_parent` only; no wrapper around paths.
4. `complete` is `laserbeam::Complete<O>::complete`, impls only in laserbeam, indexed by peel distance; `O` pinned by the dispatch return type.
5. Only `complete` constructs an `Ascent`; consumers get `into_inner` and nothing else.
6. Child returns `Ascent<ChildPath>`; a parent returns the `Up` payload unchanged or completes its own leave.
7. Over-peel past root, off-chain `complete`, `Ascent` of a root path, and route-parented ascents do not compile.
8. Arms: `Here` / `Up`. Nothing is emitted by the bind derive; consumers write `Ascent<XPath>` with their existing path aliases.
