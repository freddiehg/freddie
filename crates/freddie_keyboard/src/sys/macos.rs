//! The macOS backend, on `core-graphics`. The pure parts (the keycode table, the
//! pass/remap/drop decision, the modifier flags) are unit-tested below; the tap
//! and the posting are FFI that needs a real keyboard to exercise.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CGKeyCode, CallbackResult, EventField, KeyCode,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use freddie_hid_device::{DeviceInfo, ResolveFailure, SourceId, resolve, source_of};
use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};
use objc2::rc::autoreleasepool;

use crate::{CaptureError, EmitError};

// ---------------------------------------------------------------------------
// Pure logic.
// ---------------------------------------------------------------------------

// Every named key and its macOS virtual key code. Keys with no macOS code
// (F21-F24, Insert) are absent, so `to_code` gives `None` and `from_code` gives
// `Key::Raw`.
const TABLE: &[(Key, CGKeyCode)] = &[
    (Key::KeyA, KeyCode::ANSI_A),
    (Key::KeyB, KeyCode::ANSI_B),
    (Key::KeyC, KeyCode::ANSI_C),
    (Key::KeyD, KeyCode::ANSI_D),
    (Key::KeyE, KeyCode::ANSI_E),
    (Key::KeyF, KeyCode::ANSI_F),
    (Key::KeyG, KeyCode::ANSI_G),
    (Key::KeyH, KeyCode::ANSI_H),
    (Key::KeyI, KeyCode::ANSI_I),
    (Key::KeyJ, KeyCode::ANSI_J),
    (Key::KeyK, KeyCode::ANSI_K),
    (Key::KeyL, KeyCode::ANSI_L),
    (Key::KeyM, KeyCode::ANSI_M),
    (Key::KeyN, KeyCode::ANSI_N),
    (Key::KeyO, KeyCode::ANSI_O),
    (Key::KeyP, KeyCode::ANSI_P),
    (Key::KeyQ, KeyCode::ANSI_Q),
    (Key::KeyR, KeyCode::ANSI_R),
    (Key::KeyS, KeyCode::ANSI_S),
    (Key::KeyT, KeyCode::ANSI_T),
    (Key::KeyU, KeyCode::ANSI_U),
    (Key::KeyV, KeyCode::ANSI_V),
    (Key::KeyW, KeyCode::ANSI_W),
    (Key::KeyX, KeyCode::ANSI_X),
    (Key::KeyY, KeyCode::ANSI_Y),
    (Key::KeyZ, KeyCode::ANSI_Z),
    (Key::Num0, KeyCode::ANSI_0),
    (Key::Num1, KeyCode::ANSI_1),
    (Key::Num2, KeyCode::ANSI_2),
    (Key::Num3, KeyCode::ANSI_3),
    (Key::Num4, KeyCode::ANSI_4),
    (Key::Num5, KeyCode::ANSI_5),
    (Key::Num6, KeyCode::ANSI_6),
    (Key::Num7, KeyCode::ANSI_7),
    (Key::Num8, KeyCode::ANSI_8),
    (Key::Num9, KeyCode::ANSI_9),
    (Key::F1, KeyCode::F1),
    (Key::F2, KeyCode::F2),
    (Key::F3, KeyCode::F3),
    (Key::F4, KeyCode::F4),
    (Key::F5, KeyCode::F5),
    (Key::F6, KeyCode::F6),
    (Key::F7, KeyCode::F7),
    (Key::F8, KeyCode::F8),
    (Key::F9, KeyCode::F9),
    (Key::F10, KeyCode::F10),
    (Key::F11, KeyCode::F11),
    (Key::F12, KeyCode::F12),
    (Key::F13, KeyCode::F13),
    (Key::F14, KeyCode::F14),
    (Key::F15, KeyCode::F15),
    (Key::F16, KeyCode::F16),
    (Key::F17, KeyCode::F17),
    (Key::F18, KeyCode::F18),
    (Key::F19, KeyCode::F19),
    (Key::F20, KeyCode::F20),
    (Key::Escape, KeyCode::ESCAPE),
    (Key::Return, KeyCode::RETURN),
    (Key::Space, KeyCode::SPACE),
    (Key::Tab, KeyCode::TAB),
    (Key::Backspace, KeyCode::DELETE),
    (Key::Delete, KeyCode::FORWARD_DELETE),
    (Key::CapsLock, KeyCode::CAPS_LOCK),
    (Key::UpArrow, KeyCode::UP_ARROW),
    (Key::DownArrow, KeyCode::DOWN_ARROW),
    (Key::LeftArrow, KeyCode::LEFT_ARROW),
    (Key::RightArrow, KeyCode::RIGHT_ARROW),
    (Key::Home, KeyCode::HOME),
    (Key::End, KeyCode::END),
    (Key::PageUp, KeyCode::PAGE_UP),
    (Key::PageDown, KeyCode::PAGE_DOWN),
    (Key::ShiftLeft, KeyCode::SHIFT),
    (Key::ShiftRight, KeyCode::RIGHT_SHIFT),
    (Key::ControlLeft, KeyCode::CONTROL),
    (Key::ControlRight, KeyCode::RIGHT_CONTROL),
    (Key::AltLeft, KeyCode::OPTION),
    (Key::AltRight, KeyCode::RIGHT_OPTION),
    (Key::MetaLeft, KeyCode::COMMAND),
    (Key::MetaRight, KeyCode::RIGHT_COMMAND),
    (Key::Grave, KeyCode::ANSI_GRAVE),
    (Key::Minus, KeyCode::ANSI_MINUS),
    (Key::Equal, KeyCode::ANSI_EQUAL),
    (Key::LeftBracket, KeyCode::ANSI_LEFT_BRACKET),
    (Key::RightBracket, KeyCode::ANSI_RIGHT_BRACKET),
    (Key::BackSlash, KeyCode::ANSI_BACKSLASH),
    (Key::SemiColon, KeyCode::ANSI_SEMICOLON),
    (Key::Quote, KeyCode::ANSI_QUOTE),
    (Key::Comma, KeyCode::ANSI_COMMA),
    (Key::Dot, KeyCode::ANSI_PERIOD),
    (Key::Slash, KeyCode::ANSI_SLASH),
];

