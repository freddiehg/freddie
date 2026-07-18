# handlers push into an effect sink

A handler is `fn(&SourceEvent, Node<..>) -> Vec<MercuryEffect>`, so every handler allocates its own vector and every caller that has effects of its own merges vectors:

```rust
let mut effects = root.set_layer(inapp); // one Vec
effects.push(timer);                     // grows it
effects.push(MercuryEffect::Foreground(app));
effects
```

This change threads one `&mut Vec<MercuryEffect>` from the event loop down through dispatch into the handler. A handler pushes and returns nothing, the merges disappear, and the event loop reuses a single buffer across events, so a dispatched key allocates nothing in the steady state.

Dispatch runs exactly one handler (the leafward-most match), so the sink is written by one handler per event and there is no partial-write to unwind on a miss.

Today, per dispatched key: `j` in Ghostty allocates the `tmux` vector and reallocates it for the appended timer; a passed-through key allocates a one-element vector; `n c` allocates for the timer and reallocates for the foreground effect. After the change, all of those push into a buffer that `run_event_loop` owns and `drain`s, which keeps its capacity.

`Bindings::Output` is what makes the handler return a collection, so it goes; the marker names the effect TYPE instead, and dispatch reports only whether a binding claimed the event.

## change 1 (ships alone): mercury's effect producers take a sink

The mercury-internal helpers that build effects take `out: &mut Vec<MercuryEffect>` and push. Handlers still return `Vec<MercuryEffect>`, so `bind` is untouched and the tests are untouched; each handler makes the vector it used to receive from a helper.

`crates/mercury/src/state/mod.rs`, `Mercury::set_layer`, before:

```rust
#[must_use = "the returned flush has to be emitted, or a held modifier is stranded down"]
pub fn set_layer(&mut self, into: impl Into<Layer>) -> Vec<MercuryEffect> {
    let into = into.into();
    let before_passthrough = self.layer.is_passthrough();
    let after_passthrough = into.is_passthrough();
    self.layer = into;
    match (before_passthrough, after_passthrough) {
        (true, false) => self.typing_state.held.close(),
        (false, true) => self.typing_state.held.open(),
        _ => Vec::new(),
    }
}
```

after (the flush is pushed where it is produced, so there is no returned value to forget and no `#[must_use]` to carry the warning):

```rust
pub fn set_layer(&mut self, into: impl Into<Layer>, out: &mut Vec<MercuryEffect>) {
    let into = into.into();
    let before_passthrough = self.layer.is_passthrough();
    let after_passthrough = into.is_passthrough();
    self.layer = into;
    match (before_passthrough, after_passthrough) {
        (true, false) => self.typing_state.held.close(out),
        (false, true) => self.typing_state.held.open(out),
        _ => {}
    }
}
```

`HeldModifiers`, before:

```rust
#[must_use]
pub fn open(self) -> Vec<MercuryEffect> {
    self.emit_synchronization_events(PressType::Down)
}

#[must_use]
pub fn close(self) -> Vec<MercuryEffect> {
    self.emit_synchronization_events(PressType::Up)
}

fn emit_synchronization_events(self, press: PressType) -> Vec<MercuryEffect> {
    let mut shown = if press == PressType::Down {
        Self::default()
    } else {
        self
    };
    let mut out = Vec::new();
    for key in self.held_keys() {
        shown.apply(&KeyEvent {
            key,
            press,
            flags: ModifierFlags::empty(),
        });
        out.push(emit(key, press, shown.flags()));
    }
    out
}
```

after:

```rust
pub fn open(self, out: &mut Vec<MercuryEffect>) {
    self.emit_synchronization_events(PressType::Down, out);
}

pub fn close(self, out: &mut Vec<MercuryEffect>) {
    self.emit_synchronization_events(PressType::Up, out);
}

fn emit_synchronization_events(self, press: PressType, out: &mut Vec<MercuryEffect>) {
    let mut shown = if press == PressType::Down {
        Self::default()
    } else {
        self
    };
    for key in self.held_keys() {
        shown.apply(&KeyEvent {
            key,
            press,
            flags: ModifierFlags::empty(),
        });
        out.push(emit(key, press, shown.flags()));
    }
}
```

