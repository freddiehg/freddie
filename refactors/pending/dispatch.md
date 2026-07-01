# dispatch (design)

Dispatch takes a fired event and runs the one handler the current state binds for it, or errors when none does. It builds on the accumulate tree and on laserbeam paths. Only accumulate is implemented so far; this is the design for the other half.

## Types

Each source has an event type (`KeyboardEvent`, `ForegroundEvent`). `MercuryEvent` unifies them, one variant per source. `Bindings` grows the two associated types accumulate did not need:

```rust
trait Bindings {
    type Trigger: Clone + Eq + Hash;
    type Event;
    type Output;
}

impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
    type Event = MercuryEvent;
    type Output = Vec<MercuryEffect>;
}
```

`Output` is the handler's return, the effect data. It is not wrapped in `Option`: with no clobbering there is no fall-through decline to signal, and handled-vs-not already lives in dispatch's `Result` (`Ok(effects)` vs `Err(NoHandler)`). bind returns this value and nothing more; performing the effects is the consumer's own code.

A trigger already serves as a `Hash + Eq` key in the accumulate set. At dispatch it also matches events, so each source's trigger type matches against that source's event:

```rust
trait EventTrigger {
    type Event;
    fn is_matching(&self, event: &Self::Event) -> bool;
}

impl EventTrigger for Keyboard {
    type Event = KeyboardEvent;
    fn is_matching(&self, ev: &KeyboardEvent) -> bool {
        self.key == ev.key && self.mods == ev.mods
    }
}
impl EventTrigger for Foreground {
    type Event = ForegroundEvent;
    fn is_matching(&self, ev: &ForegroundEvent) -> bool {
        self.app == ev.app
    }
}
```

The source event is pulled out of `MercuryEvent`, which the app provides (derivable) per source:

```rust
trait FromEvent<E> {
    fn from_event(event: &E) -> Option<&Self>;
}
impl FromEvent<MercuryEvent> for KeyboardEvent {
    fn from_event(e: &MercuryEvent) -> Option<&Self> {
        match e {
            MercuryEvent::Keyboard(k) => Some(k),
            _ => None,
        }
    }
}
```

## Matching a binding

A binding `Keyboard::new("g") => on_g` matches on two levels, and both must hold:

- Source: is the fired event a keyboard event at all? `FromEvent::from_event(&event)` returns `Some(&KeyboardEvent)` or `None`. This is the type match.
- Key: is it the `g` key? `Keyboard::new("g").is_matching(ev)`. This is the value match.

The source event type, the trigger's `Event`, and `on_g`'s parameter are the same type, so inference pins all three and the derive names none of them:

```rust
// generated for `Keyboard::new("g") => on_g`:
if let Some(ev) = FromEvent::from_event(event) {   // source/type match
    if Keyboard::new("g").is_matching(ev) {        // key/value match
        return Ok(on_g(ev, path));
    }
}
```

## Why not type-level

A type per key would make the key match pure type dispatch, with no runtime predicate, and keyboard keys are a finite set you could enumerate and register in advance. Foreground app identifiers are open: any string is a valid app, so you cannot mint a type per app, and that match has to be a runtime predicate. One mechanism covers both, so matching is the runtime `is_matching`, and type-level matching is a non-goal.

## The traversal

The loop hands dispatch the event. Dispatch descends the active path from the root, the same path `resolve` walks. At each node it tries that node's binds with the two-level match above. The first binding that matches calls its handler with the node's path and returns. On no match it descends into the active child. Reaching a leaf with nothing matched returns `NoHandler`.

With no clobbering, at most one binding on the active path matches, so descent order does not change the result; the descent only has to visit every node on the path.

```rust
// generated on the Layer enum (Layer has one bind, then two variants):
fn dispatch(mut path: Path<Layer, ParentPath>, event: &MercuryEvent)
    -> Result<MercuryOutput, NoHandler>
{
    if let Some(ev) = FromEvent::from_event(event) {
        if Keyboard::new("f1").is_matching(ev) {
            return Ok(show_help(ev, path));
        }
    }
    match path.get_mut() {               // descend into the active variant
        Layer::Nav(_)    => Nav::dispatch(nav_child(path), event),
        Layer::Typing(_) => Typing::dispatch(typing_child(path), event),
    }
}
```

`nav_child` and `typing_child` build the child path with laserbeam's `Path::from_fn`, as `resolve` does. A leaf with no matching bind returns `Err(NoHandler)`.

## The path

The handler needs its node's typed `Path<Node, Parent>`. Dispatch builds it while descending, the same construction `resolve` performs, and hands the matching node its path. So dispatch, unlike accumulate, constructs paths, and the descent duplicates laserbeam's resolve walk. The bind derive either reuses laserbeam's descent or regenerates it.

## The dispatch derive

Per node, the derive generates a `dispatch` that checks the node's binds (the two-level match above, calling the handler with the node's path on a hit), then descends into the active child and recurses. The descent is the hard part, because it must build each node's laserbeam `Path`.

That descent is what `resolve` already generates: the `Path::from_fn` projection, the active-variant `match`, `Box` unwrap, and the multi-parent route enums. But `resolve` runs it only to reach the leaf and return `Resolved`; it does not check anything at intermediate nodes or expose their paths, so dispatch cannot reuse it as-is. And it must be a descent, not an ascent: a node's attributes name its children (`#[resolve_into]`, variants), never its parent, so a node cannot generate "call my parent's dispatch."

