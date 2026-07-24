# dispatch: an effect batch and a completion token

Not done. Reshapes `bind` dispatch so effects accumulate into one threaded `Vec`, and an exclusive handler reaches the root through a completion token rather than a raw `ascend_mut`. Behavior-identical to master: with no pre/post handlers nothing on the ascent pushes, and exactly one handler still produces the whole batch. This is the plumbing the pre/post work (`handler-kinds.md`) lands on; it ships alone.

Two shape changes drive it:

- dispatch stops returning the winner's `Output` in `Break` and instead threads one `effs: &mut Vec<M::Effect>` that the winner pushes onto. `ControlFlow<(), Path>`.
- an exclusive handler stops calling `ascend_mut` (which hands back a bare `&mut Root`) and calls `complete`, which returns a sealed `Completed` token. `Completed`'s only constructor is the ascent, so the return type forces a handler to have reached the root — it cannot compile otherwise.

## Change 1: `Bindings` names the effect

`type Output` was what one handler's `collect` produced. The batch is `Vec<Effect>`, so the marker names the item instead.

Before (`crates/bind/src/lib.rs`):

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Output;
}
```

after:

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}
```

mercury's marker sets `type Effect = MercuryEffect` where it set `type Output = Vec<MercuryEffect>`.

## Change 2: `ascend`, `AtRoot`, and `Completed`

The ascent and the proof are two things, not one. `ascend::<Root>()` climbs to the root and hands back a handle to it, `AtRoot`, which you MUTATE through `get_mut`. `complete` turns that handle into `Completed`, a sealed proof that exposes nothing — it does not `Deref` to the root, because a proof has no business handing out root access.

```rust
/// The handle `ascend` produces: the root, reachable to mutate, and completable. Awkward but
/// accurate — at the root the path and the root object are the same thing.
pub struct AtRoot<'a> {
    root: &'a mut Root,
}
impl<'a> AtRoot<'a> {
    pub fn get_mut(&mut self) -> &mut Root { self.root }
    pub fn complete(self) -> Completed { Completed(()) }   // (feature: Completed { gathered })
}

/// Proof a handler reached the root. Sealed: the field is private, so `AtRoot::complete` is the ONLY
/// constructor. A handler that must return one cannot fabricate it, so its return type forces the
/// ascent. It exposes nothing.
pub struct Completed(());

/// Climb to the root, minting the handle. In this prefactor it is `ascend_mut` wrapped in `AtRoot`;
/// the pre/post work makes the climb run the crossed posts.
pub trait Ascend<'a> {
    fn ascend<Root>(self) -> AtRoot<'a>;
}
```

`ascend` is generic over the root type the same way laserbeam's `ascend_to_mut::<Target>` already is; a handler writes `node.parent.ascend::<Mercury>()`.

