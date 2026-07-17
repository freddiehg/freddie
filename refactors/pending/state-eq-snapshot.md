# state Eq, snapshot comparison in tests

The state tree (`Mercury` and every node under it) derives `PartialEq, Eq` behind the `testing`
feature, and the per-event tests assert the whole post-dispatch state with one `assert_eq!` instead
of a partial `matches!` on `layer()`.

## Why

`MercuryEffect` already gates effect equality behind `testing` (`crates/mercury/src/effect.rs`), so
a test asserts exactly what a dispatch produced. The state half has no such equality: a test can
only reach for `matches!(m.layer(), Layer::Nav(_))`, which checks the active variant and nothing
else. A transition that also touches `held` or `foreground` goes unchecked, and the assertion does
not read as "the state is now exactly this".

With `Eq` on the tree, a per-event test snapshots the full state:

```rust
assert_eq!(m, Mercury::with_layer(Layer::Nav(NavLayer {})));
```

which pins the layer AND that nothing else moved, in one line, and doubles as documentation of the
resulting state the way the effect table already does.

The gate is `testing`, not an unconditional derive, for the same reason as effects: dispatch never
compares states, so the normal build has no use for the impls, and resolver 3 keeps a
dev-dependency's features out of `cargo build -p mercury` (`crates/mercury/Cargo.toml` already wires
`mercury = { path = ".", features = ["testing"] }` as a dev-dependency).

## Change 1: gate `PartialEq, Eq` on the state tree

Every node and every field type it contains gets `#[cfg_attr(feature = "testing", derive(PartialEq,
Eq))]`. `App` (`crates/mercury/src/sources.rs`) already derives `PartialEq, Eq` unconditionally, so
it is untouched and satisfies the leaf requirement for `Foreground`.

All changes are in `crates/mercury/src/state.rs`.

`Mercury`:

```rust
// before
#[derive(Bind, Debug)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(
    Foregrounded => on_foregrounded,
    Quit => on_quit,
    AnyModifierKey => on_modifier,
    AnyNonModifierKey => maybe_pass_through,
)]
pub struct Mercury {

// after
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Bind, Debug)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(
    Foregrounded => on_foregrounded,
    Quit => on_quit,
    AnyModifierKey => on_modifier,
    AnyNonModifierKey => maybe_pass_through,
)]
pub struct Mercury {
```

`Foreground`:

```rust
// before
#[derive(Debug, Default, Clone, Copy)]
pub struct Foreground {

// after
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug, Default, Clone, Copy)]
pub struct Foreground {
```

`Layer`:

```rust
// before
#[derive(Bind, Debug, derive_more::From)]
#[node(parent = MercuryPath)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => to_home)]
pub enum Layer {

// after
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Bind, Debug, derive_more::From)]
#[node(parent = MercuryPath)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => to_home)]
pub enum Layer {
```

The layer nodes `HomeLayer`, `NavLayer`, `ResizeLayer`, `TypingLayer`, `AppLayer`, the derived
nodes `AppData`, `ChromeApp`, `GhosttyApp`, and the modifier types `LeftRightPair`, `HeldModifiers`
each get the same attribute line above their existing `#[derive(..)]`. For example `AppLayer`:

```rust
// before
#[derive(Bind, Debug, Default)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyT.down() => to_typing,
)]
pub struct AppLayer {}

// after
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Bind, Debug, Default)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyT.down() => to_typing,
)]
pub struct AppLayer {}
```

`HeldModifiers` keeps its hand-written `Debug` impl; the attribute adds only `PartialEq, Eq`:

```rust
// before
#[derive(Default, Clone, Copy)]
pub struct HeldModifiers {

// after
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Default, Clone, Copy)]
pub struct HeldModifiers {
```

The complete list of types that get the attribute, in `state.rs`:

- `Mercury`
- `Foreground`
- `Layer`
- `HomeLayer`, `NavLayer`, `ResizeLayer`, `TypingLayer`, `AppLayer`
- `AppData`, `ChromeApp`, `GhosttyApp`
- `LeftRightPair`, `HeldModifiers`

`Side` is not part of the state (it is an argument to `set`), so it is not derived.

## Change 2: snapshot the state in per-event tests

In `crates/mercury/tests/transitions.rs`, each per-event test that currently checks the layer with
`matches!` asserts the full post-dispatch state with `assert_eq!` against a constructed `Mercury`.
The effect assertion on the same event is unchanged.

The expected state is built with the existing constructors: `Mercury::default()` (Typing) and
`Mercury::with_layer(layer)` for any other layer, since a fresh `home()` carries default
`foreground` and `held`, and these transitions only move the layer.

`home_n_enters_nav`:

```rust
// before
#[test]
fn home_n_enters_nav() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::KeyN)), Some(vec![]));
    assert!(matches!(m.layer(), Layer::Nav(_)));
}

// after
#[test]
fn home_n_enters_nav() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::KeyN)), Some(vec![]));
    assert_eq!(m, Mercury::with_layer(Layer::Nav(NavLayer {})));
}
```

`home_t_enters_typing`:

```rust
// before
#[test]
fn home_t_enters_typing() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::KeyT)), Some(vec![]));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

// after
#[test]
fn home_t_enters_typing() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::KeyT)), Some(vec![]));
    assert_eq!(m, Mercury::default());
}
```

`quit_event_kills_from_home`, where the point is that quit is an effect and leaves the state alone:

```rust
// before
#[test]
fn quit_event_kills_from_home() {
    let mut m = home();
    assert_eq!(m.handle(&quit_event()), Some(vec![MercuryEffect::Kill]));
    // No layer change: quit is an effect, not a transition.
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// after
#[test]
fn quit_event_kills_from_home() {
    let mut m = home();
    let before = home();
    assert_eq!(m.handle(&quit_event()), Some(vec![MercuryEffect::Kill]));
    // Quit is an effect, not a transition: the state is untouched.
    assert_eq!(m, before);
}
```

`default_boots_into_typing` stays a `matches!` on `Mercury::default().layer()`: it asserts a
property of one field of the default, not a comparison of two states, and rewriting it as
`assert_eq!(Mercury::default(), Mercury::default())` would assert nothing.

Every other per-event test in the file that ends in a `matches!(m.layer(), ..)` gets the same
treatment: replace it with `assert_eq!(m, <expected>)`, where `<expected>` is `Mercury::default()`
for a Typing result or `Mercury::with_layer(Layer::<Variant>(<Node> {}))` otherwise. Where a test
constructs a starting state other than `home()`, build the expected end state from that same
starting state so `foreground` and `held` match.

The exports the tests need (`NavLayer`, `ResizeLayer`, etc.) are already re-exported from
`crates/mercury/src/lib.rs`.