fn to_code(key: Key) -> Option<CGKeyCode> {
    if let Key::Raw(code) = key {
        return Some(code);
    }
    TABLE.iter().find(|(k, _)| *k == key).map(|(_, code)| *code)
}

fn from_code(code: CGKeyCode) -> Key {
    TABLE
        .iter()
        .find(|(_, c)| *c == code)
        .map_or(Key::Raw(code), |(key, _)| *key)
}

const fn flag_for(key: Key) -> Option<CGEventFlags> {
    Some(match key {
        Key::MetaLeft | Key::MetaRight => CGEventFlags::CGEventFlagCommand,
        Key::ShiftLeft | Key::ShiftRight => CGEventFlags::CGEventFlagShift,
        Key::AltLeft | Key::AltRight => CGEventFlags::CGEventFlagAlternate,
        Key::ControlLeft | Key::ControlRight => CGEventFlags::CGEventFlagControl,
        // Caps lock is FlagsChanged on the wire; without this the tap Keeps it and the OS
        // toggles AlphaShift while the model never sees the key.
        Key::CapsLock => CGEventFlags::CGEventFlagAlphaShift,
        _ => return None,
    })
}

/// What the callback should do with a key.
#[derive(PartialEq, Eq, Debug)]
enum Decision {
    Pass,
    Remap(KeyEvent),
    Drop,
}

// The remap decision: what `on_key` returned against what came in.
fn decide(input: &KeyEvent, out: Option<KeyEvent>) -> Decision {
    match out {
        None => Decision::Drop,
        Some(ref e) if e == input => Decision::Pass,
        Some(e) => Decision::Remap(e),
    }
}

// ---------------------------------------------------------------------------
// The tap and the posting (FFI).
// ---------------------------------------------------------------------------

/// The marker an emitted event carries so the interceptor recognizes its own output.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Tag(i64);

impl Tag {
    /// Per-process random, so an interceptor skips only its own emitter's output.
    fn new() -> Self {
        let mut h = RandomState::new().build_hasher();
        h.write_u8(0);
        Self(i64::from_ne_bytes(h.finish().to_ne_bytes()))
    }

    /// Marks `event` as this emitter's, so the tap passes it rather than handling it again.
    fn stamp(self, event: &CGEvent) {
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.0);
    }

    /// Whether `event` carries this tag, and so came from this emitter.
    fn marks(self, event: &CGEvent) -> bool {
        event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == self.0
    }
}

/// Keycode from a keyboard event, if it fits in `u16`.
fn keycode(event: &CGEvent) -> Option<CGKeyCode> {
    u16::try_from(event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)).ok()
}

