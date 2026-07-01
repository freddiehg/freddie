# dispatch (design)

Dispatch takes a fired event and runs the handler the current state binds for it. It tries the leafward subtree first, so a child's binding takes priority over an ancestor's (which is what clobbering will need), and falls back to the node's own binds. It builds on the accumulate tree and on laserbeam paths. Only accumulate is implemented so far; this is the design for the other half.

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

A binding's trigger expression is a per-source value, e.g. `Keyboard::new("g")`. It plays two roles: accumulate lifts it into `Bindings::Trigger` via `Into`, and dispatch matches it against events, so the per-source trigger type implements `EventTrigger`:

```rust
trait EventTrigger {
    type Event;
    fn is_matching(&self, event: &Self::Event) -> bool;
}

impl EventTrigger for Keyboard {
    type Event = KeyboardEvent;
    fn is_matching(&self, ev: &KeyboardEvent) -> bool {
        self.key == ev.key
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
- Key: is it the `g` key? `is_matching(&trigger, ev)`. This is the value match.

The source event type, the trigger's `Event`, and `on_g`'s parameter are the same type, so inference pins all three and the derive names none of them. The trigger is built into a local, and the calls are UFCS because the traits are not imported in the consumer's crate:

```rust
if let Some(ev) = ::bind::FromEvent::from_event(event) {   // source/type match
    let trigger = Keyboard::new("g");
    if ::bind::EventTrigger::is_matching(&trigger, ev) {    // key/value match
        return Ok(on_g(ev, path));
    }
}
```

## The traversal

The loop hands dispatch the event. Each node first tries its active child, and checks its own binds only when the child subtree returns nothing, so a child's binding beats an ancestor's. A node returns `Ok(effects)` when it or a descendant handles the event, or `Err(path)` when nothing at or below it matched, handing the path back so the parent can take its turn. The root's `Err(path)` becomes `None` at the entry.

## The Dispatch trait

`dispatch` is a trait method, mirroring `Resolve`, parameterized by the marker so it can name `M::Event` and `M::Output`. On a miss it returns the node's path, so the caller can walk up, so the error type is `Self::Path`:

```rust
trait Dispatch<M: Bindings>: Resolve {
    fn dispatch<'a>(path: Self::Path<'a>, event: &M::Event) -> Result<M::Output, Self::Path<'a>>
    where
        Self: 'a;
}
```

The root's `Path<'a>` is `&'a mut Self`, so the loop calls `<Root as Dispatch<M>>::dispatch(&mut root, &event).ok()` for an `Option<M::Output>`.

## The dispatch derive

Per node, `#[derive(Bind)]` emits the `Dispatch` impl alongside the `EventHandler` (accumulate) impl.

The child-path construction — the `Path::from_fn` projection, `Box` unwrap, the active-variant `match`, and the route-enum wrapping — is what `resolve` already generates, and it is the shared, complex part. `resolve` tail-calls the child's `resolve` on the constructed path. dispatch consumes the same construction with different control flow:

- recurse into the child: `match <child>::dispatch(child_path, event) { Ok(out) => return Ok(out), Err(child) => path = child.into_parent() }`. Through a route enum, `into_parent` yields the route enum, so it is matched back to this node's path.
- then the node's own binds (the two-level match), returning `Ok(handler(ev, path))` on a hit.
- then `Err(path)`, handing the path back up.

So the projection construction is shared with `resolve`; the control flow around it is per-derive. The work is refactoring `laserbeam_macro`'s projection generation into a shared piece both derives call.

### The generated code, spelled out

A concrete tree. `esc` is bound at the root (lowest priority), `f1` at the layer, `g` on nav:

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

The handlers, each taking its node's path type. The root's is `&mut Mercury`, since laserbeam's root path is `&mut Self`:

```rust
fn to_nav(ev: &KeyboardEvent, path: &mut Mercury) -> Vec<MercuryEffect> { .. }
fn show_help(ev: &KeyboardEvent, path: LayerPath) -> Vec<MercuryEffect> { .. }
fn on_g(ev: &KeyboardEvent, path: NavPath) -> Vec<MercuryEffect> { .. }
fn on_bksp(ev: &KeyboardEvent, path: TypingPath) -> Vec<MercuryEffect> { .. }
```

What the derive emits for `Dispatch`. Cross-crate paths are qualified; `<MercuryStruct as Bindings>::Event` is `MercuryEvent` and `::Output` is `Vec<MercuryEffect>`; std paths (`Result`, `Ok`, `matches!`) are left unqualified for readability.

Leaf: no child to try, so check the node's binds, else hand the path back.

