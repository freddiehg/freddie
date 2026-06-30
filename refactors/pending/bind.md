# bind: design

`bind` is the binding layer, a crate within freddie, and the next thing to build. It turns per-node `#[bind(listener, handler)]` declarations into a runtime that, given the current state, computes the active set of listeners, diffs that set as the state changes, and dispatches a fired event to the one handler the current state selects. It sits on top of rayban (the typed path) and is generic over the consumer's listener and effect types; it knows nothing about keyboards, macOS, or how effects are performed. mercury is the first consumer; see `freddie-keys-plan.md` for that narrative and `freddie-dispatch-precedence.md` for the precedence design space.

## Crate layout

- `bind` — the runtime: accumulation, diffing, the dispatch map, and the traits the consumer implements.
- `bind_macro` — the `#[bind]` derive, re-exported by `bind`, mirroring `rayban`/`rayban_macro` (a derive must live in a proc-macro crate).

## The attribute

`#[bind(listener, handler)]` on a node (a `Rayban`-derived struct or enum), repeatable:

```rust
#[bind(Keyboard::new('g'), on_g)]
#[bind(Keyboard::new("cmd+g"), on_cmd_g)]
struct Inner {}
```

- `listener`: an expression evaluating to a value that lifts into the consumer's `Listener` enum via `Into` (`derive_more::From`). The derive wraps it with `.into()` when building the node's binding set, so `Keyboard::new('g')` and a `Foreground::new("Chrome")` both land as `Listener`.
- `handler`: a function (signature below).
- Several `#[bind]` on one node accumulate into that node's binding set.

## The Listener type (consumer-provided)

One enum, one variant per source, `#[derive(derive_more::From)]`:

```rust
enum Listener { Keyboard(Keyboard), Foreground(Foreground), .. }
```

`bind` is generic over it with `L: Hash + Eq + Clone`. `bind` never constructs or matches `Listener` variants itself; it treats `L` as an opaque hashable key. Matching variants to actually register and deregister is the consumer's registrar (below).

## What the derive emits

Per node, the derive emits that node's bindings: its `(listener, handler)` pairs, where each handler is callable later with the event and a rayban path to this node. The exact trait shape is open (see open questions), but the contract is that a node yields its own bindings and nothing about its position in the tree; position comes from rayban at accumulation time.

## Accumulation

The active binding set for a state is computed at runtime; it cannot be known statically, because which node is the active leaf depends on the state:

1. rayban `resolve` to the active leaf.
2. Walk rootward via `into_parent`, collecting each node's bindings.
3. Coalesce into one map keyed by `L`.

Output: the active binding map `HashMap<L, Thunk>`, whose key set is the active set `HashSet<L>` used for registration.

A node can have several concurrently-active children (multiple `#[resolve_into]`, required; see `rayban-missing-features.md`), so `resolve` is multi-valued: there are several active leaves, and accumulation unions bindings across all of them. The single-active-leaf wording here generalizes to a set of active leaves.

v1 is static and non-clobberable: a listener bound at two levels of the active path is a collision and an error raised here, during accumulation, naming both bindings. The fuller space (leafward-clobbers-rootward precedence, clobberable bindings, dynamic fall-through) is in `freddie-dispatch-precedence.md` and is not in v1.

If `resolve` stops at an interior node (the state-controlled-children case, where nothing is focused), accumulation collects that node's own bindings as the leaf set; see `rayban-state-controlled-children.md`.

## Dispatch and the erasure boundary

Each handler wants the typed `Path<Node, Parent>` for its node, but the accumulated map is keyed dynamically by `L`, so there is an erasure boundary between the map and the typed handler. The derive resolves it: per binding it emits a type-erased thunk that, given `&mut Root` and the event, reconstructs the typed path to its node and calls the typed handler. The map stores `L -> Thunk`.

On a fired `(listener, event)`:

1. Look up the thunk for `listener` in the active map.
2. Run it: it reconstructs the typed path from the current root and calls `handler(event, path)`.
3. Return the handler's result (the consumer's effect collection) to the caller.

The thunk is `Fn(&mut Root, Event) -> HandlerOutput`. `bind` is generic over `Event` and `HandlerOutput` (e.g. `Option<Vec<Effect>>` for mercury); it passes them through and does not interpret them. State does not change between accumulation and dispatch within one loop iteration, so reconstructing the path against the current root is sound; the reconstruction is the same descent rayban already walks, reused rather than reimplemented.

## The handler

A handler is `fn(Event, Path<Node, Parent>) -> HandlerOutput`. It mutates state through the path (`get_mut`, `into_parent`, `get_root`) and returns effect data; it performs no I/O. `bind` calls it; the consumer's shell performs the returned effects. `get_root` (walk to the root to switch the active variant) is needed here; whether it lives in rayban or bind is open.

## Diffing and registration

`bind` computes the active listener set per state. Across a state change it diffs old vs new (set subtraction, since `L: Hash + Eq`) into `to_register` and `to_deregister`. `bind` does not touch the OS; it produces the delta and the consumer routes each listener by variant to its backend through a registrar:

```rust
trait Registrar<L> {
    fn register(&mut self, listeners: &HashSet<L>);
    fn deregister(&mut self, listeners: &HashSet<L>);
}
```

The registrar owns all registration state (taps, watchers, per-listener OS handles in a map keyed by `L`); listener values stay pure data and hold no references to it. Whether `bind` calls the registrar itself or just returns the delta to the loop is open.

## What bind owns vs what the consumer brings

- bind owns: the `#[bind]` derive, the accumulation algorithm, the precedence and clobber policy, the diff, the dispatch map, and the path-reconstruction erasure.
- The consumer brings: the `Rayban`-derived state types, the `Listener` enum, the handlers, the `Event` and effect types, the `Registrar` impl, and the event stream that yields `(Listener, Event)`.

## Dependencies

- rayban: `resolve`, `into_parent`, and `get_root`. The thunk's path reconstruction is rayban's descent, reused.

## Open questions (concrete, for the crate)

1. The bindings-trait shape and how the derive threads path reconstruction into each thunk. A per-node bind derive sees only its own node, not the descent from the root; identifying that node among the active path's ancestors at dispatch is the hard part, shared with rayban's resolve.
2. Genericity: parameterize `bind` over `(L, Event, HandlerOutput)` via one trait the consumer implements, vs free type params on each entry point.
3. `Registrar`: does `bind` call it, or return the delta for the loop to apply? Leaning toward returning the delta, to keep `bind` free of the registrar's lifetime.
4. `get_root`: lives in rayban (a generalization of `into_parent`) or in `bind`. Leaning rayban, since it is pure path machinery.
5. Static vs dynamic dispatch: v1 is the static winner, non-clobberable; the chain model comes later.
6. Erased thunk type: boxed `dyn Fn` vs a generated enum of concrete thunks. Boxed is simplest; an enum avoids the allocation but needs the derive to enumerate every binding.
7. State-controlled children: when `resolve` stops at an interior node, accumulation and the thunk must target that node as the leaf.