/// Press type for `kind`/`key`. For `CapsLock`, `caps_down` tracks physical hold: `AlphaShift` is a
/// latch, not a hold bit, so flag-based up/down is wrong for dual-role.
fn press_of_key(
    kind: CGEventType,
    event: &CGEvent,
    key: Key,
    caps_down: &Cell<bool>,
) -> Option<PressType> {
    match kind {
        CGEventType::KeyDown => Some(PressType::Down),
        CGEventType::KeyUp => Some(PressType::Up),
        CGEventType::FlagsChanged if key == Key::CapsLock => {
            if caps_down.get() {
                caps_down.set(false);
                Some(PressType::Up)
            } else {
                caps_down.set(true);
                Some(PressType::Down)
            }
        }
        // Other modifiers: down if its flag bit is set after the change.
        CGEventType::FlagsChanged => {
            let flag = flag_for(key)?;
            Some(if event.get_flags().contains(flag) {
                PressType::Down
            } else {
                PressType::Up
            })
        }
        _ => None,
    }
}

/// A keyboard event for `key`, carrying exactly `flags`, built from a long-lived private source.
///
/// `source` is the caller's private source for its thread: the emitter's, or the tap's remap
/// source. Each `CGEventSourceCreate(Private)` maps about 16KB of shared memory that `CFRelease`
/// never unmaps and no API reclaims, so a source per event grows the process by 16KB per
/// keystroke for the life of the run. The source is passed in rather than built here.
///
/// The flags on the wire are `to_cg(flags) | intrinsic_flags(code)` and nothing else. The bits
/// the event is born with are ignored, because posting through a source mutates it: an arrow
/// leaves `NumericPad` in the source's state, and every event built from it afterwards is born
/// carrying that bit. Reading those birth flags back would put `NumericPad` on a later
/// `cmd`-`space`, which stops matching Spotlight's hotkey for the rest of the run.
///
/// Not a `NULL` source, which means the shared session state rather than no state, and so
/// inherits bits other processes have left there.
///
/// # Errors
///
/// Returns [`EmitError::Unmappable`] if the key has no code on this OS, and [`EmitError::Post`]
/// if the OS refused to build the event.
fn keyboard_event(
    source: &CGEventSource,
    key: Key,
    press: PressType,
    flags: ModifierFlags,
) -> Result<CGEvent, EmitError> {
    let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
    // `new_keyboard_event` takes the source by value; the clone is a `CFRetain` of the same
    // source, not a second mapping.
    let event = CGEvent::new_keyboard_event(source.clone(), code, press == PressType::Down)
        .map_err(|_| EmitError::Post)?;
    let intrinsic = intrinsic_flags(code);
    event.set_flags(to_cg(flags) | intrinsic);
    // What actually goes on the wire, which the portable `KeyEvent` cannot show: the raw flag
    // bits, the bits the keycode itself carries, and the type the OS chose from the keycode
    // (`FlagsChanged` for a modifier, `KeyDown`/`KeyUp` otherwise). At `debug` so the log file
    // keeps it, since two presses that dispatch identically can still post differently.
    tracing::debug!(
        ?key,
        ?press,
        raw_flags = %format!("{:#010x}", event.get_flags().bits()),
        intrinsic = %format!("{:#010x}", intrinsic.bits()),
        kind = ?event.get_type(),
        "post"
    );
    Ok(event)
}

/// Grab the keyboard. The interceptor swallows and decides via `on_key`; the
/// emitter synthesizes keys, tagged so the interceptor passes them.
///
/// # Errors
///
/// Returns [`CaptureError`] if the tap cannot be installed (usually missing
/// Accessibility).
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    run_tap(move |input, _event| on_key(input))
}

/// Same tap as [`intercept`], with per-key source device categorization.
///
/// `categorize` runs once per distinct HID source (and on every synthetic key for
/// `None`). `on_key` receives the key and the categorized value `T`.
///
/// # Errors
///
/// Returns [`CaptureError`] if the tap cannot be installed (usually missing
/// Accessibility).
pub fn intercept_with_source<T, C, F>(
    mut categorize: C,
    on_key: F,
) -> Result<(Interceptor, Emitter), CaptureError>
where
    T: Clone + Send + 'static,
    C: FnMut(Option<Result<DeviceInfo, (SourceId, ResolveFailure)>>) -> T + Send + 'static,
    F: Fn((KeyEvent, T)) -> Option<KeyEvent> + Send + 'static,
{
    let mut by_source: HashMap<SourceId, T> = HashMap::new();
    run_tap(move |input, event| {
        let class = match source_of(event) {
            None => categorize(None),
            Some(id) => by_source
                .entry(id)
                .or_insert_with(|| categorize(Some(resolve(id))))
                .clone(),
        };
        on_key((input, class))
    })
}