So the dispatch derive needs the same descent codegen `resolve` has, plus a per-node bind-check. Multi-parent nodes (route enums) are core to the tree, and they are the bulk of that descent's complexity, so duplicating it in `bind_macro` is out. The descent generation gets factored into a shared generator that both `resolve` and `dispatch` drive.

The generator emits the descent — the `Path::from_fn` projections, `Box` unwrap, the active-variant `match`, and the route-enum wrapping — and takes three hooks that `resolve` and `dispatch` fill differently:

- `prefix(path)`: tokens run at each node before it descends. `resolve` emits nothing. `dispatch` emits the bind-check, which may `return handler(ev, path)` on a hit and otherwise falls through, leaving `path` for the descent.
- `recurse(child, child_path)`: the call into the child. `resolve` emits `<child>::resolve(child_path)`. `dispatch` emits `<child>::dispatch(child_path, event)`.
- `leaf(path)`: what a leaf yields when it does not descend. `resolve` emits `Resolved::Name(path)`. `dispatch` emits `NoHandler`.

The projections, the `Box` handling, and the route-enum wrapping stay in the generator, written once. This means refactoring `laserbeam_macro`'s `struct_body`/`enum_body` into the generator plus `resolve`'s three hooks, then `bind_macro` supplies dispatch's three hooks. The generator lives beside the parsing helpers in `derive_support`, or a codegen sibling.

### What dispatch generates per node

The generator emits `prefix`, then the descent (or the leaf terminal). `dispatch`'s `prefix` is the bind-check, its `recurse` calls `Dispatch`, and its `leaf` is `NoHandler`. `path` is the node's laserbeam path; `event` is `&M::Event`, in scope from the fn signature.

A struct leaf (no `#[resolve_into]`): prefix, then the leaf terminal.

```rust
fn dispatch(path: Self::Path<'_>, event: &M::Event) -> Result<M::Output, NoHandler> {
    if let Some(ev) = FromEvent::from_event(event)
        && Keyboard::new("g").is_matching(ev)
    {
        return Ok(on_g(ev, path));
    }
    Err(NoHandler)
}
```

A struct with one single-parent `#[resolve_into] child: Child`: prefix, then recurse. The `Path::from_fn(path, projection).into()` is byte-for-byte what the generator builds for `resolve`; dispatch only swaps the trailing call.

```rust
fn dispatch(path: Self::Path<'_>, event: &M::Event) -> Result<M::Output, NoHandler> {
    // prefix: this node's own binds, as above
    <Child as Dispatch<M>>::dispatch(
        Path::from_fn(path, |np| &mut np.get_mut().child).into(),
        event,
    )
}
```

A multi-parent `#[resolve_into(parent = Route)]` child: the generator wraps the path in the route variant, exactly as `resolve` does, and dispatch recurses through it. This is the case that must be shared rather than duplicated.

```rust
    // prefix ...
    <Child as Dispatch<M>>::dispatch(
        Path::from_fn(Route::ThisNode(path.into()), |p| {
            let Route::ThisNode(pp) = p else { unreachable!() };
            &mut pp.get_mut().child
        }),
        event,
    )
```

An enum: prefix (the enum's own binds), then one arm per variant, each recursing into that variant's inner with the projection the generator emits (single- or multi-parent, `Box`-unwrapped). The active variant always matches one arm and returns, so the fallthrough is `unreachable!()` for both `resolve` and `dispatch`; the enum has no leaf terminal.

```rust
fn dispatch(mut path: Self::Path<'_>, event: &M::Event) -> Result<M::Output, NoHandler> {
    // prefix: the enum's own binds ...
    if matches!(path.get_mut(), Self::Nav(_)) {
        return <Nav as Dispatch<M>>::dispatch(
            Path::from_fn(path, |np| {
                let Self::Nav(c) = np.get_mut() else { unreachable!() };
                c
            })
            .into(),
            event,
        );
    }
    // ... one arm per remaining variant ...
    unreachable!()
}
```

`NoHandler` propagates up: an interior node returns the child's `dispatch` result, so an unmatched active leaf's `NoHandler` travels back through every ancestor whose own binds also missed.

### The Dispatch trait and entry

`dispatch` is a trait method, mirroring `Resolve`, parameterized by the marker so it can name `M::Event` and `M::Output`. The node's path type is its `Resolve::Path`. One shape binds `Dispatch` on `Resolve` and reuses that path type:

```rust
trait Dispatch<M: Bindings>: Resolve {
    fn dispatch(path: Self::Path<'_>, event: &M::Event) -> Result<M::Output, NoHandler>;
}
```

`Bindings` carries all three associated types (`Trigger`, `Event`, `Output`); accumulate reads `Trigger`, dispatch reads all three. The root's `Path<'a>` is `&'a mut Self`, so the entry is `<Root as Dispatch<M>>::dispatch(&mut root, &event)`.

### Open design points

- Whether `Dispatch: Resolve` (reuse `Resolve::Path`) or a standalone `Dispatch` with its own `type Path<'a>`.
- The generator's hook interface: `prefix`/`recurse`/`leaf` as `Fn(..) -> TokenStream` closures the two macros pass in, versus a trait the generator is generic over.
- Whether the root's `&mut Self` path needs a special `prefix` shape, since its handler paths are `&mut Root` rather than a `Path`.

## Error

```rust
struct NoHandler; // no binding on the active path matched the event
```