`crates/mercury/src/handlers/mod.rs`. `and_go_home` existed to merge the caller's vector with `go_home`'s; with a sink it would be `go_home` with an ascent, so it goes and its callers call `go_home` directly. The chooser policy it documented moves onto `go_home`.

before:

```rust
/// Go to the home layer, returning the modifier flush (empty unless leaving a passthrough layer).
/// The one place the home layer is entered.
pub(crate) fn go_home(root: &mut Mercury) -> Vec<MercuryEffect> {
    root.set_layer(HomeLayer::new())
}

/// Ask for `effects`, then return home.
///
/// A layer stays only if its actions make sense to do repeatedly. Walking tmux's panes and
/// refreshing Chrome do, so the in-app layers stay. Placing a window does not: repeating it is
/// a no-op, and anything else is a different choice. So resize is a one-shot chooser, and this
/// is how it leaves. (Nav also leaves after one choice, but into the in-app layer rather than
/// home; see [`super::nav`].)
///
/// Generic over the path, so every chooser binds it from its own node.
pub(crate) fn and_go_home<'a, P: Ascend<MercuryPath<'a>>>(
    path: P,
    mut effects: Vec<MercuryEffect>,
) -> Vec<MercuryEffect> {
    effects.extend(go_home(path.ascend()));
    effects
}
```

after:

```rust
/// Go to the home layer, pushing the modifier flush (nothing unless leaving a passthrough layer).
/// The one place the home layer is entered.
///
/// A layer stays only if its actions make sense to do repeatedly. Walking tmux's panes and
/// refreshing Chrome do, so the in-app layers stay. Placing a window does not: repeating it is
/// a no-op, and anything else is a different choice. So resize is a one-shot chooser, and this
/// is how it leaves. (Nav also leaves after one choice, but into the in-app layer rather than
/// home; see [`super::nav`].)
pub(crate) fn go_home(root: &mut Mercury, out: &mut Vec<MercuryEffect>) {
    root.set_layer(HomeLayer::new(), out);
}
```

`go_home` takes the root directly, so `Ascend` and `MercuryPath` are unused in that module once `and_go_home` goes: the `use` list becomes `use crate::MercuryEffect;` and `use crate::state::{HomeLayer, Mercury};`. `app.rs` and `resize.rs` import `super::go_home` in place of `super::and_go_home`, and both already import `laserbeam::Ascend` for the ascent their handlers now do themselves.

`crates/mercury/src/handlers/app.rs`, `tmux` and one of the `select_window!` bodies, before:

```rust
fn tmux(flags: ModifierFlags, command: Key) -> Vec<MercuryEffect> {
    vec![tap(Key::KeyA, ModifierFlags::CONTROL), tap(command, flags)]
}

macro_rules! select_window {
    ($($handler:ident => $digit:ident),* $(,)?) => {$(
        pub(crate) fn $handler<'a, E, P: Ascend<MercuryPath<'a>>, D>(
            _ev: &E,
            node: Node<P, D>,
        ) -> Vec<MercuryEffect> {
            and_go_home(node.parent, tmux(ModifierFlags::SHIFT, Key::$digit))
        }
    )*};
}
```

after (still returning a `Vec` from the handler, which change 2 removes):

```rust
fn tmux(flags: ModifierFlags, command: Key, out: &mut Vec<MercuryEffect>) {
    out.push(tap(Key::KeyA, ModifierFlags::CONTROL));
    out.push(tap(command, flags));
}

macro_rules! select_window {
    ($($handler:ident => $digit:ident),* $(,)?) => {$(
        pub(crate) fn $handler<'a, E, P: Ascend<MercuryPath<'a>>, D>(
            _ev: &E,
            node: Node<P, D>,
        ) -> Vec<MercuryEffect> {
            let mut out = Vec::new();
            tmux(ModifierFlags::SHIFT, Key::$digit, &mut out);
            go_home(node.parent.ascend(), &mut out);
            out
        }
    )*};
}
```

`previous_window` and `next_window` likewise build a local and hand it back:

```rust
pub(crate) fn previous_window<E, N>(_ev: &E, _node: N) -> Vec<MercuryEffect> {
    let mut out = Vec::new();
    tmux(ModifierFlags::empty(), Key::KeyP, &mut out);
    out
}
```

