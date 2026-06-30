# freddie: event bindings and the event_source derive

## Goal

When freddie runs, the user defines the application state as a data structure, and freddie turns events into mutations of that state. The state is an enum of layers (a nav layer, a typing layer, and so on), each carrying whatever state it needs. freddie listens to events, and for each event dispatches to exactly one handler chosen by the current state.

## The state tree

- The layers are an enum, e.g. `enum Layers { Nav(Nav), Typing(Typing), .. }`.
- Each layer is a struct or enum with its own state and possibly nested sub-states.
- At any moment there is exactly one active path from the root down to the active leaf. That is the rayban resolve target.

### The root holds global state, not just the layers

The root is probably not the layers enum directly. There is likely an outer layer of global state the app needs regardless of the active layer: a handle to Hammerspoon (or whatever drives the side effects), and possibly a vector tracking the async work we have kicked off. The root is then something like `struct Freddie { hammerspoon: Handle, in_flight: Vec<_>, layers: Layers }`, with the layers enum as a field. Whether the async-tracking vector is actually needed is open.

Putting global state at the root means `get_root` lands on it, so a handler can both switch the active layer (replace `root.layers`) and touch global state (the Hammerspoon handle, the in-flight list) in the same step.

## Bindings: the event_source derive, generic over the source

A node declares bindings with an attribute. The running example:

```rust
#[event_source(source = Keyboard, trigger = 'g', handler = open_chrome)]
struct Inner {}
```

The arguments are named:

- `source = Keyboard` is the event source. The mechanism is generic over a type parameter `S`; the keyboard source is one instantiation, the foreground source another (`#[event_source(source = Foreground, trigger = "Chrome", handler = on_chrome)]`), and we can add more.
- `trigger = 'g'` identifies the specific event within that source. Its type is the source's associated trigger type (below): `'g'` is a `<Keyboard as EventSource>::Trigger`, `"Chrome"` a `<Foreground as EventSource>::Trigger`. The trigger slot is an arbitrary expression of that type, not just a literal: the derive drops it into the generated bindings-map insertion, so it is evaluated when the map is built. A block that constructs the value (`{ let mut m = HashMap::new(); m.insert(..); m }`) is legal if that is the trigger type, and nothing needs a `'static` bound, because the value is owned and moved into the map rather than borrowed.
- `handler = open_chrome` is the handler to run.

The derive emits, per node, a map of bindings: event type, then trigger, then handler. A node can carry several bindings across several event types.

What makes `TEvent` usable is a trait it implements, `EventSource`, and that impl is the whole seam. `EventSource` has an associated type, the trigger type, plus the methods that register and deregister triggers with the underlying source. The macro's trigger argument is a value of that associated type. The trigger is the key the accumulated set and map are built from, so the associated type is bounded `Hash + Eq`: a keycode for `Keyboard`, an application identifier for `Foreground`, anything usable in a `HashSet` or `HashMap`. The derive, the accumulator, and the event loop are all generic over `TEvent: EventSource`.

So any custom event type drives the same machinery, and the trigger is whatever its associated type is: `#[key(MyEvent, MyEventTrigger::Foo, handler)]` is plausible, where `<MyEvent as EventSource>::Trigger = MyEventTrigger` and `Foo` is one of its variants. The same goes for `Keyboard`, whose trigger covers chords: `#[key(Keyboard, 'cmd+g', on_cmd_g)]`.

A sketch of the trait:

```rust
trait EventSource: Sized {
    // identifies one binding within this source: a keycode or chord for Keyboard,
    // an application id for Foreground. Hash + Eq so accumulated bindings live in a HashSet/HashMap.
    type Trigger: Hash + Eq + Clone;

    // start and stop watching triggers with the underlying OS source. freddie calls these
    // with only the diff between iterations (register the added set, deregister the removed set).
    fn register(triggers: &HashSet<Self::Trigger>);
    fn deregister(triggers: &HashSet<Self::Trigger>);

    // block until a watched trigger fires; hand back the event and which trigger matched,
    // so dispatch can look the handler up by trigger.
    fn next() -> (Self, Self::Trigger);
}
```

Two things this sketch leaves open, both already in the open questions: whether `register`/`deregister`/`next` are associated functions (source state held globally per type) or take a source handle, and how the loop multiplexes `next` across several `EventSource` types into the one loop.

The name "keys" currently implies keyboard, but it should be generic over the event type. Keyboard is one instantiation; foregrounding an application is another. The same accumulation and dispatch machinery serves both, and anything else we decide to route this way.

## Accumulating the active binding set (a runtime computation via rayban)

We cannot know the active bindings statically. The nav layer might care only about `a` and `b`; the typing layer cares about `a` through `z`. If we tried to derive the full `a..z` set onto a layer struct statically, that does not work, because which bindings are live depends on the current state. So it is a runtime thing.

Given the current state:

1. Resolve to the active leaf with rayban.
2. Walk upward via `into_parent`, and at each node collect that node's bindings into a single accumulated map.

This coalesces the bindings from the active leaf up through its layer and any globals into one map for the current state.

### Precedence on conflict

Inner clobbers outer. The inner binding (closer to the active leaf, the more-specific node) wins over the outer one (closer to the root, the global default). Concretely, walking up from the active leaf, the first binding seen for a given trigger wins, and bindings encountered higher up do not overwrite it.

The constraint that forces this: there may be a global escape binding that applies to nav and the other layers, but in the typing layer escape should type an escape character. The typing layer's escape (inner) therefore clobbers the global escape (outer).

### Clobberable vs non-clobberable

A binding can be marked non-clobberable. The behavior is one of:

- The binding is non-clobberable, in which case a more-specific node attempting to override it is an error (caught at runtime, during accumulation).
- Otherwise, the more-specific binding clobbers it.

The global escape above is clobberable, which is why typing may override it. A binding we never want a layer to steal would be marked non-clobberable.

### Static winner vs dynamic fall-through

Two ways to resolve which handler actually runs:

- The static model picks a single winner per trigger during accumulation (inner clobbers outer), and that one handler runs. It is simple, but an inner handler cannot conditionally decline.
- The dynamic fall-through model keeps the chain of handlers per trigger, inner first. When the event fires, each handler returns an `Option` saying whether it handled it; if the inner handler declines, dispatch falls through to the next one outward, up to the top. An inner binding can then handle some cases and pass the rest to the global default.

The dynamic model can do everything the static one can; it costs an extra chain to keep and walk at dispatch.

The handled signal could be richer than a two-state `Option`: a third outcome (for example handled-but-keep-going, or an explicit block) is conceivable, but the need is unclear, so it is deferred.

## The event loop

One loop listens to all event sources we choose to route this way: keyboard, foregrounding, and whatever else. Not everything has to go through this loop, but routing everything through it is the default.

Each iteration:

1. From the current state, accumulate the active binding set (resolve, walk up, coalesce, applying the precedence and clobber rules).
2. Register and subscribe to exactly that set of events, and listen.
3. When an event fires, use rayban to find the single handler for it and execute it.

Registering only the accumulated set means we listen for exactly what the current state cares about, and unregistered events behave however the source's default is (for example, keys we do not bind pass through and type normally).

Dispatch is synchronous. An event arrives, we resolve and run its handler to completion, the handler mutates the state in memory and returns its effects, the shell performs those effects, and only then do we take the next event. The next iteration re-accumulates the binding set from the new state, so a layer switch immediately changes what we listen for.

### Registration and diffing

Each event type knows how to register a set of triggers with its source, and equally how to deregister. Between iterations we do not tear everything down and rebuild; we diff. If state A's set for a type is `{a, b, c}` and state B's is `{a, b, d}`, the diff says deregister `c` and register `d`, and freddie applies only that delta. Because triggers are `Hash + Eq`, the sets are `HashSet`s and the diff is set subtraction. This diffing is freddie's job, sitting between the accumulator and the per-event-type registration.

## Handler dispatch

When an event is handled, rayban finds the first and only handler for it, and runs it. The handler receives:

- the event, and
- a rayban path to the node the binding is on.

Through that path the handler can mutate the node it is on (`get_mut`) and traverse upward to its ancestors (`into_parent`), for example to switch layers or update parent state.

Example: with `#[key(Keyboard, 'g', open_chrome)]` on `Inner`, the function `open_chrome` receives both the event and a path to the `Inner` object. It can mutate `Inner` and walk up from there.

```rust
fn open_chrome(event: Keyboard, path: Path<Inner, _>) -> Option<Vec<Effect>> {
    // 'g' opens Chrome and drops us back to nav, so reach the root and swap the active layer
    path.get_root().layers = Layers::Nav(Nav::default());
    // hand the shell the effect; it does the actual foregrounding
    Some(vec![Effect::Foreground(App::Chrome)])
}
```

Nothing here touches `Inner`'s own state, but the path makes that available too (`get_mut`) for handlers that do. The `Some` marks the event handled under the dynamic model; the static version drops the `Option` and just returns the `Vec`.

## What handlers do

A handler is a pure function of the event and the path to its node. It produces two things, neither of which performs real I/O:

1. State mutations, applied in memory through the path. It mutates the node it is on (`get_mut`) and can walk up (`into_parent`, `get_root`) to change ancestors or switch layers. The two normal ones are modifying the current node's state, and navigating to the root to change layers (escape, for instance, almost always returns to the nav layer).
2. A description of side effects to perform, returned as data. The handler does not type the letter or call Hammerspoon itself; it returns an effect value and the runtime shell performs it.

This is functional-core, imperative-shell. The handler is the testable core: given a state and an event, you assert the resulting state and the returned effects, with no real keyboard or Hammerspoon involved. The loop is the shell that actually performs the effects.

### Effects as data