/// Shared tap install. `on_key` already decided pass/remap/drop via its return.
/// `event` is the live `CGEvent` for the key; only [`intercept_with_source`] reads it.
fn run_tap(
    on_key: impl FnMut(KeyEvent, &CGEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    let tag = Tag::new();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<CFRunLoop, ()>>();
    let signal = ready_tx.clone();
    // The tap callback is `Fn`, not `FnMut`; cache-owning categorize needs mutability.
    let on_key = RefCell::new(on_key);

    let thread = std::thread::spawn(move || {
        // The remap source, created once on this thread and borrowed by every remap. A source per
        // remapped key would map 16KB of shared memory per key that nothing ever unmaps, and this
        // one is never touched from another thread, which is what posting through a source needs.
        let Ok(remap_source) = CGEventSource::new(CGEventSourceStateID::Private) else {
            let _ = signal.send(Err(()));
            return;
        };
        // CapsLock AlphaShift is a latch, not a hold. Track physical down ourselves.
        let caps_down = Cell::new(false);
        let outcome = CGEventTap::with_enabled(
            CGEventTapLocation::Session,
            // Head so Drop can stop CapsLock before the OS latches AlphaShift.
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            vec![
                CGEventType::KeyDown,
                CGEventType::KeyUp,
                CGEventType::FlagsChanged,
            ],
            move |_proxy, kind, event| {
                if tag.marks(event) {
                    return CallbackResult::Keep; // our own emit
                }
                let Some(code) = keycode(event) else {
                    return CallbackResult::Keep;
                };
                let key = from_code(code);
                let Some(press) = press_of_key(kind, event, key, &caps_down) else {
                    return CallbackResult::Keep;
                };
                let input = KeyEvent {
                    key,
                    press,
                    // The modifiers the source baked onto this event. A modifier delivered as a
                    // flag rather than as its own key (an injected `cmd`-`v`, or `fn`) lives only
                    // here, so read it or it is lost.
                    flags: from_cg(event.get_flags()),
                };
                // Physical HID input is PID 0; a userspace `CGEventPost` (another app) is nonzero.
                // Logged only. (Our own emits are tagged and returned above.)
                let source_pid =
                    event.get_integer_value_field(EventField::EVENT_SOURCE_UNIX_PROCESS_ID);
                tracing::trace!(?input, source_pid, "tap");
                match decide(&input, on_key.borrow_mut()(input.clone(), event)) {
                    Decision::Pass => CallbackResult::Keep,
                    Decision::Drop => {
                        // CapsLock is a latch (AlphaShift), not a hold. Dropping the FlagsChanged
                        // at HeadInsert still leaves the HIDSystem latch flipped: the driver has
                        // already toggled it. Clear it via IOHID before Escape/Control posts.
                        if key == Key::CapsLock && press == PressType::Down {
                            clear_caps_latch();
                        }
                        CallbackResult::Drop
                    }
                    Decision::Remap(out) => {
                        if key == Key::CapsLock && press == PressType::Down {
                            clear_caps_latch();
                        }
                        match keyboard_event(&remap_source, out.key, out.press, out.flags) {
                            Ok(event) => CallbackResult::Replace(event),
                            Err(e) => {
                                tracing::warn!(key = ?out.key, error = %e, "dropped a remapped key");
                                CallbackResult::Drop
                            }
                        }
                    }
                }
            },
            || {
                let _ = ready_tx.send(Ok(CFRunLoop::get_current()));
                CFRunLoop::run_current();
            },
        );
        if outcome.is_err() {
            let _ = signal.send(Err(()));
        }
    });

    let Ok(Ok(run_loop)) = ready_rx.recv() else {
        return Err(CaptureError);
    };
    let interceptor = Interceptor {
        _tap: TapThread {
            run_loop,
            thread: Some(thread),
        },
    };
    // The emitter's own source, on the thread that will post through it. A failure here is a
    // `CaptureError` like a tap that will not install: an emitter that cannot build an event has
    // no working `emit`, and `run_tap` hands back both halves or neither.
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|()| CaptureError)?;
    let emitter = Emitter { tag, source };
    Ok((interceptor, emitter))
}

/// An active grab of the keyboard. While it is alive keys are intercepted;
/// dropping it releases the keyboard.
///
/// No `Drop` of its own: dropping it drops the [`TapThread`], which is what the release is.
pub struct Interceptor {
    _tap: TapThread,
}

