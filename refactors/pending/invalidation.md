# Invalidation: descent schedules, ascent returns a path doll

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post. Where the owned path sits is the value dispatch returns — not hop counters.

## Model

Spine `0 → A → B → C`. Descent builds a path to `C`:

```text
PathMut<C, PathMut<B, PathMut<A, 0Path>>>
```

Internally (bind only), ascent is a nested doll:

```text
// C’s internal doll
Result<BPath, Result<APath, 0Path>>

Ok(b)           still own B
Err(Ok(a))      B gone; own A
Err(Err(z))     own 0 only
```

That `Result` nest is **private to the bind crate**. User code and expand outside bind never name it, never construct `Ok`/`Err` for it.

Public surface:

1. `path.into_parent()` — peel (laserbeam). Returns the parent path by value.
2. `parent.complete()` — package that path into a **marker that must be returned**. The marker holds the private doll (or the path that bind will pack into the doll).
3. Dispatch returns the marker type (`Ascent` / `Return`). Caller recovers a path only by bind-private unpack of that marker.

```rust
// normal leave C: peel once, complete, return marker to caller
path.into_parent().complete()

// kill to 0: peel to 0, complete, return marker
path.into_parent().into_parent().into_parent().complete()
```

Path recovery is only through that return. After `into_parent`, the child layer is gone locally. Drop the marker and the path is dropped. No side channel.

### `complete()`

```rust
// On any path value you still own after peels (ParentPath, RootPath, …):
fn complete(self) -> Return<Self>;
```

`Return<P>` is `#[must_use]`. It is the only thing a leave/kill path is allowed to return into the ascent machinery. Fields private. No public accessor for the path except through bind’s match helpers used by the derive.

What is inside:

```rust
// crates/bind — public newtype, private payload
pub struct Return<P> {
    path: P, // private
}

impl<P> Return<P> {
    pub fn complete(path: P) -> Self {
        Self { path }
    }
}

// path.complete() is sugar for Return::complete(path)
```

Bind packs `Return<P>` into the node’s private doll when the return type of `dispatch` demands it (type inference + crate-private trait, or derive-emitted `bind::pack_*` calls). User never writes `Err(Ok(a))`.

### One level up (bind-private match)

Parent derive does not match `Result` in user-visible code. It calls bind helpers that unpack the private doll:

```rust
// conceptual — only bind can see Result
// got: Ascent of child = private Result<ThisPath, Rest>

match bind::unpack(got) {
    bind::Step::Here(path) => {
        // posts + exclusive with path
        path.into_parent().complete() // or pack into parent Ascent
    }
    bind::Step::Up(rest) => {
        // posts dropped (pre-snap only)
        rest // already parent’s remaining ascent
    }
}
```

Public names for the step enum can be `Here`/`Up` in bind; they are not the raw `Result`. The recursive doll layout stays an implementation detail of `Ascent`’s type parameter or a private type alias.

Root boundary: parent ascent is bare `0Path` (or `Return<0Path>`). Last peel completes to that; no extra `Ok` layer. No `Infallible`, no `Result<0Path, !>`.

### Kill vs normal leave

Both end the same way: peels then `complete()`, return the marker.

```rust
// Normal leave Inner (framework or handler):
path.into_parent().complete()
// packs as Here(OuterPath) inside Inner’s ascent

// Kill to root:
path.into_parent().into_parent().complete()
// packs as Up(RootPath) inside Inner’s ascent
```

How many peels chooses the stop level. Bind’s pack maps `Return<P>` → the private nest for `Self::Ascent` where `P` is that stop path type. Pack impls live in bind (and/or are emitted next to the derive for that node’s spine). Users do not construct the nest.

### Claim

Separate carrier, root-owned slot. Not part of the path doll.

```rust
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}
```

### Posts

Scheduled set is final; posts run on both arms of the unpack.

- Here: posts get `&mut path`.
- Up: no path at this level; pre-snap only.

## Types (`crates/bind`)

