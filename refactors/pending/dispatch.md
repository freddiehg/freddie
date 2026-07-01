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
        return ControlFlow::Break(on_g(ev, path));
    }
}
```

## The traversal

The loop hands dispatch the event. Each node first tries its active child, and checks its own binds only when the child subtree returns nothing, so a child's binding beats an ancestor's. A node returns `Break(effects)` when it or a descendant handles the event, or `Continue(path)` when nothing at or below it matched, handing the path back so the parent can walk up and take its turn. The root's `Continue` becomes `None` at the entry.

## The Dispatch trait and entry

`dispatch` is a trait method, mirroring `Resolve`. It is the recursive form, and it returns `ControlFlow`: `Break(output)` when the event is handled (stop), `Continue(path)` when it is not. A `Path` is single-owner, so building the child path consumes this node's path; on a miss the child hands the path back in `Continue`, and the parent recovers it (`into_parent`) to check its own binds. An `Option`'s `None` could not carry the path.

```rust
trait Dispatch<M: Bindings>: Resolve {
    fn dispatch<'a>(path: Self::Path<'a>, event: &M::Event)
        -> ControlFlow<M::Output, Self::Path<'a>>
    where
        Self: 'a;
}
```

Consumers do not want the path. A public entry drops it and returns `Option<M::Output>`:

```rust
fn dispatch<M, N>(root: &mut N, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    N: Dispatch<M>,
{
    <N as Dispatch<M>>::dispatch(root, event).break_value()
}
```

The root's `Path<'a>` is `&'a mut Self`, so `root: &mut N` is the root's path, and the loop calls `bind::dispatch::<MercuryStruct, _>(&mut mercury, &event)`.

## The dispatch derive

Per node, `#[derive(Bind)]` emits the `Dispatch` impl alongside the `EventHandler` (accumulate) impl.

The child-path construction — the `Path::from_fn` projection, `Box` unwrap, the active-variant `match`, and the route-enum wrapping — is what `resolve` already generates, and it is the shared, complex part. `resolve` tail-calls the child's `resolve` on the constructed path. dispatch consumes the same construction with different control flow:

- recurse into the child: `let child = <child>::dispatch(child_path, event)?; path = child.into_parent();`. The `?` propagates `Break` (handled) up and yields the child path on `Continue`. Through a route enum, `into_parent` yields the route enum, so it is matched back to this node's path.
- then the node's own binds (the two-level match), returning `ControlFlow::Break(handler(ev, path))` on a hit.
- then `ControlFlow::Continue(path)`, handing the path back up.

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

What the derive emits for `Dispatch`. Cross-crate paths are qualified; `<MercuryStruct as Bindings>::Event` is `MercuryEvent` and `::Output` is `Vec<MercuryEffect>`; `ControlFlow` is `::core::ops::ControlFlow`, left unqualified for readability.

Leaf: no child to try, so check the node's binds, else hand the path back.

```rust
#[automatically_derived]
impl ::bind::Dispatch<MercuryStruct> for Nav {
    fn dispatch<'a>(
        path: NavPath<'a>,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> ControlFlow<<MercuryStruct as ::bind::Bindings>::Output, NavPath<'a>>
    where
        Self: 'a,
    {
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("g");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return ControlFlow::Break(on_g(ev, path));
            }
        }
        ControlFlow::Continue(path)
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
    ) -> ControlFlow<<MercuryStruct as ::bind::Bindings>::Output, &'a mut Mercury>
    where
        Self: 'a,
    {
        let child = <Layer as ::bind::Dispatch<MercuryStruct>>::dispatch(
            ::laserbeam::Path::from_fn(path, |o| &mut o.layer).into(),
            event,
        )?;
        path = child.into_parent();
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("esc");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return ControlFlow::Break(to_nav(ev, path));
            }
        }
        ControlFlow::Continue(path)
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
    ) -> ControlFlow<<MercuryStruct as ::bind::Bindings>::Output, LayerPath<'a>>
    where
        Self: 'a,
    {
        match path.get_mut() {
            Self::Nav(_) => {
                let child = <Nav as ::bind::Dispatch<MercuryStruct>>::dispatch(
                    ::laserbeam::Path::from_fn(path, |np| {
                        let Self::Nav(c) = np.get_mut() else { unreachable!() };
                        c
                    })
                    .into(),
                    event,
                )?;
                path = child.into_parent();
            }
            Self::Typing(_) => {
                let child = <Typing as ::bind::Dispatch<MercuryStruct>>::dispatch(
                    ::laserbeam::Path::from_fn(path, |np| {
                        let Self::Typing(c) = np.get_mut() else { unreachable!() };
                        c
                    })
                    .into(),
                    event,
                )?;
                path = child.into_parent();
            }
        }
        if let Some(ev) = ::bind::FromEvent::from_event(event) {
            let trigger = Keyboard::new("f1");
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return ControlFlow::Break(show_help(ev, path));
            }
        }
        ControlFlow::Continue(path)
    }
}
```

`Typing` is another leaf, `Nav`-shaped: its `"bksp"` bind, then `ControlFlow::Continue(path)`.

The loop dispatches through the public entry:

```rust
let effects: Option<Vec<MercuryEffect>> =
    ::bind::dispatch::<MercuryStruct, _>(&mut mercury, &event);
```

A boxed `#[resolve_into]` field changes only the projection: the closure derefs the `Box`, `|np| &mut *np.get_mut().field` instead of `|np| &mut np.get_mut().field`. Same as `resolve`.

Multi-parent: the descent wraps the path in the route enum, and the miss unwraps it back. If `Nav` also carried `#[resolve_into(parent = CursorParent)] cursor: Cursor`, reached from both `Nav` and `Typing` through `enum CursorParent { Nav(NavPath), Typing(TypingPath) }` (one variant per parent path, so `Cursor`'s path is `Path<Cursor, CursorParent>`), then `Nav`'s dispatch tries `cursor` before its own `"g"` bind:

```rust
        let child = <Cursor as ::bind::Dispatch<MercuryStruct>>::dispatch(
            ::laserbeam::Path::from_fn(CursorParent::Nav(path.into()), |p| {
                let CursorParent::Nav(pp) = p else { unreachable!() };
                &mut pp.get_mut().cursor
            }),
            event,
        )?;
        // into_parent gives the route enum; match it back to Nav's path
        let CursorParent::Nav(nav_path) = child.into_parent() else { unreachable!() };
        path = nav_path;
```

### Open design point

The root handler's path is `&mut Root`, not a `Path`, because laserbeam's root `Path<'a>` is `&'a mut Self`. `Path<Root, ()>` is impossible: a `Path` reborrows its node out of its parent, and `()` holds no `Root`. Handlers could be made uniform by wrapping the root as `Path<Root, &mut Root>` (identity projection), whose parent is `&mut Root` rather than `()`; whether to do that is open.