/// How long a dropped [`TapThread`] waits for the tap thread to finish before giving up on it.
///
/// Stopping the run loop is what ends that thread, and it ends promptly unless it is inside a slow
/// `on_key`. Waiting forever would turn one wedged callback into a process that cannot exit, and
/// this runs on the shutdown path and during unwinds.
const RELEASE_TIMEOUT: Duration = Duration::from_millis(500);

/// The thread the event tap runs on, and the run loop that ends it.
///
/// One resource in two parts: stopping the run loop is what makes the thread return, and joining is
/// how the release finishes.
struct TapThread {
    run_loop: CFRunLoop,
    thread: Option<JoinHandle<()>>,
}

impl Drop for TapThread {
    fn drop(&mut self) {
        self.run_loop.stop();
        let Some(thread) = self.thread.take() else {
            return;
        };
        // Joined on another thread so this one can stop waiting. The tap is released when the
        // thread ends either way; what the timeout bounds is how long the caller waits to hear
        // about it.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = thread.join();
            let _ = done_tx.send(());
        });
        if done_rx.recv_timeout(RELEASE_TIMEOUT).is_err() {
            tracing::warn!("the keyboard tap did not stop; releasing without it");
        }
    }
}

/// The keycodes a clean private source puts `NumericPad` on: the four arrows, and the keypad
/// apart from `ANSI_KEYPAD_CLEAR` and `JIS_KEYPAD_COMMA`.
///
/// Measured, by building an event for every code from 0 to 127 on a clean private source and
/// reading the bit back. `HOME`, `END`, `PAGE_UP` and `PAGE_DOWN` read like the arrows and are
/// not in it.
///
/// By keycode rather than by [`Key`], because the keypad has no `Key` variant: those keys reach
/// the emitter as `Key::Raw(code)`, so a table of variants would drop the bit for exactly the
/// keys whose name says they carry it.
const NUMERIC_PAD_CODES: &[CGKeyCode] = &[
    KeyCode::LEFT_ARROW,
    KeyCode::RIGHT_ARROW,
    KeyCode::DOWN_ARROW,
    KeyCode::UP_ARROW,
    KeyCode::ANSI_KEYPAD_DECIMAL,
    KeyCode::ANSI_KEYPAD_MULTIPLY,
    KeyCode::ANSI_KEYPAD_PLUS,
    KeyCode::ANSI_KEYPAD_DIVIDE,
    KeyCode::ANSI_KEYPAD_ENTER,
    KeyCode::ANSI_KEYPAD_MINUS,
    KeyCode::ANSI_KEYPAD_EQUAL,
    KeyCode::ANSI_KEYPAD_0,
    KeyCode::ANSI_KEYPAD_1,
    KeyCode::ANSI_KEYPAD_2,
    KeyCode::ANSI_KEYPAD_3,
    KeyCode::ANSI_KEYPAD_4,
    KeyCode::ANSI_KEYPAD_5,
    KeyCode::ANSI_KEYPAD_6,
    KeyCode::ANSI_KEYPAD_7,
    KeyCode::ANSI_KEYPAD_8,
    KeyCode::ANSI_KEYPAD_9,
];

/// The non-modifier flag bits `code` carries of its own accord.
///
/// [`keyboard_event`] names the bits it emits rather than subtracting the ones it does not, so
/// what a key posts is a function of the key and the caller's modifiers, and never of what an
/// earlier post left in the source. `SecondaryFn` is not here: it is a portable [`ModifierFlags`]
/// bit and arrives through `to_cg`.
fn intrinsic_flags(code: CGKeyCode) -> CGEventFlags {
    if NUMERIC_PAD_CODES.contains(&code) {
        CGEventFlags::CGEventFlagNumericPad
    } else {
        CGEventFlags::empty()
    }
}

/// The portable/native flag pairs this backend maps between, both ways.
const FLAG_PAIRS: [(ModifierFlags, CGEventFlags); 5] = [
    (ModifierFlags::CONTROL, CGEventFlags::CGEventFlagControl),
    (ModifierFlags::COMMAND, CGEventFlags::CGEventFlagCommand),
    (ModifierFlags::ALT, CGEventFlags::CGEventFlagAlternate),
    (ModifierFlags::SHIFT, CGEventFlags::CGEventFlagShift),
    (ModifierFlags::FN, CGEventFlags::CGEventFlagSecondaryFn),
];

/// The native flags for a portable [`ModifierFlags`], for an emitted event.
fn to_cg(flags: ModifierFlags) -> CGEventFlags {
    let mut out = CGEventFlags::empty();
    for (portable, native) in FLAG_PAIRS {
        out.set(native, flags.contains(portable));
    }
    out
}