`crates/mercury/src/handlers/resize.rs`, before:

```rust
pub(crate) fn maximize<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Place(Placement::Maximize)])
}
```

after:

```rust
pub(crate) fn maximize<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let mut out = vec![MercuryEffect::Place(Placement::Maximize)];
    go_home(node.parent.ascend(), &mut out);
    out
}
```

`left_half` and `right_half` are the same with `Placement::LeftHalf` and `Placement::RightHalf`.

The remaining handlers that call `set_layer`, `go_home`, `open` or `close` build a local `Vec` and pass `&mut` to them, then return it. `crates/mercury/src/handlers/nav.rs`, before:

```rust
fn navigate<'a, P: Ascend<MercuryPath<'a>>>(path: P, app: App) -> Vec<MercuryEffect> {
    let root: MercuryPath<'_> = path.ascend();
    root.foreground.start_navigating();
    let (inapp, timer) = AppLayer::new();
    let mut effects = root.set_layer(inapp);
    effects.push(timer);
    effects.push(MercuryEffect::Foreground(app));
    effects
}
```

after:

```rust
fn navigate<'a, P: Ascend<MercuryPath<'a>>>(path: P, app: App) -> Vec<MercuryEffect> {
    let root: MercuryPath<'_> = path.ascend();
    root.foreground.start_navigating();
    let (inapp, timer) = AppLayer::new();
    let mut out = Vec::new();
    root.set_layer(inapp, &mut out);
    out.push(timer);
    out.push(MercuryEffect::Foreground(app));
    out
}
```

The same shape applies to `to_home`, `to_nav`, `to_typing`, `to_inapp`, `to_resize` (`home.rs`), `maybe_go_home` (`typing.rs`) and `quit` (`quit.rs`), each of which today receives a `Vec` from `set_layer`/`go_home`/`open` and appends to it.

The layer constructors keep handing their timer effect back as a value (`AppLayer::new() -> (Self, MercuryEffect)`, and the same for `NavLayer`, `ResizeLayer`), so the handler places it after the flush and the effect order is unchanged.

## the atomic change

Change 2 is one cross-crate signature change: `bind`'s traits, `bind_macro`'s generated bodies, mercury's handlers, and every driver and test compile only together. It is presented in parts and lands as one commit.

## change 2: dispatch threads the sink and reports only whether it matched

### `crates/bind/src/lib.rs`

`Bindings`, before:

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    /// What a handler returns: the effect data for the consumer to perform.
    type Output;
}
```

after:

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    /// One piece of effect data for the consumer to perform. A handler pushes into a
    /// `&mut Vec<Self::Effect>` rather than returning a collection of its own.
    type Effect;
}
```

The new answer dispatch gives, alongside it:

```rust
/// Whether a binding on the active path claimed the event. The effects, if any, are in the
/// sink the caller passed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dispatched {
    Handled,
    Unbound,
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
    event: &M::Event,
    out: &mut Vec<M::Effect>,
) -> ControlFlow<(), Self::Path<'a>>
where
    Self: 'a;
```

`Descend::dispatch`, before:

```rust
fn dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>;
```

after:

```rust
fn dispatch(self, event: &M::Event, out: &mut Vec<M::Effect>) -> ControlFlow<(), Self::Parent>;
```

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
pub fn dispatch<'a, M, N>(
    path: N::Path<'a>,
    event: &M::Event,
    out: &mut Vec<M::Effect>,
) -> Dispatched
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    match <N as Dispatch<M>>::dispatch(path, event, out) {
        ControlFlow::Break(()) => Dispatched::Handled,
        ControlFlow::Continue(_) => Dispatched::Unbound,
    }
}
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

after (the sink is a parameter, not a field, so a caller can queue follow-up events while reading the effects the last step produced):

```rust
/// Processes exactly one queued event, pushing its effects onto `out`. `None` means the queue
/// was empty; `Some` carries whether a binding claimed the event.
pub fn next(&mut self, out: &mut Vec<M::Effect>) -> Option<Dispatched> {
    let event = self.queue.pop_front()?;
    Some(dispatch::<M, N>(&mut *self.root, &event, out))
}

pub fn process_event(&mut self, event: M::Event, out: &mut Vec<M::Effect>) -> Dispatched {
    self.queue.push_back(event);
    let event = self
        .queue
        .pop_front()
        .expect("the queue is non-empty: an event was just queued");
    dispatch::<M, N>(&mut *self.root, &event, out)
}
```

