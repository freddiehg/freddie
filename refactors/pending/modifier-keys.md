# modifier keys

## The problem

A chord like cmd-g is tempting to model as a single trigger (`Keyboard::new("cmd+g")`), but that makes modifiers special and bakes key combinations into trigger strings. We do not want cmd, shift, ctrl, or opt to be special.

A modifier is an ordinary key whose down and up are state transitions:

- cmd down -> enter the cmd-held state
- g (while cmd is held) -> cmd-g behavior
- cmd up -> return to the nothing-held state

This matches the "special keys are not special" stance in `overall-plan.md`: cmd down is a transition `A -> B`, cmd up is `B -> A`. This is orthogonal to bind. bind just routes `(Keyboard, down/up)` triggers to handlers; the question here is how to model the held-modifier state so a handler can branch on it cleanly, without bind or laserbeam knowing what a modifier is.

## WithModifierKeys<T>

A generic wrapper that augments a node `T` with modifier-key state:

```rust
struct WithModifierKeys<T> {
    held: ModifierSet, // which modifiers are currently down: cmd, shift, ctrl, opt, ..
    inner: T,
}
```

- It binds the modifier keys itself: cmd down sets cmd in `held`, cmd up clears it. So any `T` wrapped in it gets modifier tracking, and neither `T` nor bind nor laserbeam special-cases modifiers. The modifier keys are just keys whose handlers mutate `held`.
- A handler on `T` (or below it) reads the modifier status by walking up the path to the `WithModifierKeys` node and reading `held`. So g's handler does: if `held.contains(cmd)`, run cmd-g, else plain g. The branch is ordinary code over ordinary state, not a chord matched at the trigger level.
- `WithModifierKeys` is reusable machinery, not a primitive. It is a laserbeam node like any other, holding state (`held`) and nesting `inner`.

## Initialization with the current held state

When a `WithModifierKeys` is constructed (at startup, or on entering a layer that wraps in it), it cannot assume nothing is held. If cmd is already down when the wrapper comes into existence, `held` must reflect that, or a cmd that went down before the wrapper existed is invisible and the next cmd up desyncs `held` from reality.

So construction seeds `held` from what is currently held down, queried from the OS (or threaded from the outgoing state on a layer switch). The initial `held` is whatever is physically pressed at construction time.

## Open questions

- Where `held` lives relative to the active path: one `WithModifierKeys` near the root (global modifier state) vs per-layer wrappers. Global is simpler; per-layer allows different modifier semantics per layer but risks desync across switches.
- Per-keyboard modifiers. With concurrent laptop and external keyboards (see `laserbeam-missing-features.md`), modifiers are per-device, so `held` may need keying by device, or a `WithModifierKeys` per keyboard layer.
- How `Keyboard` triggers encode direction: down vs up must be distinguishable, so the keyboard trigger carries press/release, not just the key.
- Querying the current physical modifier state at construction: the API per platform.
- Reconciling `held` with reality after focus loss or a missed up event (stuck modifiers), and auto-repeat.
