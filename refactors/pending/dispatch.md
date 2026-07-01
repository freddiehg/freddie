# dispatch (design)

Dispatch takes a fired event and runs the one handler the current state binds for it, or errors when none does. It builds on the accumulate tree and on laserbeam paths. Only accumulate is implemented so far; this is the design for the other half.

## Types

Each source has an event type (`KeyboardEvent`, `ForegroundEvent`). `MercuryEvent` unifies them, one variant per source. `Bindings` grows the two associated types accumulate did not need:

```rust
trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Output;
}

impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
    type Event = MercuryEvent;
    type Output = Vec<MercuryEffect>;
}
```

`Output` is the handler's return, the effect data. bind returns it and nothing more; performing the effects is the consumer's own code.

A binding's trigger expression is a per-source value, e.g. `Keyboard::new("g")`. It plays two roles. Accumulate lifts it into `Bindings::Trigger` via `Into`. Dispatch matches it against events, so the per-source trigger type implements `EventTrigger`:

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

The `MercuryTrigger` enum stays a plain `Eq + Hash` key; `EventTrigger` lives on the source types. The generated dispatch requires it the way accumulate requires `Into<MercuryTrigger>`.

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

- Source: is the fired event a keyboard event at all? `FromEvent::from_event(event)` returns `Some(&KeyboardEvent)` or `None`. This is the type match.
- Key: is it the `g` key? `Keyboard::new("g").is_matching(ev)`. This is the value match.

The source event type, the trigger's `Event`, and `on_g`'s parameter are the same type, so inference pins all three and the derive names none of them:

```rust
if let Some(ev) = ::bind::FromEvent::from_event(event) {     // source/type match
    let trigger = Keyboard::new("g");
    if ::bind::EventTrigger::is_matching(&trigger, ev) {     // key/value match
        return Ok(on_g(ev, path));
    }
}
```

## The traversal

The loop hands dispatch the event as `&M::Event`. Dispatch descends the active path from the root, the same path `resolve` walks. At each node it tries that node's binds with the two-level match. The first binding that matches calls its handler with the node's path and returns. On no match it descends into the active child, and reaching a leaf with nothing matched returns `NoHandler`.

With no clobbering, at most one binding on the active path matches, so descent order does not change the result. `NoHandler` propagates up: an interior node returns its child's `dispatch` result, so an unmatched active leaf's `NoHandler` travels back through every ancestor whose own binds also missed.

## The path

The handler needs its node's typed `Path<Node, Parent>`. Dispatch builds it while descending, the same construction `resolve` performs, and hands the matching node its path.

## The dispatch derive

Per node, the derive generates a `dispatch` that checks the node's binds (the two-level match, calling the handler with the node's path on a hit), then descends into the active child and recurses, building each node's laserbeam `Path` as it goes.

That descent — the `Path::from_fn` projections, the active-variant `match`, `Box` unwrap, and the multi-parent route enums — is shared with `resolve`: a single generator drives both, and `bind_macro` supplies the per-node bind-check on top.

The generator emits the descent and takes three hooks that `resolve` and `dispatch` fill differently:

- `prefix(path)`: tokens run at each node before it descends. `resolve` emits nothing. `dispatch` emits the bind-check, which may `return Ok(handler(ev, path))` on a hit and otherwise falls through, leaving `path` for the descent.
- `recurse(child, child_path)`: the call into the child. `resolve` emits `<child>::resolve(child_path)`. `dispatch` emits `<child>::dispatch(child_path, event)`.
- `leaf(path)`: what a leaf yields when it does not descend. `resolve` emits `Resolved::Name(path)`. `dispatch` emits `Err(NoHandler)`.

Refactoring `laserbeam_macro`'s `struct_body`/`enum_body` into the generator plus `resolve`'s hooks, then `bind_macro` supplies dispatch's hooks. The generator lives beside the parsing helpers in `derive_support`, or a codegen sibling.

### The Dispatch trait

`dispatch` is a trait method, mirroring `Resolve`: parameterized by the marker so it can name `M::Event` and `M::Output`, with the node's path as its `Resolve::Path`.

```rust
trait Dispatch<M: Bindings>: Resolve {
    fn dispatch<'a>(path: Self::Path<'a>, event: &M::Event) -> Result<M::Output, NoHandler>
    where
        Self: 'a;
}
```

The root's `Path<'a>` is `&'a mut Self`, so the entry is `<Root as Dispatch<M>>::dispatch(&mut root, &event)`.

### The generated code, spelled out

A concrete tree. The state, with both derives and the binds at each level:

```rust
#[derive(Laserbeam, Bind)]
#[laserbeam_root(resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Keyboard::new("esc") => to_nav)]
struct Mercury {
    #[resolve_into]
    layer: Layer,
}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = LayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Keyboard::new("f1") => show_help)]
enum Layer {
    Nav(Nav),
    Typing(Typing),
}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = NavPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Keyboard::new("g") => on_g)]
struct Nav {}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = TypingPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Keyboard::new("bksp") => on_bksp)]
struct Typing {}

type LayerPath<'a> = Path<Layer, &'a mut Mercury>;
type NavPath<'a> = Path<Nav, LayerPath<'a>>;
type TypingPath<'a> = Path<Typing, LayerPath<'a>>;
```