The module doc's line about `Break` carrying the handler's output up becomes: `Break` says a binding claimed the event, whose effects are in the sink; `Continue` hands the node's path back so the parent can walk up and take its turn.

The `check` half (`EventHandler`, `DerivedHandler`, `accumulate`, `BindError`) is untouched: it walks triggers and never produces effects.

### `crates/bind_macro/src/lib.rs`

Four generated signatures name `<#marker as ::bind::Bindings>::Output` as the `Break` type (`derived_enum_node_impl`, `derived_node_impl`, `descend_impl`, `dispatch_impl`). Each becomes `()`, and each generated `dispatch` gains a third parameter.

The place `Dispatch` (`dispatch_impl`), before:

```rust
fn dispatch<'a>(
    #binding: <Self as ::bind::Place>::Path<'a>,
    event: &<#marker as ::bind::Bindings>::Event,
) -> ::core::ops::ControlFlow<
    <#marker as ::bind::Bindings>::Output,
    <Self as ::bind::Place>::Path<'a>,
>
where
    Self: 'a,
{
    #recurse
    #(#checks)*
    ::core::ops::ControlFlow::Continue(path)
}
```

after:

```rust
fn dispatch<'a>(
    #binding: <Self as ::bind::Place>::Path<'a>,
    event: &<#marker as ::bind::Bindings>::Event,
    out: &mut ::std::vec::Vec<<#marker as ::bind::Bindings>::Effect>,
) -> ::core::ops::ControlFlow<(), <Self as ::bind::Place>::Path<'a>>
where
    Self: 'a,
{
    #recurse
    #(#checks)*
    ::core::ops::ControlFlow::Continue(path)
}
```

The per-bind check (`dispatch_impl`'s `checks`), before:

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

after (the handler is a statement; `Break` carries nothing):

```rust
if let ::core::option::Option::Some(ev) =
    ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
{
    let trigger = #trigger;
    if ::bind::EventTrigger::is_matching(&trigger, ev) {
        #handler(ev, ::bind::Node { parent: path, data: () }, out);
        return ::core::ops::ControlFlow::Break(());
    }
}
```

The check in `derived_node_impl` is the same edit, with the node built as `node` rather than `Node { parent: path, data: () }`:

```rust
#handler(ev, node, out);
return ::core::ops::ControlFlow::Break(());
```

Every descent passes an explicit reborrow, because `out` is used again after the call:

`dispatch_body` (struct and enum arms), before:

```rust
let child = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event)?;
path = #recover;
```

after:

```rust
let child = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, &mut *out)?;
path = #recover;
```

`derived_child_descent`, before:

```rust
let #place = match #f(&#place) {
    ::core::option::Option::Some(data) => {
        ::bind::Descend::<#marker>::dispatch(
            ::bind::Node { parent: #place, data },
            event,
        )?
    }
    ::core::option::Option::None => #place,
};
```

after:

```rust
let #place = match #f(&#place) {
    ::core::option::Option::Some(data) => {
        ::bind::Descend::<#marker>::dispatch(
            ::bind::Node { parent: #place, data },
            event,
            &mut *out,
        )?
    }
    ::core::option::Option::None => #place,
};
```

`derived_enum_node_impl`'s dispatch arms, before:

```rust
#name::#vi(data) => ::bind::Descend::<#marker>::dispatch(
    ::bind::Node { parent, data },
    event,
),
```

after (the arms are the tail of the body, so the reborrow is not needed and `out` is forwarded):

```rust
#name::#vi(data) => ::bind::Descend::<#marker>::dispatch(
    ::bind::Node { parent, data },
    event,
    out,
),
```

`descend_impl`, before:

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event) {
    ::core::ops::ControlFlow::Break(out) => {
        ::core::ops::ControlFlow::Break(out)
    }
    ::core::ops::ControlFlow::Continue(path) => {
        ::core::ops::ControlFlow::Continue(
            ::bind::HasParent::into_parent(path),
        )
    }
}
```

after:

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event, out) {
    ::core::ops::ControlFlow::Break(()) => ::core::ops::ControlFlow::Break(()),
    ::core::ops::ControlFlow::Continue(path) => {
        ::core::ops::ControlFlow::Continue(
            ::bind::HasParent::into_parent(path),
        )
    }
}
```

