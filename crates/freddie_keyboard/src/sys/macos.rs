//! The macOS backend, on `core-graphics`. The pure parts (the keycode table, the
//! pass/remap/drop decision, the modifier flags) are unit-tested below; the tap
//! and the posting are FFI that needs a real keyboard to exercise.

use std::hash::{BuildHasher, Hasher, RandomState};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::JoinHandle;

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CGKeyCode, CallbackResult, EventField, KeyCode,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};

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

fn new_tag() -> i64 {
    // Per-process random, so an interceptor skips only its own emitter's output.
    let mut h = RandomState::new().build_hasher();
    h.write_u8(0);
    i64::from_ne_bytes(h.finish().to_ne_bytes())
}

fn press_of(kind: CGEventType, event: &CGEvent) -> Option<PressType> {
    match kind {
        CGEventType::KeyDown => Some(PressType::Down),
        CGEventType::KeyUp => Some(PressType::Up),
        // A modifier: down if its flag bit is set after the change.
        CGEventType::FlagsChanged => {
            let code =
                u16::try_from(event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE))
                    .ok()?;
            let flag = flag_for(from_code(code))?;
            Some(if event.get_flags().contains(flag) {
                PressType::Down
            } else {
                PressType::Up
            })
        }
        _ => None,
    }
}

fn encode(source: Option<&CGEventSource>, out: &KeyEvent) -> Option<CGEvent> {
    let code = to_code(out.key)?;
    let down = out.press == PressType::Down;
    CGEvent::new_keyboard_event(source?.clone(), code, down).ok()
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
    let tag = new_tag();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<CFRunLoop, ()>>();
    let signal = ready_tx.clone();

    let thread = std::thread::spawn(move || {
        let source = CGEventSource::new(CGEventSourceStateID::Private).ok();
        let outcome = CGEventTap::with_enabled(
            CGEventTapLocation::Session,
            CGEventTapPlacement::TailAppendEventTap,
            CGEventTapOptions::Default,
            vec![
                CGEventType::KeyDown,
                CGEventType::KeyUp,
                CGEventType::FlagsChanged,
            ],
            |_proxy, kind, event| {
                if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == tag {
                    return CallbackResult::Keep; // our own emit
                }
                let Some(press) = press_of(kind, event) else {
                    return CallbackResult::Keep;
                };
                let Ok(code) = u16::try_from(
                    event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
                ) else {
                    return CallbackResult::Keep;
                };
                let input = KeyEvent {
                    key: from_code(code),
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
                match decide(&input, on_key(input.clone())) {
                    Decision::Pass => CallbackResult::Keep,
                    Decision::Drop => CallbackResult::Drop,
                    Decision::Remap(out) => encode(source.as_ref(), &out)
                        .map_or(CallbackResult::Drop, CallbackResult::Replace),
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
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| CaptureError)?;
    let interceptor = Interceptor {
        run_loop,
        thread: Some(thread),
    };
    let emitter = Emitter(Rc::new(EmitterState { source, tag }));
    Ok((interceptor, emitter))
}

/// An active grab of the keyboard. While it is alive keys are intercepted;
/// dropping it releases the keyboard.
pub struct Interceptor {
    run_loop: CFRunLoop,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Interceptor {
    fn drop(&mut self) {
        self.run_loop.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The device-independent modifier bits. Everything else the OS puts in an event's
/// flags is left as it found it.
const MODIFIERS: CGEventFlags = CGEventFlags::from_bits_truncate(
    CGEventFlags::CGEventFlagAlphaShift.bits()
        | CGEventFlags::CGEventFlagShift.bits()
        | CGEventFlags::CGEventFlagControl.bits()
        | CGEventFlags::CGEventFlagAlternate.bits()
        | CGEventFlags::CGEventFlagCommand.bits()
        | CGEventFlags::CGEventFlagSecondaryFn.bits(),
);

/// The portable/native flag pairs mercury maps between, both ways.
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

struct EmitterState {
    source: CGEventSource,
    tag: i64,
}

impl EmitterState {
    /// Post `key` going down or coming up, carrying exactly `flags`.
    ///
    /// A `CGEvent`'s own flags are baked in from the source's state when it is created, which
    /// lags a modifier posted microseconds earlier. So the event states its own modifiers rather
    /// than trusting the source: whoever built it said what it carries, and we apply exactly that.
    fn post(&self, key: Key, down: bool, flags: ModifierFlags) -> Result<(), EmitError> {
        let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
        let event = CGEvent::new_keyboard_event(self.source.clone(), code, down)
            .map_err(|_| EmitError::Post)?;
        let untouched = event.get_flags() & !MODIFIERS;
        event.set_flags(untouched | to_cg(flags));
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.tag);
        event.post(CGEventTapLocation::Session);
        Ok(())
    }
}

/// Synthesizes keys through the interceptor's tag, so they are not re-handled.
pub struct Emitter(Rc<EmitterState>);

impl Emitter {
    /// Emit one key event, a press or a release, carrying `flags`.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError`] if the key has no code on this OS or could not be posted.
    pub fn emit(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        self.0.post(key, press == PressType::Down, flags)
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
    use super::{Decision, decide, flag_for, from_code, to_code};
    use core_graphics::event::{CGEventFlags, KeyCode};
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
        assert_eq!(flag_for(Key::KeyA), None);
        assert_eq!(flag_for(Key::Escape), None);
    }
}