/// The portable flags an incoming event carries, so a passed-through key keeps a modifier that
/// was baked onto it (an injected `cmd`-`v`, or `fn`) rather than delivered as its own key.
fn from_cg(flags: CGEventFlags) -> ModifierFlags {
    let mut out = ModifierFlags::empty();
    for (portable, native) in FLAG_PAIRS {
        out.set(portable, flags.contains(native));
    }
    out
}

/// Force Caps Lock off after a suppressed `CapsLock` down.
///
/// Physical `CapsLock` toggles the `HIDSystem` latch even when the CG event is `Drop`ped.
/// `CGEventPost` of `CapsLock` does not reverse that latch; [`freddie_hid_device::set_caps_lock`]
/// (`IOHIDSetModifierLockState`) does.
fn clear_caps_latch() {
    match freddie_hid_device::set_caps_lock(false) {
        Ok(()) => tracing::debug!("cleared CapsLock HIDSystem latch"),
        Err(e) => tracing::warn!(?e, "could not clear CapsLock HIDSystem latch"),
    }
}

/// Synthesizes keys through the interceptor's tag, so they are not re-handled.
///
/// `!Send`, because a `CGEventSource` is: it stays on the thread that built it, which is the one
/// that posts through it. Posting mutates a source, so one per posting thread is what this is.
pub struct Emitter {
    tag: Tag,
    /// The one source every event this emitter posts is built from, created in [`run_tap`]. A
    /// source per event would map 16KB of shared memory per keystroke that nothing unmaps;
    /// [`keyboard_event`] ignores the birth flags this one accumulates, so reusing it cannot
    /// reach the wire.
    source: CGEventSource,
}