### `crates/mercury`

`model.rs`, before:

```rust
impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
    type Event = MercuryEvent;
    type Output = Vec<MercuryEffect>;
}
```

after:

```rust
impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
    type Event = MercuryEvent;
    type Effect = MercuryEffect;
}
```

`state/mod.rs`, `Mercury::handle`, before:

```rust
#[must_use]
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    let before = std::mem::discriminant(&self.layer);
    let mut effects = bind::dispatch::<MercuryStruct, Self>(self, event)?;
    if matches!(event, MercuryEvent::Key(_))
        && std::mem::discriminant(&self.layer) == before
        && let Some(reset) = self.layer.rearm_timeout()
    {
        effects.push(reset);
    }
    Some(effects)
}
```

after (the `?` used to skip the rearm on an unbound event; the `Handled` guard does that now):

```rust
pub fn handle(&mut self, event: &MercuryEvent, out: &mut Vec<MercuryEffect>) -> Dispatched {
    let before = std::mem::discriminant(&self.layer);
    let dispatched = bind::dispatch::<MercuryStruct, Self>(self, event, out);
    // A keypress that stays in the in-app layer is activity: reset its return-home timer, so it
    // fires only after you go idle, not a fixed span after you entered.
    if matches!(dispatched, Dispatched::Handled)
        && matches!(event, MercuryEvent::Key(_))
        && std::mem::discriminant(&self.layer) == before
        && let Some(reset) = self.layer.rearm_timeout()
    {
        out.push(reset);
    }
    dispatched
}
```

`bind::Dispatched` is re-exported from `crates/mercury/src/lib.rs` alongside the model types, so a consumer that calls `handle` does not depend on `bind` directly:

```rust
pub use bind::Dispatched;
```

Every handler drops its return type and takes the sink as its third parameter. `handlers/mod.rs`'s module doc says the shape, before:

```rust
//! Each is a `fn(&SourceEvent, Node<OwnPath, ()>) -> Vec<MercuryEffect>`. It reaches the tree
//! through the path the node carries, and returns inert effects.
```

after:

```rust
//! Each is a `fn(&SourceEvent, Node<OwnPath, ()>, &mut Vec<MercuryEffect>)`. It reaches the
//! tree through the path the node carries, and pushes inert effects onto the sink.
```

`handlers/root.rs`, before:

```rust
pub(crate) fn maybe_pass_through(
    ev: &KeyEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent;
    if ev.key.is_modifier() {
        root.typing_state.held.apply(ev);
    }
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, ev.flags)]
    } else {
        Vec::new()
    }
}
```

after:

```rust
pub(crate) fn maybe_pass_through(
    ev: &KeyEvent,
    node: Node<&mut Mercury, ()>,
    out: &mut Vec<MercuryEffect>,
) {
    let root = node.parent;
    if ev.key.is_modifier() {
        root.typing_state.held.apply(ev);
    }
    if root.layer().is_passthrough() {
        out.push(emit(ev.key, ev.press, ev.flags));
    }
}
```

`handlers/nav.rs`'s `navigate`, which change 1 left building a local, before:

```rust
fn navigate<'a, P: Ascend<MercuryPath<'a>>>(path: P, app: App) -> Vec<MercuryEffect> {
    let root: MercuryPath<'_> = path.ascend();
    root.foreground.start_navigating();
    let (inapp, timer) = AppLayer::new();
    let mut out = Vec::new();
    root.set_layer(inapp, &mut out);
    out.push(timer);
    out.push(MercuryEffect::Foreground(app));
    out
}

pub(crate) fn open_chrome<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Chrome)
}
```

after:

```rust
fn navigate<'a, P: Ascend<MercuryPath<'a>>>(path: P, app: App, out: &mut Vec<MercuryEffect>) {
    let root: MercuryPath<'_> = path.ascend();
    root.foreground.start_navigating();
    let (inapp, timer) = AppLayer::new();
    root.set_layer(inapp, out);
    out.push(timer);
    out.push(MercuryEffect::Foreground(app));
}

pub(crate) fn open_chrome<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
    out: &mut Vec<MercuryEffect>,
) {
    navigate(node.parent, App::Chrome, out);
}
```

