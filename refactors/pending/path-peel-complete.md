# PathMut peel + complete (Here / Up)

Not done. Prefactor for `invalidation.md`. Depends on `ascend-to-ancestor.md` (landed; uses `path_nest!`).

Everything below is the implementation, compile-checked against the real `laserbeam::PathMut`: the traits, the newtype, the distance-indexed impls at depth 12, every peel value, the parent-composition seam, root uniformity (one generic handler binding at nav and at the root), and the negatives (over-peel, off-chain complete, route-parented complete).

## What

The leave API is on the path values themselves. No user-facing constructor:

```text
path.complete()                                // Here(path)
path.into_parent().complete()                  // Up(Here(parent))
path.into_parent().into_parent().complete()    // Up(Up(root))
```

Peel is laserbeam `PathMut::into_parent`. `complete` is laserbeam's `Complete<O>` trait method, `O` the origin path, and it returns the newtype `Completed<O>`. A handler receives a path and returns `path.complete()` or `path.into_parent().complete()`; its signature names the origin path, not a nest:

```rust
fn handler<'a>(path: NavPath<'a>) -> Completed<NavPath<'a>>
```

So there are no new type aliases to write or to derive: `Completed<NavPath<'a>>` is built from the path alias consumers already have. The bind derive is untouched; the whole feature is laserbeam-only.

The root is uniform, not special. `Completed<AppPath>` exists and wraps the bare root path (the only place a leave from the root can stop), so a root handler has the same shape as any other and one generic handler binds at every depth, root included:

```rust
fn stay<P: Complete<P> + HasStop>(path: P) -> Completed<P> {
    path.complete()
}

fn to_root<'a, P>(path: P) -> Completed<P>
where
    P: IntoAncestor<MercuryPath<'a>> + HasStop,
    MercuryPath<'a>: Complete<P>,
{
    path.into_ancestor().complete()
}
```

At `P = MercuryPath` these are the identity ancestor plus the root's own zero-peel complete; at any deeper `P` they are the same code with more `Up` layers. Every node's dispatch returns `Completed<Self::Path>`, root included.

Runtime peel depth unifies to one return type, and a `Completed` can only be produced by `complete` (its constructor is private).

## The types

```text
Completed<P>  — a completed leave from origin P; wraps P's Stop
Stop<H, U>    — Here(H): stopped at this path | Up(U): went above
Above::Up     — what a completed leave hands upward past a child of this path:
                the root path itself, or Completed<this path>
HasStop::Stop — a path's stop layer: Stop<Self, parent's Up> for a PathMut;
                the bare path for a root, which can only stop at itself
```

Expanded for the test tree (`App → Layer → Nav`):

```text
Completed<NavPath>   wraps  Stop<NavPath, Completed<LayerPath>>
Completed<LayerPath> wraps  Stop<LayerPath, AppPath>   // bare root in Up
Completed<AppPath>   wraps  AppPath                    // bare; no Stop layer
```

The `Up` payload of a child's `Completed` IS the parent's own `Completed` type, so a parent forwards it unchanged (see Parent match).

## laserbeam additions (all of them)

Below the ancestor machinery, after `path_nest!`:

```rust
/// Where a leave stopped: at this path, or somewhere further up.
///
/// No derives: paths are neither `Debug` nor `PartialEq`; consumers
/// destructure.
pub enum Stop<H, U> {
    Here(H),
    Up(U),
}

/// What a completed leave hands upward once it has peeled past a child of
/// this path: the root path itself, or the completed leave from this path.
pub trait Above {
    type Up;
}

impl<'a, R> Above for &'a mut R {
    type Up = &'a mut R;
}

impl<N, P: Above> Above for PathMut<N, P> {
    type Up = Completed<PathMut<N, P>>;
}

/// A path's stop layer: stopped here, or went above. A root path can only
/// stop at itself, so its layer is the bare path.
pub trait HasStop {
    type Stop;
}

impl<N, P: Above> HasStop for PathMut<N, P> {
    type Stop = Stop<PathMut<N, P>, P::Up>;
}

impl<'a, R> HasStop for &'a mut R {
    type Stop = &'a mut R;
}

/// A completed leave from origin `P`: where the peeling stopped.
///
/// Only `complete` constructs one; `new` is private.
pub struct Completed<P: HasStop> {
    stop: P::Stop,
}

impl<P: HasStop> Completed<P> {
    fn new(stop: P::Stop) -> Self {
        Self { stop }
    }

    pub fn into_inner(self) -> P::Stop {
        self.stop
    }
}

/// Complete a leave from origin `O` at this path: wrap the focus into
/// `Completed<O>` at its chain position.
///
/// `O` is a type parameter, not an associated type, because one focus
/// completes into every `Completed` whose chain contains it (a `LayerPath`
/// into `Completed<NavPath>`, `Completed<TypingPath>`, `Completed<LayerPath>`).
/// The call site's expected type pins `O`: the dispatch return type, or an
/// annotation.
///
/// Impls are indexed by peel distance, like `HasAncestor`: one impl for zero
/// peels at any depth, then per distance one impl for a focus still on the
/// chain and one for a focus at the root. Unifying two distances needs a type
/// that contains itself, which the occurs check rejects, so no phantom index
/// is needed. Off-chain completes have no impl and do not compile.
pub trait Complete<O: HasStop> {
    fn complete(self) -> Completed<O>;
}

/// One `Completed::new(Stop::Up(..))` per peeled-past type parameter.
macro_rules! up_wrap {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        Completed::new(Stop::Up(up_wrap!($e $(, $rest)*)))
    };
}

/// Stopping at the origin: zero peels, every depth, one impl.
impl<N, P: Above> Complete<PathMut<N, P>> for PathMut<N, P> {
    fn complete(self) -> Completed<PathMut<N, P>> {
        Completed::new(Stop::Here(self))
    }
}

/// Stopping at the root, for a leave that began there: the bare path.
impl<'a, R> Complete<&'a mut R> for &'a mut R {
    fn complete(self) -> Completed<&'a mut R> {
        Completed::new(self)
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
            fn complete(self) -> Completed<path_nest!(PathMut<N, P>, $($done,)* $head)> {
                up_wrap!(Completed::new(Stop::Here(self)), $($done,)* $head)
            }
        }

        impl<'a, R, $($done,)* $head> Complete<path_nest!(&'a mut R, $($done,)* $head)>
            for &'a mut R
        {
            fn complete(self) -> Completed<path_nest!(&'a mut R, $($done,)* $head)> {
                up_wrap!(self, $($done,)* $head)
            }
        }

        complete_impls!([$($done,)* $head] $(, $rest)*);
    };
}

complete_impls!([], N1, N2, N3, N4, N5, N6, N7, N8, N9, N10, N11, N12);
```

Impl count: 2 + 2×12 = 26, once, for every tree.

What the distance-1 pair expands to, for reference:

```rust
impl<N1, N, P: Above> Complete<PathMut<N1, PathMut<N, P>>> for PathMut<N, P> {
    fn complete(self) -> Completed<PathMut<N1, PathMut<N, P>>> {
        Completed::new(Stop::Up(Completed::new(Stop::Here(self))))
    }
}

impl<'a, R, N1> Complete<PathMut<N1, &'a mut R>> for &'a mut R {
    fn complete(self) -> Completed<PathMut<N1, &'a mut R>> {
        Completed::new(Stop::Up(self))
    }
}
```

## Call sites

```text
// in Nav dispatch (return type Completed<NavPath<'a>>)
path.complete()                                // Here(nav)
path.into_parent().complete()                  // Up(Here(layer))
path.into_parent().into_parent().complete()    // Up(Up(app))

// in Layer dispatch (return type Completed<LayerPath<'a>>)
path.complete()                                // Here(layer)
path.into_parent().complete()                  // Up(app)

// in App dispatch (return type Completed<AppPath<'a>>)
path.complete()                                // the bare &mut App, wrapped
```

Handwritten handlers use method syntax, so their files need `use laserbeam::Complete;` (as they already need `IntoAncestor` for `.into_ancestor()`). Generated dispatch needs nothing in scope: it emits the fully qualified `::laserbeam::Complete::complete(path)`, like every other path in the macro's expansions. A `complete` outside return position needs an annotation.

## Parent match

One `into_inner` per level; the `Up` payload is already the parent's own `Completed`:

```rust
match nav_completed.into_inner() {
    Stop::Here(nav_path) => {
        // posts at nav; then e.g. nav_path.into_parent().complete()
    }
    Stop::Up(rest) => rest, // rest: Completed<LayerPath> — return it unchanged
}

match layer_completed.into_inner() {
    Stop::Here(layer_path) => { /* … */ }
    Stop::Up(app) => { /* app: &mut App */ }
}
```

Posts when receiving `Up`: `invalidation.md`.

## Route-enum parents

