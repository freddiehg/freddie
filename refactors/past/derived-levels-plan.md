# derived levels: the plan

The coordinating doc. What ships in what order.

STEPS 1 THROUGH 8 ARE SHIPPED. The design (`refactors/past/resolution.md`) and the
`accumulate` change (`refactors/past/accumulate-takes-a-path.md`) are done and tested;
mercury no longer stores the foregrounded app twice. What follows in "Downstream" is not
started. Every design claim there was compiled in a scratchpad; none of it is in the repo.

## The goal

Mercury stores the foregrounded app twice. `root.foregrounded` holds it, and the in-app layer holds it AGAIN as an enum variant whose whole content is the discriminant. `on_foregrounded` re-derives the duplicate by hand on every foreground event.

Delete the duplicate. A level that is not in the tree, built from root state on every dispatch, carrying the data its handlers need. `resolution.md`.

## Order

The design is last. Everything before it is additive or a pure refactor, lands on master on its own, and leaves the tree working.

```
1  Node<Parent, Data>, data: () everywhere    SHIPPED  e0cecdf
2  HasParent                                  SHIPPED  993e12b
3  Descend                                    SHIPPED  993e12b
4  accumulate takes a path                    SHIPPED  ba0450e
5  the check does not ship                    SHIPPED  ba0450e
6  #[derived_node(parent = ..)]               SHIPPED  22cdc34
7  #[derived_child(f)]                        SHIPPED  22cdc34
8  mercury uses it                            SHIPPED  45850e9
```

Each landed alone, in order, with the full suite green at every step.

## 1. `Node<Parent, Data>`, with `data: ()` everywhere

This is the whole migration, and it carries no design.

```rust
pub struct Node<Parent, Data> {
    pub parent: Parent,
    pub data: Data,
}
```

Every handler takes `Node<OwnPath, ()>` instead of a bare path. The derive hands it one:

```rust
return ControlFlow::Break(on_escape(ev, ::bind::Node { parent: path, data: () }));
```

`path.get_mut()` becomes `node.parent.get_mut()`. `()` is zero-sized, so nothing costs anything.

A handler bound at several places keeps its `Ascend` bound, which moves to `parent`:

```rust
fn to_home<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Out {
    go_home(&mut node.parent.ascend());
    vec![]
}
```

Nothing in laserbeam changes. Compiled, including the real `#[derive(Bind)]` emitting it. In `bind`'s own harness the migration was 14 lines across 8 handlers; mercury has around 25.

Ships alone. After it, `data` exists and is always `()`, and adding a level that puts something in it is a local change.

## 2. `HasParent`

```rust
pub trait HasParent {
    type Parent;
    fn into_parent(self) -> Self::Parent;
}

impl<Parent, Data> HasParent for Node<Parent, Data> { /* Parent = Parent */ }
impl<N, P> HasParent for ::laserbeam::Path<N, P> { /* Parent = P */ }
```

Two impls, no consumers yet. It exists so a generated impl can reach the parent's type without naming it.

Ships alone.

## 3. `Descend`

```rust
pub trait Descend<M: Bindings>: HasParent + Sized {
    fn dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>;
}
```

One descent, whatever the child is. The derive emits a `Descend` impl per PLACE node, delegating to its own `Dispatch` and then `into_parent()`.

It must be per-node and cannot be a blanket `impl<N, P> Descend<M> for Path<N, P>`: `Dispatch` carries `Self: 'a`, and the HRTB needed to state the blanket is E0311.

Nothing calls it yet. `#[resolve_into]` keeps descending the way it does today. Ships alone, as unused generated code.

## 4. `accumulate` takes a path

`accumulate(&self)` has no path, so it cannot run a derived child fn, so a derived level's binds would never reach the trigger set.

```rust
fn accumulate<'a>(
    path: Self::Path<'a>,
    out: &mut HashSet<M::Trigger>,
) -> Result<Self::Path<'a>, BindError>;
```

`bind_macro`'s `accumulate_body` then descends through the same `derive_support::Edge` that `dispatch_body` uses, so the two walks cannot drift.

Behaviour-preserving. Every derived impl builds unchanged; only the four callers of `bind::accumulate` move. `accumulate-takes-a-path.md`.

## 5. The check does not ship

`accumulate` is the clobber check and nothing else. It has no callers outside `bind`'s tests, and its stated purpose (the trigger set an app registers with the OS) does not exist: `CGEventTap` subscribes to event TYPES, not keys.

Behind a `check` feature, on by default, with mercury's binary and `freddie_keys` taking `bind` with `default-features = false`.

Verified by expansion:

```
cargo expand -p mercury --lib          0 EventHandler impls, 10 Dispatch
cargo expand -p mercury --lib --tests  10 EventHandler impls
```

Same doc as step 4. Could be folded into it.

## 6. `#[derived_node(parent = ..)]`

On a struct with no place in the tree. It emits `impl Descend<M> for Node<ParentPath<'a>, Self>` carrying that struct's `#[bind]`s.

Nothing produces such a node yet, so nothing calls it. Ships alone.

## 7. `#[derived_child(f)]`

The design lights up. On a node whose child is not a field:

```rust
let path = match chrome(&path) {
    Some(data) => Descend::<M>::dispatch(Node { parent: path, data }, event)?,
    None => path,
};
```

`f` is `fn(&Parent) -> Option<Data>`. A shared reference, so it cannot mutate, cannot lose the parent, and cannot materialize into the tree.

When `Data` is an enum, the derive destructures per variant. There is no separate mechanism for several possible children.

## 8. Mercury uses it

`root.foregrounded` becomes the only copy of the foregrounded app. `InAppLayer` gets `#[derived_child]`. The existing binds (`refresh`, `previous_window`, `next_window`, `window_1..0`) move onto the derived payloads.

Deletes `AppLayer`, `OtherApp {}`, and the resync in `on_foregrounded`. An app with no bindings gets no struct: the derived child fn returns `None` for it.

Does NOT delete `ChromeApp {}` and `GhosttyApp {}`. They become `ChromeInfo` and `GhosttyInfo` and stay unit structs, because mercury tracks nothing per app yet. They cannot be `()`, because the derive needs distinct `Data` types to select a handler: two levels with the same `Data` would have conflicting `Descend` impls.

They stop being units when mercury tracks something per app (a tab name, a pane index). Punted.

## Downstream: where each piece landed

```
9   Resolved is dead weight        DONE. refactors/past/resolved-is-dead-weight.md.
10  Option<Child> on resolve_into   PENDING. refactors/pending/option-resolve-into.md.
11  Fix B (Path becomes a Node)     DO NOT DO. refactors/past/ascend-through-derived.md.
12  multiple parents / hosts        DO NOT DO (needs Fix B). refactors/past/derived-level-multiple-parents.md.
13  several children (IntoIterator) PENDING, recorded. refactors/pending/derived-child-iterator.md.
    derived-child persistence       DO NOT DO, rejected. refactors/past/derived-child-persistence.md.
```

The eight shipped steps are the design and its adoption; that work is done, so this doc
moves to `past`. The two items still open (Option on a place field, the iterator
generalisation) keep their own docs in `pending`.