The handlers the user writes, each taking its node's path type:

```rust
fn to_nav(ev: &KeyboardEvent, path: &mut Mercury) -> Vec<MercuryEffect> { .. }
fn show_help(ev: &KeyboardEvent, path: LayerPath) -> Vec<MercuryEffect> { .. }
fn on_g(ev: &KeyboardEvent, path: NavPath) -> Vec<MercuryEffect> { .. }
fn on_bksp(ev: &KeyboardEvent, path: TypingPath) -> Vec<MercuryEffect> { .. }
```

The `Bind` derive emits the `Dispatch` impl alongside the `EventHandler` (accumulate) impl. What it emits for `Dispatch`, per node. Cross-crate paths are qualified; `<MercuryStruct as Bindings>::Event` is `MercuryEvent` and `::Output` is `Vec<MercuryEffect>`; std paths (`Result`, `Ok`, `matches!`) are left unqualified for readability.

Root struct: a bind, then descent into `#[resolve_into] layer`.

```rust
#[automatically_derived]
#[allow(clippy::useless_conversion)]
impl ::bind::Dispatch<MercuryStruct> for Mercury {
    fn dispatch<'a>(
        path: &'a mut Mercury,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> Result<<MercuryStruct as ::bind::Bindings>::Output, ::bind::NoHandler>
    where
        Self: 'a,
    {
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("esc");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return Ok(to_nav(ev, path));
            }
        }
        <Layer as ::bind::Dispatch<MercuryStruct>>::dispatch(
            ::laserbeam::Path::from_fn(path, |o| &mut o.layer).into(),
            event,
        )
    }
}
```

Enum: a bind, then a match on the active variant.

```rust
#[automatically_derived]
#[allow(clippy::useless_conversion)]
impl ::bind::Dispatch<MercuryStruct> for Layer {
    fn dispatch<'a>(
        mut path: LayerPath<'a>,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> Result<<MercuryStruct as ::bind::Bindings>::Output, ::bind::NoHandler>
    where
        Self: 'a,
    {
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("f1");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return Ok(show_help(ev, path));
            }
        }
        if matches!(path.get_mut(), Self::Nav(_)) {
            return <Nav as ::bind::Dispatch<MercuryStruct>>::dispatch(
                ::laserbeam::Path::from_fn(path, |np| {
                    let Self::Nav(c) = np.get_mut() else { unreachable!() };
                    c
                })
                .into(),
                event,
            );
        }
        if matches!(path.get_mut(), Self::Typing(_)) {
            return <Typing as ::bind::Dispatch<MercuryStruct>>::dispatch(
                ::laserbeam::Path::from_fn(path, |np| {
                    let Self::Typing(c) = np.get_mut() else { unreachable!() };
                    c
                })
                .into(),
                event,
            );
        }
        unreachable!()
    }
}
```

Leaf struct: a bind, then `NoHandler`.

```rust
#[automatically_derived]
impl ::bind::Dispatch<MercuryStruct> for Nav {
    fn dispatch<'a>(
        path: NavPath<'a>,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> Result<<MercuryStruct as ::bind::Bindings>::Output, ::bind::NoHandler>
    where
        Self: 'a,
    {
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("g");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return Ok(on_g(ev, path));
            }
        }
        Err(::bind::NoHandler)
    }
}
```

`Typing` is another leaf, `Nav`-shaped: its `"bksp"` bind, then `Err(::bind::NoHandler)`.

[A boxed `#[resolve_into]` field changes only the projection: the closure derefs the `Box`, `|np| &mut *np.get_mut().field` instead of `|np| &mut np.get_mut().field`. Same for `resolve`.]

Multi-parent: the descent wraps the path in the route enum, as `resolve` does. If `Nav` also carried `#[resolve_into(parent = CursorParent)] cursor: Cursor`, reached from both `Nav` and `Typing` through `enum CursorParent { Nav(Path<Cursor, NavPath>), Typing(Path<Cursor, TypingPath>) }`, then `Nav`'s descent (in place of its `Err(NoHandler)`) is:

```rust
        <Cursor as ::bind::Dispatch<MercuryStruct>>::dispatch(
            ::laserbeam::Path::from_fn(CursorParent::Nav(path.into()), |p| {
                let CursorParent::Nav(pp) = p else { unreachable!() };
                &mut pp.get_mut().cursor
            }),
            event,
        )
```

### Open design points

- Whether `Dispatch: Resolve` (reuse `Resolve::Path`) or a standalone `Dispatch` with its own `type Path<'a>`.
- The generator's hook interface: `prefix`/`recurse`/`leaf` as `Fn(..) -> TokenStream` closures the two macros pass in, versus a trait the generator is generic over.
- Whether the root's `&mut Self` path needs a distinct `prefix` shape, since its handler paths are `&mut Root` rather than a `Path`.

## Error

```rust
struct NoHandler; // no binding on the active path matched the event
```
