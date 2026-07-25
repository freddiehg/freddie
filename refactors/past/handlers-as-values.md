# handlers as values

A trigger in a bind is a value: `Key("c")`. A handler is a free function: `#[bind(Key("c") => open_chrome)]`, where `open_chrome` is `fn(&KeyEvent, NavLayerPath) -> Vec<MercuryEffect>`. The two sides of the arrow are written differently, and every handler is its own named function even when it does something uniform.

The idea is to let the handler be a value too, built the way the trigger is:

```rust
#[bind(Key("c") => OpenChrome)]
#[bind(Key("r") => Command("r"))]
```

`Command("r")` is a value that, when run, returns `vec![MercuryEffect::Command("r")]`, so the per-app `command` function and the four identical `open_*` functions collapse into values the binding constructs inline.

## What a value handler is

A handler value produces the marker's `Output` from the event and the node's path. That is a trait:

```rust
trait Handler<M: Bindings> {
    fn run(self, event: &SourceEvent, path: NodePath) -> M::Output;
}
```

The generated dispatch calls `handler.run(ev, path)` instead of `handler(ev, path)`. A free function can still be a handler (a blanket impl over `Fn`), so both forms coexist. The source-event type is pinned the same way it is now, by `Handler::run`'s signature rather than the function's.

## The key-remap angle

Once a handler is a value it can carry more than a command. A value handler for a key could hold both the command it sends and a key it remaps to, so a single binding expresses "this key sends `cmd`+`r` and also types `x`", instead of a hand-written function that builds both effects. The trigger stays the physical key; the handler value carries the intent.

## Open

- Whether the handler trait takes the path by value (like the function does now) and how it names the node's path type generically.
- Whether `Output` construction stays `Vec<Effect>` or a value handler can return a single effect that the derive wraps.
- How a value handler that needs the event's data (the typed key) reads it, versus one that is a constant (`OpenChrome`).
