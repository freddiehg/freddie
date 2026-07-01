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
    type Output = Option<Vec<MercuryEffect>>;
}
```

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

## Error

```rust
struct NoHandler; // no binding on the active path matched the event
```