```rust
#[automatically_derived]
impl ::bind::Dispatch<MercuryStruct> for Nav {
    fn dispatch<'a>(
        path: NavPath<'a>,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> Result<<MercuryStruct as ::bind::Bindings>::Output, NavPath<'a>>
    where
        Self: 'a,
    {
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("g");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return Ok(on_g(ev, path));
            }
        }
        Err(path)
    }
}
```

Root struct: try the child first, then this node's binds (`esc`, lowest priority).

```rust
#[automatically_derived]
#[allow(clippy::useless_conversion)]
impl ::bind::Dispatch<MercuryStruct> for Mercury {
    fn dispatch<'a>(
        mut path: &'a mut Mercury,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> Result<<MercuryStruct as ::bind::Bindings>::Output, &'a mut Mercury>
    where
        Self: 'a,
    {
        match <Layer as ::bind::Dispatch<MercuryStruct>>::dispatch(
            ::laserbeam::Path::from_fn(path, |o| &mut o.layer).into(),
            event,
        ) {
            Ok(out) => return Ok(out),
            Err(child) => path = child.into_parent(),
        }
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("esc");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return Ok(to_nav(ev, path));
            }
        }
        Err(path)
    }
}
```

Enum: try the active variant first, then the enum's own binds.

```rust
#[automatically_derived]
#[allow(clippy::useless_conversion)]
impl ::bind::Dispatch<MercuryStruct> for Layer {
    fn dispatch<'a>(
        mut path: LayerPath<'a>,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> Result<<MercuryStruct as ::bind::Bindings>::Output, LayerPath<'a>>
    where
        Self: 'a,
    {
        if matches!(path.get_mut(), Self::Nav(_)) {
            match <Nav as ::bind::Dispatch<MercuryStruct>>::dispatch(
                ::laserbeam::Path::from_fn(path, |np| {
                    let Self::Nav(c) = np.get_mut() else { unreachable!() };
                    c
                })
                .into(),
                event,
            ) {
                Ok(out) => return Ok(out),
                Err(child) => path = child.into_parent(),
            }
        } else if matches!(path.get_mut(), Self::Typing(_)) {
            match <Typing as ::bind::Dispatch<MercuryStruct>>::dispatch(
                ::laserbeam::Path::from_fn(path, |np| {
                    let Self::Typing(c) = np.get_mut() else { unreachable!() };
                    c
                })
                .into(),
                event,
            ) {
                Ok(out) => return Ok(out),
                Err(child) => path = child.into_parent(),
            }
        }
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("f1");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return Ok(show_help(ev, path));
            }
        }
        Err(path)
    }
}
```

`Typing` is another leaf, `Nav`-shaped: its `"bksp"` bind, then `Err(path)`.

The loop dispatches with `.ok()`:

```rust
let effects: Option<Vec<MercuryEffect>> =
    <Mercury as ::bind::Dispatch<MercuryStruct>>::dispatch(&mut mercury, &event).ok();
```

[A boxed `#[resolve_into]` field changes only the projection: the closure derefs the `Box`, `|np| &mut *np.get_mut().field` instead of `|np| &mut np.get_mut().field`. Same as `resolve`.]

Multi-parent: the descent wraps the path in the route enum, and the miss unwraps it back. If `Nav` also carried `#[resolve_into(parent = CursorParent)] cursor: Cursor`, reached from both `Nav` and `Typing` through `enum CursorParent { Nav(Path<Cursor, NavPath>), Typing(Path<Cursor, TypingPath>) }`, then `Nav`'s dispatch tries `cursor` before its own `"g"` bind:

```rust
        match <Cursor as ::bind::Dispatch<MercuryStruct>>::dispatch(
            ::laserbeam::Path::from_fn(CursorParent::Nav(path.into()), |p| {
                let CursorParent::Nav(pp) = p else { unreachable!() };
                &mut pp.get_mut().cursor
            }),
            event,
        ) {
            Ok(out) => return Ok(out),
            Err(child) => {
                // into_parent gives the route enum; match it back to Nav's path
                let CursorParent::Nav(nav_path) = child.into_parent() else { unreachable!() };
                path = nav_path;
            }
        }
```

### Open design point

The root handler's path is `&mut Root`, not a `Path`, because laserbeam's root `Path<'a>` is `&'a mut Self`. `Path<Root, ()>` is impossible: a `Path` reborrows its node out of its parent, and `()` holds no `Root`. Handlers could be made uniform by wrapping the root as `Path<Root, &mut Root>` (identity projection), whose parent is `&mut Root` rather than `()`; whether to do that is open.
