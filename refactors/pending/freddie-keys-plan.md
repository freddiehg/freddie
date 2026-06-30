# freddie: event bindings and the bind derive

## Goal

When freddie runs, the user defines the application state as a data structure, and freddie turns events into mutations of that state. The state is an enum of layers (a nav layer, a typing layer, and so on), each carrying whatever state it needs. freddie listens to events, and for each event dispatches to exactly one handler chosen by the current state.

## Names

- freddie is the framework: the event loop, effects-as-data, and the glue over rayban and bind.
- rayban is a library within freddie: the typed mutable path (resolve, `into_parent`, `get_root`).
- bind is a library within freddie: the binding layer (the `#[bind]` derive, accumulation, diff, dispatch). Detailed in `bind.md`.
- mercury is the concrete use of freddie, the keyboard-remapping application built on it. Its root state struct is `Mercury`.
- `MercuryEvent` and `MercuryEffect` are mercury's concrete event and effect types. freddie is generic over the event and effect types; mercury fixes them to these.

## The state tree

- The layers are an enum, e.g. `enum Layers { Nav(Nav), Typing(Typing), .. }`.
- Each layer is a struct or enum with its own state and possibly nested sub-states.
- At any moment there is exactly one active path from the root down to the active leaf. That is the rayban resolve target.

### The root holds global state, not just the layers

The root is probably not the layers enum directly. There is likely an outer layer of global state the app needs regardless of the active layer: a handle to Hammerspoon (or whatever drives the side effects), and possibly a vector tracking the async work we have kicked off. The root is then something like `struct Mercury { hammerspoon: Handle, in_flight: Vec<_>, layers: Layers }`, with the layers enum as a field. Whether the async-tracking vector is actually needed is open.

Putting global state at the root means `get_root` lands on it, so a handler can both switch the active layer (replace `root.layers`) and touch global state (the Hammerspoon handle, the in-flight list) in the same step.

## Bindings: the bind derive

A node declares bindings with an attribute:

```rust
#[bind(Keyboard::new('g'), on_g)]
struct Inner {}
```

The two arguments are a trigger and a handler:

- `Keyboard::new('g')` is the trigger: which event to listen for. The source (`Keyboard`) and the specific key (`'g'`) are folded into one value, not two arguments. `Keyboard::new('g')` is a keyboard trigger for `g`, and a different source is a different constructor (`Foreground::new("Chrome")`). The trigger is pure data, `Hash + Eq`, so triggers can be set and map keys, and the value holds no reference to any registration state. Modifiers are not chords baked into the trigger; they are modeled as state (see `modifier-keys.md`).
- `on_g` is the handler to run when it fires.

The derive emits, per node, a map of trigger to handler. A node can carry several bindings across several sources.

The attribute is `bind`, not `key`: a node binds a trigger to a handler, and the mechanism is not keyboard-specific. Keyboard is one source; foregrounding an application is another; the same accumulation and dispatch serves both and anything else we route this way.

### One trigger enum, one outer handler

Triggers are unified into a single enum, one variant per source, with `From` derived per variant (`#[derive(derive_more::From)]`) so `Keyboard::new('g')` lifts in with `.into()`:

```rust
enum Trigger { Keyboard(Keyboard), Foreground(Foreground), .. }
```

There is conceptually one event source and one sink. The accumulated set of active bindings is a `HashSet<Trigger>`. A single outer handler owns registration: it receives the trigger diff (the set to register, the set to deregister) and does whatever each requires, matching the variant to its OS mechanism (a keyboard tap for `Trigger::Keyboard`, a workspace observer for `Trigger::Foreground`). We represent the triggers as one enum, and the outer handler breaks them apart again by variant.

This is why no trigger value holds a reference to registration state. The taps, the watchers, and any per-trigger OS handle (a hotkey id needed to unregister, say) live on the one outer handler; the trigger values stay pure data. `Keyboard::new('g')` and `Keyboard::new('y')` are just two keyboard triggers the outer handler registers through the same tap; they share nothing themselves, and any per-trigger handle sits in a map on the outer handler keyed by trigger. That keeps triggers hashable and testable and avoids reintroducing the shared mutable state rayban exists to avoid.