A path whose parent is a route enum (`TitlePath = PathMut<Title, TitleParent>`) has no `Above` impl on its parent, so `Completed<TitlePath>` and every `complete` involving it do not compile. Multi-parent leaves are out, the same stance `HasAncestor` documents.

## Tests

laserbeam side, beside `ancestor_tests`, on a local three-level tree; plus the shape pin:

```rust
fn shapes<'a>(nav: Completed<NavPath<'a>>) {
    let stop: Stop<NavPath<'a>, Completed<LayerPath<'a>>> = nav.into_inner();
    if let Stop::Up(rest) = stop {
        let _: Stop<LayerPath<'a>, AppPath<'a>> = rest.into_inner();
    }
}
```

```rust
#[test]
fn complete_at_nav() {
    let mut app = tree(/* nav.hits = 7 */);
    let out: Completed<NavPath<'_>> = nav_path(&mut app).complete();
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
    let out: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
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
    let out: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().into_parent().complete();
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
    let out: Completed<LayerPath<'_>> = layer_path(&mut app).into_parent().complete();
    let Stop::Up(root) = out.into_inner() else {
        panic!("expected Up(app)");
    };
    root.hits = 1;
    assert_eq!(app.hits, 1);
}

#[test]
fn root_completes_bare() {
    let mut app = tree(/* app.hits = 0 */);
    let out: Completed<AppPath<'_>> = (&mut app).complete();
    let root = out.into_inner(); // bare &mut App, no Stop layer
    root.hits = 5;
    assert_eq!(app.hits, 5);
}

#[test]
fn same_generic_handler_at_nav_and_root() {
    // stay::<NavPath>  → Here(nav)
    // stay::<AppPath>  → bare &mut App
    // to_root::<NavPath> → Up(Up(app));  to_root::<AppPath> → bare &mut App
    // (stay / to_root as defined in What)
}

#[test]
fn all_peel_depths_unify() {
    fn all_depths(nav: NavPath<'_>, branch: u8) -> Completed<NavPath<'_>> {
        match branch {
            0 => nav.complete(),
            1 => nav.into_parent().complete(),
            _ => nav.into_parent().into_parent().complete(),
        }
    }
    // one match per branch, asserting the arm as above
}

// The composition seam invalidation builds on:
fn parent_returns_up_payload<'a>(child: Completed<NavPath<'a>>) -> Completed<LayerPath<'a>> {
    match child.into_inner() {
        Stop::Here(nav) => nav.into_parent().complete(),
        Stop::Up(rest) => rest,
    }
}
```

bind side: the same five, on the real `App` tree from `tests/common`, via the paths the macro builds.

trybuild — must not compile:

```rust
nav.into_parent().into_parent().into_parent();     // no into_parent on &mut App
let _: Completed<LayerPath<'_>> = nav.complete();  // off-chain: no impl (E0277)
let _: Completed<AppPath<'_>> = nav.complete();    // only the root completes a root leave
fn g(a: Completed<TitlePath<'_>>) {}               // route parent: no Above
```

## Ordered changes

### 1 — laserbeam: `Stop`, `Above`, `HasStop`, `Completed`, `Complete`, `up_wrap!`, `complete_impls!` + depth-12 invocation; unit tests + shape pin

### 2 — bind: the five tests on the real `App` tree; trybuild negatives

### 3 — invalidation: dispatch returns `Completed<Self::Path>` for every node, root included; parent matches `into_inner`; posts on `Up`

## Rules

1. `Place::Path` only: `&mut Root` or `PathMut<Self, ParentPath>`.
2. `Completed<P>` is the one leave type, root included; a `PathMut`'s inner layer is `Stop<P, parent's Up>` with the root path bare at the bottom; a root's inner layer is the bare path itself.
3. Peel is laserbeam `PathMut::into_parent` only; no wrapper around paths.
4. `complete` is `laserbeam::Complete<O>::complete`, impls only in laserbeam, indexed by peel distance; `O` pinned by the dispatch return type.
5. Only `complete` constructs a `Completed`; consumers get `into_inner` and nothing else.
6. Every node's dispatch returns `Completed<Self::Path>`; a parent returns the `Up` payload unchanged or completes its own leave; a generic handler that completes carries a `HasStop` bound.
7. Over-peel past root, off-chain `complete` (including a non-root completing a root leave), and route-parented completes do not compile.
8. Arms: `Here` / `Up`. Nothing is emitted by the bind derive; consumers write `Completed<XPath>` with their existing path aliases.
