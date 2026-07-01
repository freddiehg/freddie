# dispatch (design)

Dispatch takes a fired event and runs the one handler the current state binds for its trigger, or errors when none does. It builds on the accumulate tree and on laserbeam paths. Only accumulate is implemented so far; this is the design for the other half.

## The event enum mirrors the trigger enum

Each source has an event type (`KeyboardEvent`, `ForegroundEvent`). `MercuryEvent` unifies them, one variant per source, and `derive_more::TryInto` projects it back to a source event:

```rust
#[derive(derive_more::TryInto)]
enum MercuryEvent {
    Keyboard(KeyboardEvent),
    Foreground(ForegroundEvent),
}
```

Dispatch needs two associated types accumulate did not, so `Bindings` grows them:

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

## Calling on_g

`on_g` takes a `KeyboardEvent`; dispatch holds a `MercuryEvent`. It extracts with `TryInto` and lets inference pick the target from `on_g`'s parameter, so the derive never names `KeyboardEvent`:

```rust
on_g(event.try_into()?, path)
```

This is the event-side mirror of the trigger lift. A trigger goes source to unified through `From` (`Keyboard -> MercuryTrigger`); an event goes unified to source through `TryInto` (`MercuryEvent -> KeyboardEvent`). mercury derives both: `derive_more::From` on `MercuryTrigger`, `derive_more::TryInto` on `MercuryEvent`. The extraction fails only when the event's variant disagrees with the matched trigger's source, which the loop should never deliver, so dispatch reports it rather than trusting it.

## The traversal

The loop hands dispatch the fired trigger and the event. The trigger says which binding to run; the event is the payload.

Dispatch descends the active path from the root, the same path `resolve` walks. At each node it compares the fired trigger to that node's binds. On a match it extracts the event, calls that node's handler with the node's path, and returns. On no match it descends into the active child. Reaching a leaf with nothing matched returns `DispatchError::NoHandler`.

With no clobbering, at most one node on the active path binds the trigger, so the first match is the only one and the descent order does not change the result; the descent only has to visit every node on the path. There is no upward pass.

```rust
// generated on the Nav leaf:
fn dispatch(path: Path<Nav, ParentPath>, fired: &MercuryTrigger, event: MercuryEvent)
    -> Result<MercuryOutput, DispatchError>
{
    if *fired == Keyboard::new("g").into() {
        let ev = event.try_into().map_err(|_| DispatchError::WrongEvent)?;
        return Ok(on_g(ev, path));
    }
    Err(DispatchError::NoHandler) // a leaf with no match
}

// generated on the Layer enum:
fn dispatch(mut path: Path<Layer, ParentPath>, fired: &MercuryTrigger, event: MercuryEvent)
    -> Result<MercuryOutput, DispatchError>
{
    if *fired == Keyboard::new("f1").into() {
        let ev = event.try_into().map_err(|_| DispatchError::WrongEvent)?;
        return Ok(show_help(ev, path));
    }
    match path.get_mut() {               // descend into the active variant
        Layer::Nav(_)    => Nav::dispatch(nav_child(path), fired, event),
        Layer::Typing(_) => Typing::dispatch(typing_child(path), fired, event),
    }
}
```

`nav_child` and `typing_child` build the child path with laserbeam's `Path::from_fn`, as `resolve` does.

## The path

The handler needs its node's typed `Path<Node, Parent>`. Dispatch builds it while descending, the same construction `resolve` performs, and hands the matching node its path. So dispatch, unlike accumulate, constructs paths, and the descent duplicates laserbeam's resolve walk. The bind derive either reuses laserbeam's descent or regenerates it, the same open choice accumulate has for the active-variant match.

## Errors

```rust
enum DispatchError {
    NoHandler,  // no node on the active path binds the fired trigger
    WrongEvent, // the event's variant did not match the trigger's source
}
```