Events arrive as one merged stream of fired triggers; dispatch matches the fired trigger to its handler. Listening is async so the loop can `select!` over the stream plus a shutdown signal and compose under tokio.

A source plugs in by adding a `Trigger` variant and the match arm that registers, deregisters, and listens for it; `#[bind(MyEvent::new(..), on_it)]` then works like the rest. Adding a source edits the central `Trigger` enum, which is acceptable; `derive_more::From` covers the lifting boilerplate, so no macro-generated enum is needed.

## Accumulating the active binding set (a runtime computation via rayban)

We cannot know the active bindings statically. The nav layer might care only about `a` and `b`; the typing layer cares about `a` through `z`. If we tried to derive the full `a..z` set onto a layer struct statically, that does not work, because which bindings are live depends on the current state. So it is a runtime thing.

Given the current state:

1. Resolve to the active leaf with rayban.
2. Walk upward via `into_parent`, and at each node collect that node's bindings into a single accumulated map.

This coalesces the bindings from the active leaf up through its layer and any globals into one map for the current state.

### Precedence, clobbering, and dispatch model

v1 is the simplest point in the design space: static dispatch and non-clobberable bindings. A trigger is bound at one place on the active path; a trigger bound at two levels is a collision and an error caught during accumulation. There is no overriding and no fall-through.

The fuller design space (leafward-clobbers-rootward precedence, clobberable vs non-clobberable bindings, static winner vs dynamic fall-through, and a richer handled signal) is in `freddie-dispatch-precedence.md`. None of it is in v1.

## The event loop

One loop listens to all event sources we choose to route this way: keyboard, foregrounding, and whatever else. Not everything has to go through this loop, but routing everything through it is the default.

Each iteration:

1. From the current state, accumulate the active binding set (resolve, walk up, coalesce, applying the precedence and clobber rules).
2. Register and subscribe to exactly that set of events, and listen.
3. When an event fires, use rayban to find the single handler for it and execute it.

Registering only the accumulated set means we listen for exactly what the current state cares about, and unregistered events behave however the source's default is (for example, keys we do not bind pass through and type normally).

Dispatch is synchronous. An event arrives, we resolve and run its handler to completion, the handler mutates the state in memory and returns its effects, the shell performs those effects, and only then do we take the next event. The next iteration re-accumulates the binding set from the new state, so a layer switch immediately changes what we listen for.

This synchrony is deliberate, and it constrains the data model rather than the handlers. Dispatch sits in the path of every keystroke and must be fast; a handler that ran slow work to completion would block the next key from being typed. So slow or async work does not run inside dispatch. The handler starts it and returns immediately, the spawned future lands in the root's in-flight vector (the `in_flight: Vec<_>` above), and it makes progress in the background while the loop goes back to taking events. Pending work is therefore represented as state, so a later event can observe or cancel it; dispatch stays quick and async lives in the data, not in the dispatch path.

A future cannot hold a rayban path across its await, because the mutable borrow cannot outlive a dispatch. So the future carries plain data or a key, does its I/O off the dispatch thread, and when it resolves it re-enters as a completion event: the loop re-resolves from the current root and applies the result through the same synchronous path. See the serializability open question below.

### Registration and diffing

Between iterations we do not tear everything down and rebuild; we diff the accumulated `HashSet<Trigger>`. If state A's set is `{a, b, c}` and state B's is `{a, b, d}`, the diff says deregister `c` and register `d`, and freddie hands only that delta to the outer handler, which routes each trigger by variant to its source's register and deregister. Because triggers are `Hash + Eq`, the set is a `HashSet` and the diff is set subtraction. Computing the delta is freddie's job, sitting between the accumulator and the outer handler.

## Handler dispatch