`Root` is the concrete root type the tree ascends to (mercury's `Mercury`). `laserbeam` is generic over it exactly as `AscendMut`'s `Target` is; the snippet writes `Root` for the node the whole tree bottoms out at.

`ascend` and `into_parent` are the same climb: `ascend` is `into_parent` in a loop to the root. So the pre/post feature hangs its post on `into_parent`, and `ascend`/`complete` run every crossed post for free on the fire path. The prefactor puts the seam in place; nothing runs through it yet.

## Change 3: `Nested` and `into_parent`'s post seam

`into_parent` gains the two parameters a post needs — which arm is ascending, and where effects go. In this prefactor there is no post, so it projects up as on master and touches neither; they are the seam the feature fills, and the reason `into_parent` stays the single funnel both the miss-unwind and `complete` go through.

```rust
/// Whether a handler won at or below the level being ascended out of. Consulted only once posts
/// exist; defined here so the `into_parent` signature is stable across prefactor and feature.
pub enum Nested {
    Handled,   // complete drove this ascent: a handler won below
    Missed,    // the miss-unwind drove it
}
```

Before (`PathMut::into_parent`):

```rust
pub fn into_parent(self) -> P { /* project up */ }
```

after:

```rust
pub fn into_parent(self, _nested: Nested, _sink: &mut Vec<Effect>) -> P { /* project up */ }
```

`ascend` threads `Nested::Handled` and its scratch sink through the `into_parent`s it loops over; the miss-unwind threads `Nested::Missed` and the batch.

## Change 4: `Dispatch`/`Descend` thread the batch

The dispatch traits drop the `Output` from `Break` and gain `effs`. `Break(())` means a handler won (its effects are already on `effs`); `Continue(path)` a miss.

Before (`crates/bind/src/lib.rs`):

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(path: Self::Path<'a>, event: &M::Event)
        -> ControlFlow<M::Output, Self::Path<'a>>
    where Self: 'a;
}
pub trait Descend<M: Bindings>: HasParent + Sized {
    fn dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>;
}
```

after:

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(path: Self::Path<'a>, event: &M::Event, effs: &mut Vec<M::Effect>)
        -> ControlFlow<(), Self::Path<'a>>
    where Self: 'a;
}
pub trait Descend<M: Bindings>: HasParent + Sized {
    fn dispatch(self, event: &M::Event, effs: &mut Vec<M::Effect>) -> ControlFlow<(), Self::Parent>;
}
```

The generated bodies follow. In `dispatch_body` the recover threads the seam (`Nested::Missed`, `effs`) into `into_parent`; the recursion threads `effs`. Before:

```rust
let child = <Child as Dispatch<M>>::dispatch(child_path, event)?;
path = child.into_parent();
```

after:

```rust
let child = <Child as Dispatch<M>>::dispatch(child_path, event, effs)?;
path = child.into_parent(Nested::Missed, effs);
```

In `dispatch_impl` the bind checks stop collecting into `Break` and instead push the winner's effects onto `effs`, dropping the token. Before:

```rust
if is_matching(&trigger, ev) {
    return Break(collect(into_iter(#handler(ev, Node { parent: path, data: () }))));
}
// ...
Continue(path)
```

after (`own.into()` targets `Vec<M::Effect>`, so a handler may return a single effect, a `Vec`, or anything `Into<Vec<Effect>>`):

```rust
if is_matching(&trigger, ev) {
    let (own, _done) = #handler(ev, Node { parent: path, data: () });   // _done: Completed, forced and dropped
    Extend::extend(effs, Into::<Vec<M::Effect>>::into(own));
    return Break(());
}
// ...
Continue(path)
```

The `collect` this replaces accepted any `IntoIterator`; `Into<Vec<Effect>>` is the analogue that also admits a bare `Effect`. mercury adds `impl From<MercuryEffect> for Vec<MercuryEffect>`, and `Vec<Effect>` converts to itself, so both a single effect and a `Vec` are valid returns.

`descend_impl` threads `effs` and the seam (`Nested::Missed`, `effs`) into its `into_parent` on the `Continue` arm; its `Break` arm becomes `Break(()) => Break(())`.

## Change 5: handlers ascend, mutate through `get_mut`, and complete

Each exclusive handler ascends to the root, mutates through `get_mut`, and hands back its effects with the proof. Before (`crates/mercury/src/handlers/home.rs`):

```rust
pub(crate) fn to_nav<'a, E, P: AscendMut<MercuryPath<'a>>>(
    _ev: &E, node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let (nav, timer) = NavLayer::new();
    let mut effects = node.parent.ascend_mut().set_layer(nav);
    effects.push(timer);
    effects
}
```

after:

```rust
pub(crate) fn to_nav<'a, E, P: Ascend<'a>>(
    _ev: &E, node: Node<P, ()>,
) -> (Vec<MercuryEffect>, Completed) {
    let (nav, timer) = NavLayer::new();
    let mut a = node.parent.ascend::<Mercury>();     // AtRoot: the root, reachable and completable
    let mut effects = a.get_mut().set_layer(nav);
    effects.push(timer);
    (effects, a.complete())                          // mutate first, then mint the proof
}
```

Every bind handler in `crates/mercury/src/handlers/` changes the same way: the `AscendMut` bound becomes `Ascend`, `ascend_mut()` becomes `ascend::<Mercury>()`, root mutation goes through `a.get_mut()`, and the return becomes `(V, Completed)` with `V: Into<Vec<MercuryEffect>>` and `a.complete()` handed back. A handler with a single effect returns `(MercuryEffect, Completed)`; one that already builds a `Vec` (like `to_nav`) returns `(Vec<MercuryEffect>, Completed)`. mercury adds the conversion once:

```rust
impl From<MercuryEffect> for Vec<MercuryEffect> {
    fn from(e: MercuryEffect) -> Self { vec![e] }
}
```

## Change 6: `from_fn` is framework-only

Building a child path is the only way to choose an `on_post`, so it must not be reachable from a handler. `PathMut::from_fn` (and the descent it drives) becomes crate-private to `bind`/`laserbeam` — the derive macros already call it from generated code, which stays inside the crate. A handler holds a `Node`, whose `parent` path exposes `get_mut`, `ascend`, and `complete`, never `from_fn`.

## Change 7: `bind::dispatch` seeds and returns the batch

Before:

```rust
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where M: Bindings, N: Dispatch<M> + 'a {
    match <N as Dispatch<M>>::dispatch(path, event) {
        ControlFlow::Break(out) => Some(out),
        ControlFlow::Continue(_) => None,
    }
}
```

after:

```rust
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<M::Effect>>
where M: Bindings, N: Dispatch<M> + 'a {
    let mut effs = Vec::new();
    match <N as Dispatch<M>>::dispatch(path, event, &mut effs) {
        ControlFlow::Break(()) => Some(effs),
        ControlFlow::Continue(_) => None,
    }
}
```

`Some(effs)` on a win carries exactly what `Some(out)` did — the one winner's effects. `None` on a miss is unchanged. The event loop's `next`/`process_event` carry `Vec<M::Effect>` where they carried `M::Output`.

## Behavior identical

No pre/post handlers exist yet, so `into_parent` runs nothing and pushes nothing, and `complete` is `ascend_mut` under a token. Exactly one handler fires per dispatch and produces the whole `Vec`, as one handler's `collect` did. The `Break`/`Continue` split still reports win versus miss, so `bind::dispatch` returns `Some`/`None` as before. The only observable change is the return type spelling: `Vec<M::Effect>` for `M::Output`.

## Tests

The existing `crates/bind/tests/` and `crates/mercury/tests/transitions.rs` assert the same effects and the same `Some`/`None`; they pass unchanged once the return type is respelled. Add:

- `complete` forces the ascent: a handler that returns without calling `complete` does not compile (the sealed `Completed` cannot be built otherwise). This is a compile-fail test.
- the batch is seeded empty and a total miss returns `None`, not `Some(vec![])`.
