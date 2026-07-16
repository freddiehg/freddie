# handlers take the source event by value

A handler is `fn handler(ev: &KeyEvent, node: Node<...>) -> Vec<MercuryEffect>`. This change makes it `fn handler(ev: KeyEvent, node: Node<...>)`: the source event arrives owned, so a straight passthrough moves it into an effect (`Emit(ev)`) instead of reading its fields and rebuilding an identical event.

Dispatch offers one event to a chain of triggers and runs exactly one handler (the leafward-most match). Today the event is a shared borrow threaded through that walk. To hand the winner an owned event, the walk threads the unified event by value: each node borrows it (`&event`) for the type-and-key match, and on a miss hands it back alongside the path so the parent takes its turn; on a match it moves the event into the handler. The `?` on the child descent already carries a `ControlFlow`, so threading the event on a miss is one extra binding in the tuple it hands back.

The source narrowing is the enabling piece: `MercuryEvent` already gives `TryFrom<&MercuryEvent> for &SourceEvent` (the borrow used by the match); it gains `TryFrom<MercuryEvent> for SourceEvent` (the owned move used at handoff).

## change 1 (prefactor, ships alone): owned narrowing on `MercuryEvent`

Add the owned `TryInto` alongside the borrowing one. Additive: the new impls are unused until change 2, so this compiles and ships on its own.

`crates/mercury/src/model.rs`, before:

```rust
#[derive(Debug, derive_more::TryInto)]
#[try_into(ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(QuitEvent),
}
```

after:

```rust
#[derive(Debug, derive_more::TryInto)]
#[try_into(owned, ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(QuitEvent),
}
```

`#[try_into(owned, ref)]` emits both `TryFrom<MercuryEvent> for T` and `TryFrom<&MercuryEvent> for &T` for each variant's `T`.

## the atomic change

Changes 2 through 5 are one cross-crate signature change: `bind`'s traits, `bind_macro`'s generated bodies, every handler, and every driver compile only together. They are presented in parts, but they land as one commit.

## change 2: thread the owned event through `bind`'s traits

Every dispatch signature that takes `event: &M::Event` takes `event: M::Event`, and every `ControlFlow::Continue` payload that was a path or parent becomes `(that, M::Event)`, so a miss hands the event back with the path. `EventTrigger::is_matching` is unchanged: the match still borrows.

`crates/bind/src/lib.rs`.

Top-level `dispatch`, before:

```rust
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    match <N as Dispatch<M>>::dispatch(path, event) {
        ControlFlow::Break(out) => Some(out),
        ControlFlow::Continue(_) => None,
    }
}
```

after:

```rust
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: M::Event) -> Option<M::Output>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    match <N as Dispatch<M>>::dispatch(path, event) {
        ControlFlow::Break(out) => Some(out),
        ControlFlow::Continue(_) => None, // total miss: the event is dropped here
    }
}
```

`Dispatch::dispatch`, before:

```rust
fn dispatch<'a>(
    path: Self::Path<'a>,
    event: &M::Event,
) -> ControlFlow<M::Output, Self::Path<'a>>
where
    Self: 'a;
```

after:

```rust
fn dispatch<'a>(
    path: Self::Path<'a>,
    event: M::Event,
) -> ControlFlow<M::Output, (Self::Path<'a>, M::Event)>
where
    Self: 'a;
```

`Descend::dispatch`, before:

```rust
fn dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>;
```

after:

```rust
fn dispatch(self, event: M::Event) -> ControlFlow<M::Output, (Self::Parent, M::Event)>;
```

`SimpleRunner`, before:

```rust
pub fn next(&mut self) -> Option<Option<M::Output>> {
    let event = self.queue.pop_front()?;
    Some(dispatch::<M, N>(&mut *self.root, &event))
}

pub fn process_event(&mut self, event: M::Event) -> Option<M::Output> {
    self.queue.push_back(event);
    let event = self
        .queue
        .pop_front()
        .expect("the queue is non-empty: an event was just queued");
    dispatch::<M, N>(&mut *self.root, &event)
}
```

after (both already own the popped event; pass it by value):

```rust
pub fn next(&mut self) -> Option<Option<M::Output>> {
    let event = self.queue.pop_front()?;
    Some(dispatch::<M, N>(&mut *self.root, event))
}

pub fn process_event(&mut self, event: M::Event) -> Option<M::Output> {
    self.queue.push_back(event);
    let event = self
        .queue
        .pop_front()
        .expect("the queue is non-empty: an event was just queued");
    dispatch::<M, N>(&mut *self.root, event)
}
```

## change 3: `bind_macro` moves the event into the matched arm

The generated dispatch bodies change by one uniform rule, applied everywhere the macro emits them: a per-bind check narrows on `&event` to select, then moves `event` into the handler on a match; the child descent destructures the event back out of its `?`; and the fallthrough `Continue` carries `(path_or_parent, event)`.

`crates/bind_macro/src/lib.rs`.

