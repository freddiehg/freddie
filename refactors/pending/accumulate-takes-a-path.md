# `accumulate` takes a path

Not done. Forced by `resolution.md`, but independently true.

## What `accumulate` is

```rust
pub trait EventHandler<M: Bindings> {
    fn accumulate(&self, out: &mut HashSet<M::Trigger>) -> Result<(), BindError>;
}
```

It walks the active tree and collects every live bind's trigger into a set, erroring on a collision. `duplicate_trigger_is_error` in `crates/bind/tests/accumulate.rs` asserts it.

## It is the clobber check and nothing else

Its documented purpose is "the active trigger set (what the app registers with the OS)". No such registration exists. `CGEventTap::with_enabled` in `crates/freddie_keyboard/src/sys/macos.rs` subscribes to event TYPES, `KeyDown`, `KeyUp`, and `FlagsChanged`. Every key event arrives and the model decides per event. Nothing is registered per key.

It has no callers outside `bind`'s own tests. `crates/mercury`, `crates/freddie_main_loop`, and `crates/freddie_keyboard` never touch it.

What it actually produces is `insert_or_error` returning `DuplicateTrigger`. That is `no-clobber.md`.

## `&self` cannot run a derived child fn

A derived child fn is `fn(&Parent) -> Option<Data>` and needs a path. `accumulate(&self)` has none, so a derived level's binds never enter the trigger set, and completeness is the one thing a trigger set exists for. A check that cannot see a node's binds is worthless for that node.

## The change

Take a path and hand it back, exactly as `dispatch` does, and for the same reason: a node that has descended still has its own triggers to insert.

```rust
pub trait EventHandler<M: Bindings>: ::laserbeam::Resolve {
    fn accumulate<'a>(
        path: Self::Path<'a>,
        out: &mut HashSet<M::Trigger>,
    ) -> Result<Self::Path<'a>, BindError>
    where
        Self: 'a;
}
```

`bind_macro`'s `accumulate_body` then descends through the same `derive_support::Edge` that `dispatch_body` uses, so the two walks are structurally identical and cannot drift.

Compiled. Every derived impl in the workspace built unchanged under that signature; only the four callers of `bind::accumulate` moved.

## It should not ship

It is a test. Behind a `check` feature, on by default, with mercury's binary and `freddie_keys` taking `bind` with `default-features = false`. Mercury's dev-dependency turns it on for tests, and with resolver 3 that does not leak into the normal build.

A derive cannot see the features of the crate it expands into, so `bind` exports a `check_only!` macro, compiled with `bind`'s own features, which keeps or drops the `EventHandler` impl.

Compiled, and verified by expansion rather than assertion:

```
cargo expand -p mercury --lib          0 EventHandler impls, 10 Dispatch
cargo expand -p mercury --lib --tests  10 EventHandler impls
```

## The wart

`Bindings::Trigger` cannot be cfg'd away. A consumer implements `Bindings` and cannot see `bind`'s features, so an associated type that came and went would not compile for them. It stays in the trait unconditionally even though only the check uses it.

## Order

Independent of `resolution.md` and can land first. `resolution.md` needs it, because a derived level's binds are invisible to the check without it.