When an event is handled, rayban finds the first and only handler for it, and runs it. The handler receives:

- the event, and
- a rayban path to the node the binding is on.

Through that path the handler can mutate the node it is on (`get_mut`) and traverse upward to its ancestors (`into_parent`), for example to switch layers or update parent state.

Example: with `#[bind(Keyboard::new("esc"), to_nav)]` on `Inner`, the function `to_nav` receives both the event and a path to the `Inner` object. It can mutate `Inner`, but here it walks up to the root to switch layers.

```rust
fn to_nav(event: MercuryEvent, path: Path<Inner, _>) -> Option<Vec<MercuryEffect>> {
    // escape returns to the nav layer from anywhere, so reach the root and swap the active layer
    path.get_root().layers = Layers::Nav(Nav::default());
    // and flash an overlay so the switch is visible
    Some(vec![MercuryEffect::ShowOverlay])
}
```

Nothing here touches `Inner`'s own state, but the path makes that available too (`get_mut`) for handlers that do. The `Some` marks the event handled under the dynamic model; the static version drops the `Option` and just returns the `Vec`.

## What handlers do

A handler is a pure function of the event and the path to its node. It produces two things, neither of which performs real I/O:

1. State mutations, applied in memory through the path. It mutates the node it is on (`get_mut`) and can walk up (`into_parent`, `get_root`) to change ancestors or switch layers. The two normal ones are modifying the current node's state, and navigating to the root to change layers (escape, for instance, almost always returns to the nav layer).
2. A description of side effects to perform, returned as data. The handler does not type the letter or call Hammerspoon itself; it returns an effect value and the runtime shell performs it.

This is functional-core, imperative-shell. The handler is the testable core: given a state and an event, you assert the resulting state and the returned effects, with no real keyboard or Hammerspoon involved. The loop is the shell that actually performs the effects.

### Effects as data

Side effects are a single enum. freddie is generic over the effect type; mercury's concrete one is `MercuryEffect`, e.g. `enum MercuryEffect { EmitKey(Key), ShowOverlay, Foreground(App), Arbitrary(..), .. }`. A handler returns a collection of them (order may matter, so probably a `Vec`).

The effect type is independent of the trigger. A key-triggered handler is not restricted to keyboard-shaped effects; a handler reached via a `Keyboard` trigger may foreground an app, show an overlay, or run something arbitrary. So there is one effect enum shared by every handler, not a per-binding or per-source effect type, and `Arbitrary` is the escape hatch for the long tail.

`MercuryEffect` is hand-written, with one variant per effect type, and `From` derived per variant (`#[derive(derive_more::From)]`) so a handler lifts individual effects in with `.into()` rather than naming the variant. The "sink" is purely shell-side: the shell `match`es each variant to whatever performs it. Because every handler returns the same `MercuryEffect`, effects travel back to the loop as a uniform `Vec<MercuryEffect>` and the shell performs them; nothing about the effect type has to cross the dispatch erasure boundary.

- Remapping A to B: the handler returns `EmitKey(b).into()`. Emitting a key is just another effect, not a special case.
- A to B and also an overlay: the handler returns both `EmitKey(b)` and `ShowOverlay`, two variants of the same `MercuryEffect`, in one `Vec`.

Because the handler only mutates in memory and returns effect data, each handler is trivially testable in isolation. Deriving `PartialEq` on `MercuryEffect` is what lets a test `assert_eq!` on the returned vec.

Under the dynamic fall-through model the return also carries whether the event was handled, so the type is likely `Option<Vec<MercuryEffect>>`: `None` falls through to the rootward handler, `Some(effects)` means handled, perform these. The static model just returns the effects.

### Navigating to the root

For the layer switch above, a handler needs to reach the root from wherever it was bound, not just one level up. `into_parent` moves up one level; on top of that we want a way to walk all the way to the root so a handler can switch layers regardless of its depth.

### get_root