The per-bind check (`dispatch_impl`'s `checks`, and the identical one in `derived_node_impl`), before:

```rust
if let ::core::option::Option::Some(ev) =
    ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
{
    let trigger = #trigger;
    if ::bind::EventTrigger::is_matching(&trigger, ev) {
        return ::core::ops::ControlFlow::Break(#handler(
            ev,
            ::bind::Node { parent: path, data: () },
        ));
    }
}
```

after:

```rust
{
    let trigger = #trigger;
    // Select on a borrow; `event` is untouched on a miss.
    let matched = ::core::result::Result::ok(
        ::core::convert::TryFrom::try_from(&event),
    )
    .is_some_and(|ev| ::bind::EventTrigger::is_matching(&trigger, ev));
    if matched {
        // The borrow confirmed this is the source's variant, so the owned
        // narrow succeeds; it moves `event` into the handler.
        let ev = ::core::result::Result::unwrap(
            ::core::convert::TryFrom::try_from(event),
        );
        return ::core::ops::ControlFlow::Break(#handler(
            ev,
            ::bind::Node { parent: path, data: () },
        ));
    }
}
```

The handler pins the owned source type, and `is_matching` pins the borrowed one; a trigger whose source does not match its handler's parameter still fails to compile, exactly as today. The `derived_node_impl` check is the same, with its node built as `node` instead of `Node { parent: path, data: () }`.

The child descent (`dispatch_body`, the struct and enum arms), before:

```rust
let child = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event)?;
path = #recover;
```

after (the `?` yields the `Continue` payload, now a tuple; rebind `event` from it):

```rust
let (child, event) = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event)?;
path = #recover;
```

The place `Dispatch::dispatch` tail (`dispatch_impl`), before:

```rust
#recurse
#(#checks)*
::core::ops::ControlFlow::Continue(path)
```

after:

```rust
#recurse
#(#checks)*
::core::ops::ControlFlow::Continue((path, event))
```

The place `Descend` that delegates to `Dispatch` (`descend_impl`), before:

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event) {
    ::core::ops::ControlFlow::Break(out) => ::core::ops::ControlFlow::Break(out),
    ::core::ops::ControlFlow::Continue(path) => {
        ::core::ops::ControlFlow::Continue(::bind::HasParent::into_parent(path))
    }
}
```

after:

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event) {
    ::core::ops::ControlFlow::Break(out) => ::core::ops::ControlFlow::Break(out),
    ::core::ops::ControlFlow::Continue((path, event)) => {
        ::core::ops::ControlFlow::Continue((::bind::HasParent::into_parent(path), event))
    }
}
```

The derived node's fallthrough (`derived_node_impl`), before:

```rust
::core::ops::ControlFlow::Continue(::bind::HasParent::into_parent(node))
```

after:

```rust
::core::ops::ControlFlow::Continue((::bind::HasParent::into_parent(node), event))
```

The derived enum node's arms (`derived_enum_node_impl`) already forward `event` into the variant child's `Descend::dispatch`; with `event` owned, each arm moves it, which is sound because the arms are mutually exclusive. The signatures on those two generated `Descend`/`Dispatch` impls pick up the new `event: M::Event` parameter and `(_, M::Event)` `Continue` payload from the same edits above.

The `accumulate`/check half (`EventHandler`, `DerivedHandler`, the `#[cfg(feature = "check")]` bodies) is untouched: it walks triggers, never the event.

## change 4: handlers take the event by value

Every handler's first parameter drops the `&`. The passthrough handlers stop rebuilding the event and move it.

`crates/mercury/src/handlers/root.rs`, before:

```rust
pub(crate) fn maybe_pass_through(ev: &KeyEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    if root.layer.is_passthrough() {
        vec![emit(ev.key, ev.press, ev.flags)]
    } else {
        Vec::new()
    }
}
```

after:

```rust
pub(crate) fn maybe_pass_through(ev: KeyEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    if root.layer.is_passthrough() {
        vec![MercuryEffect::Emit(ev)]
    } else {
        Vec::new()
    }
}
```

`on_modifier` keeps reading `ev` for `HeldModifiers::apply` (which takes `&KeyEvent`, so it borrows `&ev`) and then moves it into `Emit(ev)` on the passthrough branch.

The handlers that ignore the event drop the `&` on the `_ev` binding, e.g. `crates/mercury/src/handlers/quit.rs`:

```rust
pub(crate) fn on_quit(_ev: QuitEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
```

The full list to change the signature of: `on_modifier`, `maybe_pass_through` (`root.rs`); `maybe_go_home` (`typing.rs`); `on_foregrounded` (`foreground.rs`, `ev: ForegroundEvent`); `on_quit` (`quit.rs`, `_ev: QuitEvent`); and the layer/app handlers binding `_ev` in `home.rs`, `nav.rs`, `resize.rs`, `app.rs` (`_ev: KeyEvent`).

## change 5: the drivers pass the event by value

`Mercury::handle` and the two call sites that dispatch a live event own it already; they stop borrowing.

`crates/mercury/src/state.rs`, before:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

after:

```rust
pub fn handle(&mut self, event: MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

`crates/mercury/src/main.rs`, `dispatch_event`. `handle` now consumes the event, and the per-dispatch record still wants the event, the effects, and the state on one line (`Logs` standard), so the event is rendered before the move.

before:

```rust
fn dispatch_event(
    state: &mut Mercury,
    event: &MercuryEvent,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let effects = state.handle(event).unwrap_or_default();
    info!(event = ?event, effects = ?effects, state = ?state, "dispatch");
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
}
```

after:

```rust
fn dispatch_event(
    state: &mut Mercury,
    event: MercuryEvent,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let rendered = format!("{event:?}");
    let effects = state.handle(event).unwrap_or_default();
    info!(event = %rendered, effects = ?effects, state = ?state, "dispatch");
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
}
```

Its caller (`run_event_loop`) already binds an owned `event` off the channel, so it passes `event` in place of `&event`.

Tests drive the tree through `SimpleRunner` (change 2), so `tests/transitions.rs` needs no event-borrow edits; it constructs `MercuryEvent`s and queues them by value as it does now.
