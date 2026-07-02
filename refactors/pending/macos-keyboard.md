# macOS keyboard and foreground

How the real build (figaro) fills the stubs in event-loop.md: the keyboard source is a `CGEventTap`, the `Type`/`Command` effects are synthesized `CGEvent`s, and the foreground source is an `NSWorkspace` observer. None of it is research-hard; the work is permissions, not locking yourself out, and not feeding your own synthesized events back into your tap.

Rust crates: `core-graphics` (`CGEvent`, `CGEventTap`, `CGEventSource`, the field/flag enums), `core-foundation` (`CFRunLoop`), and `objc2` + `objc2-app-kit` + `objc2-foundation` for `NSWorkspace` (or the older `cocoa`/`objc`). The exact binding names drift between crate versions, so check them; the CoreGraphics/AppKit calls below are the stable ground truth.

## Keyboard hijacking (CGEventTap)

A tap sees every key event before the focused app does, and can pass it through or swallow it. Returning the event passes it; returning null drops it. Dropping a handled key is the remap: the physical key never reaches the app, and you emit the remapped keys by synthesis (below).

Setup, on its own thread that owns a run loop (this is the keyboard source of event-loop.md):

```
tap = CGEventTapCreate(
    kCGSessionEventTap,          // session level; kCGHIDEventTap is lower still
    kCGHeadInsertEventTap,
    kCGEventTapOptionDefault,    // active tap: may modify/swallow (not ListenOnly)
    CGEventMaskBit(kCGEventKeyDown)
        | CGEventMaskBit(kCGEventKeyUp)
        | CGEventMaskBit(kCGEventFlagsChanged),
    callback,
    user_info,
);
source = CFMachPortCreateRunLoopSource(NULL, tap, 0);
CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
CGEventTapEnable(tap, true);
CFRunLoopRun();                  // blocks this thread; the callback fires here
```

The callback must be fast: it forwards a `KeyEvent` into the event channel and returns. Do not dispatch or synthesize inside it.

```
callback(proxy, type, event, user_info) -> CGEventRef {
    // Re-enable if the OS disabled us (see timeouts below).
    if type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput {
        CGEventTapEnable(tap, true);
        return event;
    }
    // Skip our own synthesized events (see tagging) or we loop forever.
    if is_ours(event) {
        return event;
    }
    let keycode = CGEventGetIntegerValueField(event, kCGEventKeyboardEventKeycode);
    event_tx.send(key_event_from(keycode, type));
    // Swallow keys we handle; pass the rest through.
    if we_handle(keycode) { NULL } else { event }
}
```

Whether to swallow is the remapper's decision. A layer that rebinds a key returns null (swallow) and emits the replacement; an unbound key returns the event (pass through).

Permissions: an active (modifying) keyboard tap needs Accessibility, and on 10.15+ keyboard taps also need Input Monitoring. Both are granted to whatever launches the binary, which during development is the terminal (or the built `.app`). The first run prompts; grant in System Settings > Privacy & Security > Accessibility and Input Monitoring, then relaunch. The exact TCC requirement shifts by macOS version, so verify on the target.

Footguns:

- Self-lockout. Swallowing everything with no way out locks the machine. This is why the runner keeps the dev killswitches (event-loop.md): a `Kill` effect on a timer, a hard `_exit` backstop, and `SIGHUP` left fatal.
- Tap disabled by timeout. If the callback is slow, the OS sends `kCGEventTapDisabledByTimeout` and stops delivering; re-enable with `CGEventTapEnable`. Keep the callback to forward-and-return.
- Secure input. Password fields, and any app that calls `EnableSecureEventInput`, bypass taps. You cannot intercept those, by design.

## Emitting keyboard events (synthesis)

The `Type` and `Command` effects post synthetic key events. Create one `CGEventSource` and reuse it.

```
source = CGEventSourceCreate(kCGEventSourceStateHIDSystemState);

// A keystroke: down then up.
down = CGEventCreateKeyboardEvent(source, keycode, true);
up   = CGEventCreateKeyboardEvent(source, keycode, false);
tag(down); tag(up);                         // see tagging
CGEventPost(kCGHIDEventTap, down);
CGEventPost(kCGHIDEventTap, up);

// A chord like cmd+r: set the modifier flag on the key event.
CGEventSetFlags(down, kCGEventFlagMaskCommand);
CGEventSetFlags(up,   kCGEventFlagMaskCommand);

// Arbitrary text, layout-independent (no keycode table needed):
CGEventKeyboardSetUnicodeString(down, len, utf16_buffer);
```

For typing text, `CGEventKeyboardSetUnicodeString` avoids maintaining a char-to-keycode map and is layout-independent; for chords and named keys you need virtual keycodes (`kVK_ANSI_R` and friends from `Carbon/HIToolbox`).

Permission: posting events needs Accessibility.

## Tagging (avoid self-feedback)

Events you post are delivered to your own tap, so without a mark you would re-dispatch and re-emit them forever. Tag every synthesized event and skip tagged events in the callback.

```
const MINE: i64 = 0x6672_6564; // "fred"

fn tag(event) {
    CGEventSetIntegerValueField(event, kCGEventSourceUserData, MINE);
}
fn is_ours(event) -> bool {
    CGEventGetIntegerValueField(event, kCGEventSourceUserData) == MINE
}
```

`kCGEventSourceUserData` is a 64-bit field you own, which is the simplest marker. The alternative is to give your `CGEventSource` a known state id and compare `kCGEventSourceStateID` in the callback. Either way, the tap must ignore its own output.

## Foreground listening (NSWorkspace)

Which app is frontmost, and a notification when it changes. No special permission; it needs a run loop, which the tap thread already has (or use the main thread).

```
// Current:
let app = NSWorkspace.sharedWorkspace().frontmostApplication(); // NSRunningApplication

// On change:
NSWorkspace.sharedWorkspace()
    .notificationCenter()
    .addObserverForName(NSWorkspaceDidActivateApplicationNotification, ...) { note in
        let app = note.userInfo()[NSWorkspaceApplicationKey]; // NSRunningApplication
        event_tx.send(foreground_event_from(app.bundleIdentifier()));
    };
```

`NSRunningApplication` gives `bundleIdentifier`, `localizedName`, and `processIdentifier`, which is how you map a running app onto Mercury's `App`. This observer is the second source feeding the event channel.

## Foregrounding apps (the Foreground effect)

The `Foreground` effect activates or launches an app; the observer above then reports it coming up as a normal event (the decoupling in event-loop.md), so nothing here mutates state.

```
// Already running: bring it up.
NSRunningApplication.runningApplicationsWithBundleIdentifier(id).first?.activate(options);

// Not running: launch it.
NSWorkspace.sharedWorkspace().openApplicationAtURL(url, configuration, handler);
```

No Accessibility needed. Cross-app activation tightened in Sonoma, so a forced foreground is not always granted; launching works, and the observer confirms the real frontmost app regardless.

## Where each piece lands in the runner

- Keyboard tap thread: the keyboard source. Its callback forwards `KeyEvent`s into the event channel and returns fast, swallowing handled keys.
- Foreground observer: the second source, forwarding `ForegroundEvent`s into the same channel.
- Effect loop: performs `Type`/`Command` by synthesizing tagged events, and `Foreground` by activating/launching. `Kill` still exits here.
- The event channel `Sender` is cloned to both source threads; synthesized events must be tagged so the tap ignores them.

## Permissions, at a glance

- Keyboard tap (active): Accessibility, plus Input Monitoring on 10.15+.
- Posting synthetic events: Accessibility.
- Foreground watching and app activation/launch: none.