Side effects are a single enum, e.g. `enum Effect { EmitKey(Key), ShowOverlay, Foreground(App), Arbitrary(..), .. }`, and a handler returns a collection of them (order may matter, so probably a `Vec`).

The effect type is independent of the trigger. A key-triggered handler is not restricted to keyboard-shaped effects; `open_chrome` is reached via a `Keyboard` trigger but may foreground an app, show an overlay, or run something arbitrary. So there is one `Effect` union shared by every handler, not a per-binding or per-event-type effect type, and `Arbitrary` is the escape hatch for the long tail.

The enum is hand-written, with one variant per effect type, and `From` derived per variant (`#[derive(derive_more::From)]`) so a handler lifts individual effects in with `.into()` rather than naming the variant. The "sink" is purely shell-side: the shell `match`es each `Effect` variant to whatever performs it. Because every handler returns the same `Effect`, effects travel back to the loop as a uniform `Vec<Effect>` and the shell performs them; nothing about the effect type has to cross the dispatch erasure boundary.

- Remapping A to B: the handler returns `EmitKey(b).into()`. Emitting a key is just another effect, not a special case.
- A to B and also an overlay: the handler returns both `EmitKey(b)` and `ShowOverlay`, two variants of the same `Effect`, in one `Vec`.

Because the handler only mutates in memory and returns effect data, the inner handler is trivially testable in isolation. Deriving `PartialEq` on `Effect` is what lets a test `assert_eq!` on the returned vec.

Under the dynamic fall-through model the return also carries whether the event was handled, so the type is likely `Option<Vec<Effect>>`: `None` falls through to the outer handler, `Some(effects)` means handled, perform these. The static model just returns the effects.

### Navigating to the root

For the layer switch above, a handler needs to reach the root from wherever it was bound, not just one level up. `into_parent` moves up one level; on top of that we want a way to walk all the way to the root so a handler can switch layers regardless of its depth.

### get_root

There is a trait, call it `GetRoot`, with a single function `get_root`, implemented on all the different path types. `get_root` walks the parent chain to the root and yields the root, so the handler can change the active layer there. Changing layers is a mutation of the root enum's active variant: `get_root` gives a mutable handle at the root, and the handler replaces the variant (for example, set the root to the nav layer).

Because each path's parent chain terminates at `&mut Root`, `get_root` is a fixed walk up the chain, the same shape rayban already uses for `into_parent`. The rayban derive knows each node's parent, so it can generate the `GetRoot` impl per path type the same way it generates `Resolve`. Whether this lives in rayban or freddie is an open question below.

## Open questions

1. Handler signature at the dispatch boundary. The handler conceptually wants the typed `Path<Node, Parent>`, but the loop dispatches dynamically over a heterogeneous map, so there is an erasure boundary between "accumulated map of handlers" and "typed path handed to the handler." Design how the typed path is reconstructed for the chosen handler.
2. Per-event-type trigger types. A keyboard trigger is a key; a foreground trigger is an application identifier. The accumulated map is keyed by (event type, trigger), so each event type needs its own trigger type, and the derive must keep them straight.
3. Non-clobberable enforcement timing. Enforce during accumulation (so the error can name both the protected binding and the offending override) rather than at registration.
4. What "register" means per source: OS key grabs versus a polled or subscribed foreground watcher, and the teardown when the active set changes between iterations.
5. Single winner vs handler chain. Under the static model, accumulation yields one winner per (event type, trigger); under the dynamic fall-through model, it yields an ordered chain that dispatch walks. Decide which, and for the static model, what happens if two equally-specific bindings collide.
6. Relationship to the missing rayban features. Layers as an enum and upward traversal are supported today; if a layer ever holds a list of sub-states (a `Vec` of children), that hits the collections gap in rayban-missing-features.md and the active-child selection it describes.
7. Where `GetRoot` lives. It is a generalization of `into_parent` to "walk all the way up," so rayban is the natural home, and the derive can emit it from the parent chain alongside `Resolve`. But layer-switching is freddie semantics. Decide whether rayban exposes the generic `get_root` and freddie builds layer-switching on it, or freddie owns the whole trait.
8. What `get_root` returns. A `&mut Root` is enough to replace the active layer (`*root = App::Nav(..)`), but a typed path at the root would let the handler re-descend into the new layer in the same step. Decide between the bare `&mut Root` and a root path, given that after a layer switch the next loop iteration re-resolves anyway.
9. Effect feedback. The shell performs returned effects. Does `EmitKey(b)` re-enter freddie's own loop, so `b`'s binding can fire, or go straight to the OS as a synthesized keystroke? Decide whether effects cascade, and if they can, how to bound the cascade and avoid loops.
10. Effect ordering and atomicity. Effects are a `Vec`, so confirm they run in order. The state mutation lands in memory before any effect runs; decide whether that matters for effects that should reflect the new state (an overlay showing the post-mutation layer, say).