impl Emitter {
    /// Post `key` going down or coming up, carrying exactly `flags`.
    ///
    /// The event states its own modifiers rather than trusting a source: whoever built it said
    /// what it carries, and we apply exactly that. See [`keyboard_event`].
    ///
    /// The body runs inside an autorelease pool because `CGEventPost` autoreleases two
    /// `CFData`s per call, about 574 bytes. An `Emitter` posts from whatever thread owns it,
    /// which for a daemon is a worker thread with no pool of its own, so a pool here is what
    /// makes a post free what it allocated. Draining per post rather than per batch keeps the
    /// property local to the call that needs it: pushing and popping a pool is tens of
    /// nanoseconds against a post that costs tens of microseconds.
    fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        autoreleasepool(|_pool| {
            let event = keyboard_event(&self.source, key, press, flags)?;
            self.tag.stamp(&event);
            event.post(CGEventTapLocation::Session);
            Ok(())
        })
    }

    /// Emit one key event, a press or a release, carrying `flags`.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError`] if the key has no code on this OS or could not be posted.
    pub fn emit(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        self.post(key, press, flags)
    }

    /// Press then release `key`, both halves carrying `flags`. A chord: `cmd`-`r` is
    /// `tap(Key::KeyR, ModifierFlags::COMMAND)`, the key with the modifier baked into its flags,
    /// so no synthetic modifier event strands a modifier the user is really holding.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError`] if the key has no code on this OS or could not be posted.
    pub fn tap(&self, key: Key, flags: ModifierFlags) -> Result<(), EmitError> {
        self.emit(key, PressType::Down, flags)?;
        self.emit(key, PressType::Up, flags)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Decision, EmitError, Tag, decide, flag_for, from_code, intrinsic_flags, keyboard_event,
        to_cg, to_code,
    };
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventType, KeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};

    fn ev(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        }
    }

    #[test]
    fn named_keys_round_trip() {
        assert_eq!(to_code(Key::KeyR), Some(KeyCode::ANSI_R));
        assert_eq!(from_code(KeyCode::ANSI_R), Key::KeyR);
        assert_eq!(to_code(Key::Escape), Some(KeyCode::ESCAPE));
        assert_eq!(from_code(KeyCode::ESCAPE), Key::Escape);
        assert_eq!(to_code(Key::MetaLeft), Some(KeyCode::COMMAND));
        assert_eq!(from_code(KeyCode::RIGHT_SHIFT), Key::ShiftRight);
    }

    #[test]
    fn unknown_code_becomes_raw() {
        assert_eq!(from_code(64000), Key::Raw(64000));
    }

    #[test]
    fn raw_round_trips_its_code() {
        assert_eq!(to_code(Key::Raw(64000)), Some(64000));
        assert_eq!(from_code(64000), Key::Raw(64000));
    }

    #[test]
    fn keys_without_a_mac_code_are_unmappable() {
        assert_eq!(to_code(Key::F24), None);
        assert_eq!(to_code(Key::Insert), None);
    }

    #[test]
    fn decide_passes_unchanged() {
        let a = ev(Key::KeyA);
        assert_eq!(decide(&a, Some(a.clone())), Decision::Pass);
    }

    #[test]
    fn decide_remaps_a_different_key() {
        let a = ev(Key::KeyA);
        let b = ev(Key::KeyB);
        assert_eq!(decide(&a, Some(b.clone())), Decision::Remap(b));
    }

    #[test]
    fn decide_drops_on_none() {
        assert_eq!(decide(&ev(Key::KeyA), None), Decision::Drop);
    }

    #[test]
    fn decide_remaps_when_only_press_changes() {
        let down = ev(Key::KeyA);
        let up = KeyEvent {
            key: Key::KeyA,
            press: PressType::Up,
            flags: ModifierFlags::empty(),
        };
        assert_eq!(decide(&down, Some(up.clone())), Decision::Remap(up));
    }

    // The six device-independent modifier bits. Only the tests name them now: production names
    // the bits it emits, in `to_cg` and `intrinsic_flags`, rather than subtracting the rest.
    const MODIFIERS: CGEventFlags = CGEventFlags::from_bits_truncate(
        CGEventFlags::CGEventFlagAlphaShift.bits()
            | CGEventFlags::CGEventFlagShift.bits()
            | CGEventFlags::CGEventFlagControl.bits()
            | CGEventFlags::CGEventFlagAlternate.bits()
            | CGEventFlags::CGEventFlagCommand.bits()
            | CGEventFlags::CGEventFlagSecondaryFn.bits(),
    );

    fn private_source() -> CGEventSource {
        CGEventSource::new(CGEventSourceStateID::Private).expect("a private source")
    }

    // Only the arrows and the keypad are born carrying NumericPad, measured against a clean
    // private source. The second list is the trap: those keys read like the arrows and do not
    // carry it, so naming them would put a bit on the wire that the key does not have.
    #[test]
    fn only_the_arrows_and_the_keypad_are_intrinsically_numeric_pad() {
        for code in [
            KeyCode::UP_ARROW,
            KeyCode::LEFT_ARROW,
            KeyCode::ANSI_KEYPAD_ENTER,
            KeyCode::ANSI_KEYPAD_7,
        ] {
            assert_eq!(
                intrinsic_flags(code),
                CGEventFlags::CGEventFlagNumericPad,
                "keycode {code} carries NumericPad on a clean source"
            );
        }
        for code in [
            KeyCode::HOME,
            KeyCode::END,
            KeyCode::PAGE_UP,
            KeyCode::PAGE_DOWN,
            KeyCode::ANSI_KEYPAD_CLEAR,
            KeyCode::SPACE,
            KeyCode::ANSI_A,
        ] {
            assert_eq!(
                intrinsic_flags(code),
                CGEventFlags::empty(),
                "keycode {code} carries nothing outside MODIFIERS"
            );
        }
    }

    // The whole point of the change: what goes on the wire is a function of the key and the
    // portable flags, and of nothing the source is holding. Exact equality, so a reintroduced
    // `| (get_flags() & !MODIFIERS)` fails here rather than in Spotlight six hours later.
    #[test]
    fn the_wire_flags_are_the_portable_ones_plus_the_intrinsic_ones() {
        let source = private_source();
        for (key, code, flags) in [
            (Key::Space, KeyCode::SPACE, ModifierFlags::COMMAND),
            (Key::UpArrow, KeyCode::UP_ARROW, ModifierFlags::empty()),
            (Key::UpArrow, KeyCode::UP_ARROW, ModifierFlags::COMMAND),
            (Key::KeyR, KeyCode::ANSI_R, ModifierFlags::CONTROL),
        ] {
            let event =
                keyboard_event(&source, key, PressType::Down, flags).expect("an event for the key");
            assert_eq!(
                event.get_flags(),
                to_cg(flags) | intrinsic_flags(code),
                "{key:?} with {flags:?}"
            );
        }
    }

    // A chord carries only its own modifier: an arrow must not leave NumericPad on a later
    // `cmd`-`space`, or Spotlight's hotkey posts 0x00300000 instead of 0x00100000.
    #[test]
    fn a_chord_carries_its_modifier_and_nothing_else() {
        let space = keyboard_event(
            &private_source(),
            Key::Space,
            PressType::Down,
            ModifierFlags::COMMAND,
        )
        .expect("a space");
        assert_eq!(
            space.get_flags() & MODIFIERS,
            CGEventFlags::CGEventFlagCommand
        );
        assert!(
            !space
                .get_flags()
                .contains(CGEventFlags::CGEventFlagNumericPad)
        );
    }

    // A key's own flags survive: `intrinsic_flags` names NumericPad for the arrows, so an arrow
    // keeps the bit it is born with while a space never gains one.
    #[test]
    fn a_keys_own_flags_survive_and_others_do_not_appear() {
        let source = private_source();
        let arrow = keyboard_event(
            &source,
            Key::UpArrow,
            PressType::Down,
            ModifierFlags::empty(),
        )
        .expect("an arrow");
        let space = keyboard_event(&source, Key::Space, PressType::Down, ModifierFlags::empty())
            .expect("a space");
        assert!(
            arrow
                .get_flags()
                .contains(CGEventFlags::CGEventFlagNumericPad)
        );
        assert!(
            !space
                .get_flags()
                .contains(CGEventFlags::CGEventFlagNumericPad)
        );
    }

    // A keypad key has no `Key` variant and arrives as `Key::Raw`, which is the case a table of
    // variants would miss: it must still post the NumericPad bit it is born with.
    #[test]
    fn a_raw_keypad_key_keeps_its_numeric_pad_bit() {
        let event = keyboard_event(
            &private_source(),
            Key::Raw(KeyCode::ANSI_KEYPAD_ENTER),
            PressType::Down,
            ModifierFlags::empty(),
        )
        .expect("a keypad enter");
        assert_eq!(event.get_flags(), CGEventFlags::CGEventFlagNumericPad);
    }

    // A remapped key carries the flags it was given, not whatever a shared source baked in.
    #[test]
    fn a_remapped_key_carries_the_flags_it_was_given() {
        let event = keyboard_event(
            &private_source(),
            Key::KeyR,
            PressType::Down,
            ModifierFlags::COMMAND,
        )
        .expect("a key");
        assert!(event.get_flags().contains(CGEventFlags::CGEventFlagCommand));
    }

    // The OS picks the type from the keycode: a modifier is FlagsChanged, anything else KeyDown/Up.
    #[test]
    fn a_modifier_is_a_flags_changed_and_a_key_is_not() {
        let source = private_source();
        let cmd = keyboard_event(
            &source,
            Key::MetaLeft,
            PressType::Down,
            ModifierFlags::COMMAND,
        )
        .expect("cmd");
        let space = keyboard_event(&source, Key::Space, PressType::Down, ModifierFlags::empty())
            .expect("a space");
        assert!(matches!(cmd.get_type(), CGEventType::FlagsChanged));
        assert!(matches!(space.get_type(), CGEventType::KeyDown));
    }

    #[test]
    fn a_key_with_no_code_is_unmappable() {
        assert!(matches!(
            keyboard_event(
                &private_source(),
                Key::F24,
                PressType::Down,
                ModifierFlags::empty()
            ),
            Err(EmitError::Unmappable(Key::F24))
        ));
    }

    // The tag is what keeps the interceptor from handling its own emissions, so it must mark
    // an event it stamped and no other.
    #[test]
    fn a_tag_marks_only_its_own_events() {
        let source = CGEventSource::new(CGEventSourceStateID::Private).expect("a private source");
        let event =
            CGEvent::new_keyboard_event(source, KeyCode::SPACE, true).expect("a keyboard event");
        let (mine, theirs) = (Tag::new(), Tag(1));
        assert!(!mine.marks(&event));
        mine.stamp(&event);
        assert!(mine.marks(&event));
        assert!(!theirs.marks(&event));
    }

    #[test]
    fn flags_map_modifiers_only() {
        assert_eq!(
            flag_for(Key::MetaLeft),
            Some(CGEventFlags::CGEventFlagCommand)
        );
        assert_eq!(
            flag_for(Key::ShiftRight),
            Some(CGEventFlags::CGEventFlagShift)
        );
        assert_eq!(
            flag_for(Key::ControlLeft),
            Some(CGEventFlags::CGEventFlagControl)
        );
        assert_eq!(
            flag_for(Key::AltRight),
            Some(CGEventFlags::CGEventFlagAlternate)
        );
        assert_eq!(
            flag_for(Key::CapsLock),
            Some(CGEventFlags::CGEventFlagAlphaShift)
        );
        assert_eq!(flag_for(Key::KeyA), None);
        assert_eq!(flag_for(Key::Escape), None);
    }
}