```rust
/// Marker: path recovered for the caller. Must be returned. Payload private.
#[must_use]
pub struct Return<P> {
    path: P,
}

impl<P> Return<P> {
    pub const fn new(path: P) -> Self {
        Self { path }
    }

    pub fn complete(path: P) -> Self {
        Self::new(path)
    }
}

/// Extension on path values after into_parent.
pub trait Complete: Sized {
    fn complete(self) -> Return<Self>;
}

impl<P> Complete for P {
    fn complete(self) -> Return<Self> {
        Return::new(self)
    }
}

/// One unpack step for the derive. Not a public Result.
pub enum Step<Here, Up> {
    Here(Here),
    Up(Up),
}

// Private doll layout (crate-private type alias or newtype chain).
// type Doll<H, U> = Result<H, U>;  // only referenced inside bind
// pub struct Ascent<D>(D);         // D is Doll; field private

pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}

impl<'c> Claim<'c> {
    pub fn new(slot: &'c mut Option<()>) -> Self {
        Self { slot }
    }

    pub fn is_taken(&self) -> bool {
        self.slot.is_some()
    }

    pub fn try_take(&mut self) -> Option<()> {
        if self.slot.is_some() {
            None
        } else {
            *self.slot = Some(());
            Some(())
        }
    }

    pub fn with_exclusive<P, E>(
        &mut self,
        path: &mut P,
        body: impl FnOnce(&mut P) -> Vec<E>,
    ) -> Vec<E> {
        match self.try_take() {
            None => Vec::new(),
            Some(()) => body(path),
        }
    }
}

// Crate-private (or pub for derive via bind:: paths only — not for app code):
// fn unpack<H, U>(ascent: Ascent<Result<H, U>>) -> Step<H, Ascent<U>>;
// fn pack_here<H, U>(r: Return<H>) -> Ascent<Result<H, U>>;
// fn pack_up from Return at deeper path — per spine / sealed trait in bind.
```

Dispatch:

```rust
pub trait Dispatch<M: Bindings>: Place {
    /// Opaque ascent out of this node. Contains private doll.
    type Ascent<'a>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a;
}
```

`type Ascent<'a>` is a bind newtype over the private nest, not a public `Result`.

## User signatures

```rust
// pre — descent
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

// post Here arm
fn post(pre_return: T, path: &mut P) -> Vec<M::Effect>;

// post Up arm — pre-snap only
fn post_dropped(pre_return: T) -> Vec<M::Effect>;

// exclusive that leaves or kills: takes path by value, returns Return<_>
fn exclusive(ev: &SourceEvent, path: P) -> (Vec<M::Effect>, Return<SomeAncestorPath>);

// exclusive that only mutates and leaves path with framework: still &mut P
// kill/leave that peels: by value + complete()
```

Leave/kill path:

```rust
fn inner_handler(_ev: &KeyEvent, path: InnerPath) -> (Vec<DemoEffect>, Return<RootPath>) {
    let root = path.into_parent().into_parent();
    (vec![], root.complete())
}
```

Framework packs `Return<RootPath>` into `Inner::Ascent` (private `Err(root)`). User never types `Result`.

## PathMut (laserbeam)

Unchanged. `into_parent(self) -> Parent` only. `complete` is on bind’s `Complete` trait (or a bind wrapper), not necessarily on laserbeam.

## Level order

```text
DESCENT: schedule opts

if child:
  child_path = PathMut::from_fn(path, ...)
  match bind::unpack(Child::dispatch(child_path, event, effs, claim)) {
    Here(path) => {
      // posts + exclusive
      path.into_parent().complete()   // pack to Self::Ascent
    }
    Up(rest) => {
      // posts dropped
      rest                            // already Self::Ascent shape
    }
  }

if leaf:
  exclusive may return Return<_> after peels + complete()
  else path.into_parent().complete()
```

## DX example

```rust
// Inner::Ascent — opaque; internally Result<OuterPath, RootPath>
// Outer::Ascent — opaque; internally RootPath (bare at boundary)

#[derive(Bind)]
#[node(parent = RootPath)]
#[binds(M)]
#[pre_post(AnyKey => (snap_child_id, after_child))]
#[post(AnyKey => only_if_intact(rearm))]
#[bind(KeyA => outer_handler)]
struct Outer {
    #[resolve_into]
    inner: Inner,
    return_home: AndReturnHome,
}

#[derive(Bind)]
#[node(parent = OuterPath)]
#[binds(M)]
#[bind(KeyA => inner_handler)]
struct Inner {
    id: ChildId,
}

fn snap_child_id(_ev: &KeyEvent, node: Node<&OuterPath, ()>) -> ChildId {
    node.parent.get().inner.id
}

fn after_child_ok(id: ChildId, path: &mut OuterPath) -> Vec<DemoEffect> {
    let live = path.get().inner.id;
    debug_assert_eq!(live, id);
    vec![]
}

fn after_child_dropped(id: ChildId) -> Vec<DemoEffect> {
    vec![log_destroyed(id)]
}

fn rearm(path: &mut OuterPath) -> Vec<DemoEffect> {
    let (guard, schedule) = arm_return_home();
    path.get_mut().return_home.guard = guard;
    vec![schedule]
}

fn outer_handler(_ev: &KeyEvent, _path: &mut OuterPath) -> Vec<DemoEffect> {
    vec![DemoEffect::SetLayerHome]
}

// Peels + complete. Returns marker. Bind packs into Inner::Ascent.
fn inner_handler(_ev: &KeyEvent, path: InnerPath) -> (Vec<DemoEffect>, Return<RootPath>) {
    (vec![], path.into_parent().into_parent().complete())
}
```

