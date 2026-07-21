---
title: Grabbing the Keyboard
sidebar_position: 2
---

# Grabbing the Keyboard

The grab swallows every key and hands it to the model as an event. It also hands back an emitter, which is how keys get back out.

## The tap

`freddie_keyboard` grabs the keyboard with a session `CGEventTap`, installed on a thread of its own that runs a `CFRunLoop` for it:

```rust
CGEventTap::with_enabled(
    CGEventTapLocation::Session,
    CGEventTapPlacement::TailAppendEventTap,
    CGEventTapOptions::Default,
    vec![
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
    ],
    callback,
    || {
        ready.send(CFRunLoop::get_current());
        CFRunLoop::run_current();
    },
);
```

The system keeps an ordered list of the active taps at that location, and every key runs through them in turn. A callback returns the event to pass it, a different event to replace it, or nothing to drop it, and the result feeds the next tap and then the app. `with_enabled` does the run-loop wiring inside `core-graphics`, so nothing in the crate is `unsafe` and it stays under the workspace's `forbid(unsafe_code)`.

The alternative is the HID layer, which is what Karabiner runs on: a DriverKit system extension seizes the physical keyboard and posts to a separate virtual device, so output is never input. It buys two things the tap cannot give. It sits below secure input, so remaps keep working in password fields and in apps that call `EnableSecureEventInput`, where a session tap sees nothing. And it is correct across processes: a tap that re-posts its output can loop against another remapper that re-posts back, which no tag prevents.

The tap wins on cost. It is one userland process, safe Rust over `core-graphics`, no driver and no root, gated only on Accessibility and Input Monitoring, which the user grants in System Settings. The HID route is a C++ system extension, code-signed with HID entitlements Apple has to approve, notarized, plus a root daemon to drive the virtual device. Both sit behind the same crate API, so the swap stays possible without the model noticing.

Staying alive is a matter of keeping the callback fast. macOS disables a tap whose callback takes too long, and `freddie_keyboard` does not turn it back on. `mercury`'s callback does one channel send and returns:

```rust
freddie_keyboard::intercept(move |ev| {
    event_tx.send(MercuryEvent::Key(ev));
    None
});
```

`None` swallows the key. Everything after that is the model's, and the effect loop puts back whatever should reach the app.

## Modifier flags

Modifiers arrive as `FlagsChanged` rather than as `KeyDown` and `KeyUp`, and the keycode says which side, so `Key::ShiftLeft` and `Key::ShiftRight` are separate keys. `press_of` reads the direction off the flag bit: the modifier is down if its bit is set after the change.

The root holds the physical truth, one `LeftRightPair` per modifier:

```rust
pub struct HeldModifiers {
    pub control: LeftRightPair,
    pub meta: LeftRightPair,
    pub alt: LeftRightPair,
    pub shift: LeftRightPair,
}
```

`maybe_pass_through`, the root's catch-all, calls `held.apply(ev)` for every modifier event in every layer, so the record stays right even while a command layer is swallowing the keys. `held.flags()` reads it back as a `ModifierFlags` bitset, which is the value stamped on a passed-through key. Caps lock is a lock rather than a held key, so it is not tracked here, and `fn` has no `Key` variant, so it rides through only as the `FN` bit an event already carries.

An emitted key states its own flags because a `CGEvent`'s flags are baked in when it is created, from the event source's state, which lags a modifier posted microseconds earlier. A chord posted back to back would carry the wrong ones. So the flags live on the event:

```rust
pub struct KeyEvent {
    pub key: Key,
    pub press: PressType,
    pub flags: ModifierFlags,
}
```

and the emitter applies exactly them, over the six device-independent bits `MODIFIERS` names:

```rust
let untouched = event.get_flags() & !MODIFIERS;
event.set_flags(untouched | to_cg(flags));
```

Everything outside those six bits is left as the OS put it there, which is how an arrow keeps its `NumericPad` bit and a space never gains one.

## Dying with the grab

The keyboard is not held by anything that outlives the process. Dropping the `Interceptor` stops the tap's run loop and joins its thread; a process that dies without dropping it has its tap torn down by the OS. Either way the keyboard goes back to behaving the way macOS would, with no stuck grab to clear.

What does not clean itself up is the app's idea of which modifiers are held. A command layer swallows the real modifier downs, so the app underneath was never told about them. `mercury stop` sends SIGTERM, which the daemon routes into the event channel as the same `Quit` the menu bar's item sends, and the quit handler emits the held modifiers' downs before it asks to die:

```rust
let mut effects = root.typing_state.held.open();
effects.push(MercuryEffect::Kill);
```

`Kill` breaks the effect loop rather than exiting the process, so destructors run and the `Interceptor` releases the keyboard after the flush has gone out.

`mercury stop --force` sends SIGKILL instead. The kernel destroys the process, no destructor runs, and a modifier the user is physically holding stays down in the app underneath. It is the only out for a daemon whose worker is stuck in an effect, and that is the price.