The same rule applies to every other handler: drop `-> Vec<MercuryEffect>`, take `out: &mut Vec<MercuryEffect>`, and delete the local vector that change 1 introduced. The full list is `record_front_app` (`foreground.rs`, which stays `const` and whose body becomes the one `set_front_app` call), `quit` (`quit.rs`), `to_home`, `to_nav`, `to_typing`, `to_inapp`, `to_resize` (`home.rs`), `open_chrome`, `open_finder`, `open_ghostty`, `open_zed` (`nav.rs`), `maximize`, `left_half`, `right_half` (`resize.rs`), `refresh`, `previous_window`, `next_window` and the `select_window!` bodies (`app.rs`), `maybe_go_home` (`typing.rs`), and `maybe_pass_through` (`root.rs`).

`handlers/app.rs`'s window handlers show the shape after the local goes:

```rust
pub(crate) fn previous_window<E, N>(_ev: &E, _node: N, out: &mut Vec<MercuryEffect>) {
    tmux(ModifierFlags::empty(), Key::KeyP, out);
}

macro_rules! select_window {
    ($($handler:ident => $digit:ident),* $(,)?) => {$(
        pub(crate) fn $handler<'a, E, P: Ascend<MercuryPath<'a>>, D>(
            _ev: &E,
            node: Node<P, D>,
            out: &mut Vec<MercuryEffect>,
        ) {
            tmux(ModifierFlags::SHIFT, Key::$digit, out);
            go_home(node.parent.ascend(), out);
        }
    )*};
}
```

`main.rs`, `dispatch_event`, before:

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

after (a per-call buffer, which change 3 hoists into the loop):

```rust
fn dispatch_event(
    state: &mut Mercury,
    event: &MercuryEvent,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let mut effects = Vec::new();
    let _ = state.handle(event, &mut effects);
    info!(event = ?event, effects = ?effects, state = ?state, "dispatch");
    for effect in effects.drain(..) {
        let _ = effect_tx.send(effect);
    }
}
```

### `crates/bind/tests`

`common/mod.rs`: the marker names the effect type, the handlers push, and the shared helper turns a dispatch into the `Option<Vec<usize>>` the assertions already read.

before:

```rust
impl Bindings for Demo {
    type Trigger = DemoTrigger;
    type Event = DemoEvent;
    type Output = usize;
}

// Handlers. Each takes its node's path and returns the fired key's length.
pub const fn on_esc(ev: &KeyEvent, node: Node<&mut App, ()>) -> usize {
    let app = node.parent;
    app.hits += 1;
    ev.key.len()
}
```

after:

```rust
impl Bindings for Demo {
    type Trigger = DemoTrigger;
    type Event = DemoEvent;
    type Effect = usize;
}

/// Dispatch and collect: `Some(effects)` when a binding claimed the event, `None` when none did.
pub fn fire<'a, N>(path: N::Path<'a>, event: &DemoEvent) -> Option<Vec<usize>>
where
    N: bind::Dispatch<Demo> + 'a,
{
    let mut out = Vec::new();
    match bind::dispatch::<Demo, N>(path, event, &mut out) {
        bind::Dispatched::Handled => Some(out),
        bind::Dispatched::Unbound => None,
    }
}

// Handlers. Each takes its node's path and pushes the fired key's length.
pub fn on_esc(ev: &KeyEvent, node: Node<&mut App, ()>, out: &mut Vec<usize>) {
    let app = node.parent;
    app.hits += 1;
    out.push(ev.key.len());
}
```

Every other handler in that file (`on_f1`, `on_g`, `on_slack`, `on_bksp`, `on_d`, `on_title`, `ignore`) takes the same third parameter and pushes instead of returning.

`dispatch.rs` and `derived.rs` assert through `fire`, before:

```rust
let out = bind::dispatch::<Demo, App>(&mut app, &key("g"));
assert_eq!(out, Some(1)); // "g"
```

after:

```rust
assert_eq!(fire::<App>(&mut app, &key("g")), Some(vec![1])); // "g"
```

An unbound event is still `None`:

```rust
assert_eq!(fire::<App>(&mut app, &key("x")), None);
```

`derived.rs`'s own handlers (`on_r`, `on_g`, `on_esc`) push into `&mut Vec<usize>` the same way, and its assertions go through `common::fire::<Root>`.

`compile_fail/trigger_not_into.rs` keeps its line count so its `.stderr` stays valid: `type Output = ();` becomes `type Effect = ();`, and `fn handler(_: &KeyEv, _path: impl Sized) {}` becomes `fn handler(_: &KeyEv, _path: impl Sized, _out: &mut Vec<()>) {}`.

### `crates/mercury/tests/transitions.rs`

The per-event tests assert `Option<Vec<MercuryEffect>>` against `handle`. A file-local helper keeps every one of those assertions as it stands:

```rust
/// Dispatch one event and collect its effects, so the assertions read as they did when `handle`
/// returned them.
fn handle(m: &mut Mercury, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    let mut out = Vec::new();
    match m.handle(event, &mut out) {
        Dispatched::Handled => Some(out),
        Dispatched::Unbound => None,
    }
}
```

Each call site changes mechanically: `m.handle(&key(Key::KeyN))` becomes `handle(&mut m, &key(Key::KeyN))`.

`settle` drains the sink after each step, so a `Foreground` effect still queues its event before the next step runs. before:

```rust
fn settle(
    runner: &mut SimpleRunner<'_, MercuryStruct, Mercury>,
    performed: &mut Vec<MercuryEffect>,
) {
    while let Some(dispatched) = runner.next() {
        if let Some(output) = dispatched {
            for effect in output {
                if let MercuryEffect::Foreground(app) = &effect {
                    runner.queue_event(foreground(*app));
                }
                performed.push(effect);
            }
        }
    }
}
```

after:

```rust
fn settle(
    runner: &mut SimpleRunner<'_, MercuryStruct, Mercury>,
    performed: &mut Vec<MercuryEffect>,
) {
    let mut effects = Vec::new();
    while runner.next(&mut effects).is_some() {
        for effect in effects.drain(..) {
            if let MercuryEffect::Foreground(app) = &effect {
                runner.queue_event(foreground(*app));
            }
            performed.push(effect);
        }
    }
}
```

## change 3 (ships alone): the event loop reuses one buffer

The buffer moves out of `dispatch_event` and into the loop, so it keeps its capacity across events and a dispatched key allocates nothing once it has warmed up. `drain` leaves it empty for the next event.

`crates/mercury/src/main.rs`, before:

```rust
async fn run_event_loop(
    mut state: Mercury,
    mut event_rx: UnboundedReceiver<MercuryEvent>,
    effect_tx: UnboundedSender<MercuryEffect>,
) {
    info!(state = ?state, "initial state");
    while let Some(event) = event_rx.recv().await {
        dispatch_event(&mut state, &event, &effect_tx);
    }
}

fn dispatch_event(
    state: &mut Mercury,
    event: &MercuryEvent,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let mut effects = Vec::new();
    let _ = state.handle(event, &mut effects);
    info!(event = ?event, effects = ?effects, state = ?state, "dispatch");
    for effect in effects.drain(..) {
        let _ = effect_tx.send(effect);
    }
}
```

after:

```rust
async fn run_event_loop(
    mut state: Mercury,
    mut event_rx: UnboundedReceiver<MercuryEvent>,
    effect_tx: UnboundedSender<MercuryEffect>,
) {
    info!(state = ?state, "initial state");
    // One buffer for the life of the loop: `dispatch_event` drains it, so it comes back empty
    // with its capacity intact and a dispatched key allocates nothing.
    let mut effects = Vec::new();
    while let Some(event) = event_rx.recv().await {
        dispatch_event(&mut state, &event, &mut effects, &effect_tx);
    }
}

/// Dispatch one event and enqueue whatever effects it produced. `effects` arrives empty and
/// leaves empty.
fn dispatch_event(
    state: &mut Mercury,
    event: &MercuryEvent,
    effects: &mut Vec<MercuryEffect>,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let _ = state.handle(event, effects);
    info!(event = ?event, effects = ?effects, state = ?state, "dispatch");
    for effect in effects.drain(..) {
        let _ = effect_tx.send(effect);
    }
}
```
