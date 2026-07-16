# laserbeam: state-controlled children

> Implemented, via derived children (`derived-levels-plan.md`): a `#[derived_child]` fn computes the active child from state and returns `None` when none is focused, which is the state-selected, fallible descent this doc argues for. The mechanism sketched below predates two refactors and does NOT match how it shipped: `Resolved`/`Resolve::resolve()` were removed as dead weight (`resolved-is-dead-weight.md`), so there is no `Resolved` enum to gain a node-as-leaf variant, and dispatch is `ControlFlow`-based, descending one level at a time and running each node's own binds on a miss rather than resolving to a single leaf. And "two ways today" (below) is three now: enum variant, `#[resolve_into]` field, and derived child. Kept as the design thinking that led there.

## The realization

"What is focused" is state, not always an enum variant. laserbeam picks the active child two ways today: an enum's active variant, or a struct's single `#[resolve_into]` field. Both are static in shape, with the type tag as the discriminator. The general case is that the discriminator is ordinary state: a browser's focused element is an id held in state, not a distinct type per element. The design already names this case ("a state tag/index over stored children" picks the active leaf); this makes it real. The enum-variant descent is the special case where the selector happens to be the type discriminant.

## The runtime already supports it

`Path` carries `Proj`, which is already `Bare(fn(..))` or `Dyn(Box<dyn Fn(..)>)`. The `Dyn` arm is a closure that can capture, e.g. an index, and `Path::from_box` constructs it. So a projection that indexes a collection, capturing the index, is an existing, hand-writable `Proj::Dyn`:

```rust
Path::from_box(parent, Box::new(move |w: &mut Workspace| &mut w.panes[idx]))
```

laserbeam-plan.md's "Deferred (post-v1)" already records this: the runtime takes the indexed box, only macro support is missing. So this feature is a derive change plus a semantics decision, not a runtime change. And because the child type is uniform, the path type is unchanged: `PanePath<'a> = Path<Pane, WorkspacePath<'a>>`, with the index living in the box, not the type.

## Scope: homogeneous, state-selected

A node holds a homogeneous collection of children and selects one by state:

- `Vec<Child>` plus an active index,
- `HashMap<K, Child>` plus an active key,
- `Option<Child>` (the degenerate single-slot case).

Heterogeneous children stay the enum's job; you cannot store distinct types in a uniform collection. Heterogeneous-and-dynamic composes by nesting: `Vec<ChildEnum>` lets the collection pick the index and the enum pick the variant. So this feature is specifically one child type, selected at runtime.

## The hook: `#[custom_resolve_into_fn(...)]`

Instead of the macro generating the descent, the user supplies the selection. Two granularities:

- Selector: `fn(&Node) -> Option<Index>` (or `Option<K>`). Returns which child is active, or none. The macro builds the `Proj::Dyn` from the collection field and the returned index/key, so it must know the field and the child type.
- Full custom projection: `fn(&mut Node) -> Option<&mut Child>`. The user does selection and projection both. Most general, for computed or filtered access. The macro still needs the child type named to call `<Child as Resolve>::resolve`, so the attribute carries it: `#[custom_resolve_into_fn(pick, child = Pane)]`.

The `Option` in the return is load-bearing: see fallible resolve.

## Resolve becomes fallible here

Today `resolve` always lands on exactly one leaf. The selector can return `None` (empty collection, nothing focused, stale index). Then this node is itself the active leaf and resolve stops. That is the browser semantics: a container with nothing focused is the focus.

Consequences:

- A node can be interior and a leaf at once. `Resolved` gains a variant for this node-as-leaf alongside its descend paths. The generated descent has two arms: `Some` builds the capturing `Path::from_box` and recurses into `<Child as Resolve>::resolve`; `None` returns `Resolved::<Self>(path)`.
- None-focused is a normal outcome, not an error; the node handles events itself.

This generalizes the `Option`/`Result` deferred note in laserbeam-plan.md: an empty `#[resolve_into]` and a none-focused collection are the same shape, a conditional edge with `Resolved` gaining a terminal variant for the empty case.

## Projection is snapshotted, not re-read

`get_mut` re-runs the projection on every call. If it re-read `node.focused` each time, a handler that mutates `focused` would retarget the cursor mid-call. So the index/key is captured at resolve time inside the `Proj::Dyn` box, pinning the cursor to the child it resolved to. This matches the existing staleness model: changing which child is active does not silently move an outstanding cursor; structural change means re-resolve, and `into_parent` consuming prevents reuse of a stale child cursor.

Caveat: a captured `Vec` index is invalidated by removal or reorder. If a cursor must outlive a structural edit to the collection, prefer stable keys (`HashMap`, generational ids) over positional indices. Under the consume-on-ascend discipline a cursor normally does not outlive the edit, which is the same contract as reassigning an enum variant.

## Macro work required

- Stop passing `Vec<Child>` / `HashMap<K, Child>` / `Option<Child>` through as the child type (the current silent gap, where only `Box` is unwrapped). Recognize the collection or option and unwrap to `Child`.
- Parse `#[custom_resolve_into_fn(fn, child = Child)]` and allow it as a third descent kind alongside an enum's variant and a struct's `#[resolve_into]`. It is still one active child, selected at runtime, not a second simultaneous one, so the "at most one descent" rule holds.
- Generate the fallible descent: call the selector, build the capturing `Proj::Dyn` on `Some`, emit node-as-leaf on `None`.
- Add this node's path to `Resolved` as a leaf variant. Leaf construction already exists (`Resolved::<Self>(path)`); reuse it for the `None` arm.

## Interaction with freddie (bindings)

Walk-up (`into_parent`) is unchanged. Walk-down (`resolve`) may stop at the interior node when nothing is focused, so accumulation collects that node's own bindings as the leaf set. Siblings share bindings by construction (same type), so the type-level double-bind detection across siblings is gone; that is inherent to uniform children and fine. This closes the keys-plan open question about a layer holding a `Vec` of sub-states.

## Open questions

- Selector signature: `&Node -> Option<Index>` (macro builds the projection, must know the field and indexing) vs `&mut Node -> Option<&mut Child>` (user owns the projection, macro needs only the child type). Likely both: the latter as the general escape hatch, the former as sugar for the index/key case.
- `Resolved` when a node is both interior and leaf: one variant for the node-as-leaf; confirm no collision with the descend arm's leaves.
- Index vs key stability under structural mutation, and whether to bless generational indices.
- Whether `#[custom_resolve_into_fn]` should ever return a heterogeneous sum directly, or always nest an enum (`Vec<ChildEnum>`). Leaning: always nest, keep this feature homogeneous.