Supporting types (`ChildId`, `DemoEffect`, …) as before in earlier drafts.

## Generated: Inner (leaf)

```rust
#[automatically_derived]
impl ::bind::Dispatch<M> for Inner {
    type Ascent<'a> = /* opaque bind::Ascent newtype over private Result<OuterPath, RootPath> */
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: <Inner as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
        claim: &mut ::bind::Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a,
    {
        let opt_0: ::core::option::Option<&KeyEvent> = /* match KeyA */;

        if let ::core::option::Option::Some(ev) = opt_0 {
            if let ::core::option::Option::Some(()) = claim.try_take() {
                let (e, ret) = inner_handler(ev, path);
                ::core::iter::Extend::extend(effs, e);
                // pack Return<RootPath> -> Inner::Ascent (private Err(root))
                return ::bind::pack(ret);
            }
        }

        // normal leave
        ::bind::pack(path.into_parent().complete())
    }
}
```

`pack` is bind-public for the derive (`::bind::pack`) but only accepts `Return<_>` and only produces `Ascent`; it does not expose `Result` to the app.

## Generated: Outer

```rust
#[automatically_derived]
impl ::bind::Dispatch<M> for Outer
where
    Inner: ::bind::Dispatch<M>,
{
    type Ascent<'a> = /* opaque; internally RootPath */
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: <Outer as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
        claim: &mut ::bind::Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a,
    {
        let opt_0 = /* pre snap ChildId */;
        let opt_1 = /* post rearm scheduled? */;
        let opt_2 = /* exclusive KeyA */;

        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
        );

        match ::bind::unpack(<Inner as ::bind::Dispatch<M>>::dispatch(
            inner_path, event, effs, claim,
        )) {
            ::bind::Step::Here(mut path) => {
                if let ::core::option::Option::Some(id) = opt_0 {
                    ::core::iter::Extend::extend(effs, after_child_ok(id, &mut path));
                }
                if opt_1 {
                    ::core::iter::Extend::extend(effs, rearm(&mut path));
                }
                if let ::core::option::Option::Some(ev) = opt_2 {
                    let e = claim.with_exclusive(&mut path, |path| outer_handler(ev, path));
                    ::core::iter::Extend::extend(effs, e);
                }
                ::bind::pack(path.into_parent().complete())
            }
            ::bind::Step::Up(rest) => {
                if let ::core::option::Option::Some(id) = opt_0 {
                    ::core::iter::Extend::extend(effs, after_child_dropped(id));
                }
                rest
            }
        }
    }
}
```

## Walk: KeyA, Inner jumps to root

```text
Inner exclusive:
  claim take
  path.into_parent().into_parent().complete()  // Return<RootPath>
  pack → private Err(root)
  return Ascent

Outer unpack:
  Up(root):
    after_child_dropped
    return rest (root ascent)
```

## Walk: KeyB

```text
Inner: pack(path.into_parent().complete())  // Here(outer)
Outer unpack Here(outer):
  after_child_ok, rearm, …
  pack(outer.into_parent().complete()) → root
```

## Ordered changes

### P0 — Sink; drop ControlFlow for effects

### P1 — `Return` + `complete` + private doll + `pack`/`unpack`/`Step`

No public `Result` ascent. No hop counters.

### P2 — Exclusive via claim; leave/kill return `Return<_>` after peels

### F1 — `#[post]` on Here arm

### F2 — kill = multi-peel + `complete()`; pack chooses Up nest

### F3 — `#[pre_post]`; Up arm uses pre-snap

### F4 — `only_if_intact` = Here-arm-only post

### F5 — reshape carrier if needed

## Rules

1. Descent schedules; set final. Ascent runs every scheduled post.
2. Owned path position is the ascent return value (private doll), not counters.
3. User peels with `into_parent`, then `complete()` → `Return<P>` marker that must be returned.
4. Private `Result` nest lives only in bind. App code never constructs or matches it.
5. Derive uses `bind::pack` / `bind::unpack` / `Step::Here|Up`.
6. One level up: Here → posts → `into_parent().complete()`; Up → posts dropped → pass rest.
7. Kill = multi-peel + `complete()`; pack maps stop path into Up nest.
8. `Claim` separate. laserbeam `into_parent` is the only peel.
9. No `Infallible`, no `Result<Path, !>`. Root ascent is bare path / `Return` of path.

## Tests

- Normal walk: Here all the way; posts see paths
- Jump to root: Outer Up arm; no OuterPath
- `Return` is `#[must_use]`; complete after peels
- App crate cannot match ascent as `Result`
- Claim trap door

## Open

- Exact packing trait vs derive-emitted `pack` specializations for each node spine
- Whether `complete` lives on laserbeam paths or only via bind `Complete` trait
- Derived / enum nodes
- Exclusive on Up arm (usually skip; claim often already taken)