There is a trait, call it `GetRoot`, with a single function `get_root`, implemented on all the different path types. `get_root` walks the parent chain to the root and yields the root, so the handler can change the active layer there. Changing layers is a mutation of the root enum's active variant: `get_root` gives a mutable handle at the root, and the handler replaces the variant (for example, set the root to the nav layer).

Because each path's parent chain terminates at `&mut Root`, `get_root` is a fixed walk up the chain, the same shape rayban already uses for `into_parent`. The rayban derive knows each node's parent, so it can generate the `GetRoot` impl per path type the same way it generates `Resolve`. Whether this lives in rayban or freddie is an open question below.

## Open questions

1. Handler signature at the dispatch boundary. The handler conceptually wants the typed `Path<Node, Parent>`, but the loop dispatches dynamically over a heterogeneous map, so there is an erasure boundary between "accumulated map of handlers" and "typed path handed to the handler." Design how the typed path is reconstructed for the chosen handler.
2. Trigger identity across sources (resolved by the `Trigger` enum). A keyboard trigger wraps a key; a foreground trigger wraps an application id; both are variants of the single `Trigger` enum, so the accumulated set is keyed by one type and the outer handler routes by variant rather than the derive tracking a separate key per source and sub-key.
3. Non-clobberable enforcement timing. Enforce during accumulation (so the error can name both the protected binding and the offending override) rather than at registration.
4. What "register" means per source: OS key grabs versus a polled or subscribed foreground watcher, and the teardown when the active set changes between iterations.
5. Single winner vs handler chain. Under the static model, accumulation yields one winner per trigger; under the dynamic fall-through model, it yields an ordered chain that dispatch walks. Decide which, and for the static model, what happens if two equally-specific bindings collide.
6. Relationship to the missing rayban features. Layers as an enum and upward traversal are supported today; if a layer ever holds a list of sub-states (a `Vec` of children) selected by state, that is the state-controlled-children feature scoped in `rayban-state-controlled-children.md`.
7. Where `GetRoot` lives. It is a generalization of `into_parent` to "walk all the way up," so rayban is the natural home, and the derive can emit it from the parent chain alongside `Resolve`. But layer-switching is freddie semantics. Decide whether rayban exposes the generic `get_root` and freddie builds layer-switching on it, or freddie owns the whole trait.
8. What `get_root` returns. A `&mut Root` is enough to replace the active layer (`*root = App::Nav(..)`), but a typed path at the root would let the handler re-descend into the new layer in the same step. Decide between the bare `&mut Root` and a root path, given that after a layer switch the next loop iteration re-resolves anyway.
9. Effect feedback. The shell performs returned effects. Does `EmitKey(b)` re-enter freddie's own loop, so `b`'s binding can fire, or go straight to the OS as a synthesized keystroke? Decide whether effects cascade, and if they can, how to bound the cascade and avoid loops.
10. Effect ordering and atomicity. Effects are a `Vec`, so confirm they run in order. The state mutation lands in memory before any effect runs; decide whether that matters for effects that should reflect the new state (an overlay showing the post-mutation layer, say).
11. Serializability vs latency. Sync dispatch keeps typing snappy, but background completions mean a handler's state effect can land after later keystrokes, so mutations are not applied in event-arrival order. React tolerates this (concurrent rendering can show torn, inconsistent intermediate state), which we treat as a defect, but jittery typing is worse. The recommended resolution keeps one serial applier: every mutation, a sync handler or an async completion, goes through the single dispatch loop on one thread, so there are never two mutations at once and no tearing, and only real-time ordering of completions is given up, which is inherent to backgrounding the work. Separate priority lanes do not help, because any handler can mutate any state through its path, so changes cannot be classified high- vs low-priority by whether they mutate. Decide whether the single-serial-applier model suffices, or some completions need ordering against later events.
12. Incremental reaccumulation (perf). rayban could record the rootmost node mutably accessed during a dispatch. Since nothing above that node changed, reaccumulation only needs to redo the path at and below it and reuse the rest of the accumulated set. Not v1; a perf optimization to keep in mind.
