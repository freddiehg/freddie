//! The macOS backend, on `core-graphics`. The pure parts (the keycode table, the
//! pass/remap/drop decision, the modifier flags) are unit-tested below; the tap
//! and the posting are FFI that needs a real keyboard to exercise.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
use freddie_keys::{Key, KeyEvent, PressType};

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
                };
                // Source PID for telling injected events (a userspace `CGEventPost`, nonzero PID)
                // from physical HID input (PID 0). Logging only for now, to confirm the split
                // before acting on it (see the "pass through injected events" plan).
                let source_pid =
                    event.get_integer_value_field(EventField::EVENT_SOURCE_UNIX_PROCESS_ID);
                tracing::debug!(key = ?input.key, ?press, source_pid, "tap key");
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
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|()| CaptureError)?;
    let interceptor = Interceptor {
        run_loop,
        thread: Some(thread),
    };
    let emitter = Emitter(Rc::new(EmitterState {
        source,
        tag,
        held: RefCell::new(HashMap::new()),
        flags: Cell::new(CGEventFlags::CGEventFlagNull),
    }));
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

/// The modifier bit a key sets while it is down, if it is a modifier at all.
const fn flag_of(key: Key) -> Option<CGEventFlags> {
    Some(match key {
        Key::ShiftLeft | Key::ShiftRight => CGEventFlags::CGEventFlagShift,
        Key::ControlLeft | Key::ControlRight => CGEventFlags::CGEventFlagControl,
        Key::AltLeft | Key::AltRight => CGEventFlags::CGEventFlagAlternate,
        Key::MetaLeft | Key::MetaRight => CGEventFlags::CGEventFlagCommand,
        Key::CapsLock => CGEventFlags::CGEventFlagAlphaShift,
        _ => return None,
    })
}

/// The flags after `key` goes down or comes up. A non-modifier changes nothing.
fn next_flags(current: CGEventFlags, key: Key, down: bool) -> CGEventFlags {
    let mut flags = current;
    if let Some(flag) = flag_of(key) {
        flags.set(flag, down);
    }
    flags
}

struct EmitterState {
    source: CGEventSource,
    tag: i64,
    held: RefCell<HashMap<Key, usize>>,
    /// The modifiers this emitter believes are down, from what it has emitted.
    ///
    /// A `CGEvent`'s modifier flags are baked in when it is created, from the event
    /// source's state, and that state only catches up once a posted modifier key
    /// has been *processed*. So a chord posted back to back, `ctrl` down then `a`,
    /// creates the `a` microseconds later and it carries no `ctrl`. Tracking the
    /// flags here and setting them on every event makes the emitted stream say what
    /// it means, whatever the source thinks and however fast we post.
    flags: Cell<CGEventFlags>,
}

impl EmitterState {
    /// Emit `key`, going down or coming up, with the modifiers we are holding.
    fn emit(&self, key: Key, down: bool) -> Result<(), EmitError> {
        let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
        // A modifier's own event carries its new state: `ctrl` down has the control
        // bit set, `ctrl` up has it clear, which is what a real one looks like.
        self.flags.set(next_flags(self.flags.get(), key, down));
        self.post(code, down)
    }

    fn post(&self, code: CGKeyCode, down: bool) -> Result<(), EmitError> {
        let event = CGEvent::new_keyboard_event(self.source.clone(), code, down)
            .map_err(|()| EmitError::Post)?;
        let untouched = event.get_flags() & !MODIFIERS;
        event.set_flags(untouched | self.flags.get());
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.tag);
        event.post(CGEventTapLocation::Session);
        Ok(())
    }
}

/// Synthesizes keys through the interceptor's tag, so they are not re-handled.
pub struct Emitter(Rc<EmitterState>);

impl Emitter {
    /// Emit one key event, a press or a release.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError`] if the key has no code on this OS or could not be posted.
    pub fn emit(&self, key: Key, press: PressType) -> Result<(), EmitError> {
        self.0.emit(key, press == PressType::Down)
    }

    /// Press then release a key.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError`] if the key has no code on this OS or could not be posted.
    pub fn tap(&self, key: Key) -> Result<(), EmitError> {
        self.emit(key, PressType::Down)?;
        self.emit(key, PressType::Up)
    }

    /// Hold `modifiers` down for the duration of `f`, then release them in reverse.
    ///
    /// This is how a chord should be emitted. Holding is structural, so the release
    /// cannot be forgotten, and a key cannot carry a modifier that was never
    /// pressed: the flags an event gets are the modifiers this emitter is holding.
    ///
    /// ```ignore
    /// emitter.with_modifiers(&[Key::MetaLeft], |e| e.tap(Key::KeyR))?; // cmd-r
    /// ```
    ///
    /// The modifiers are released even if `f` fails.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError`] if a modifier or the key has no code on this OS, or
    /// could not be posted. Propagates whatever `f` returns.
    pub fn with_modifiers<F, R>(&self, modifiers: &[Key], f: F) -> Result<R, EmitError>
    where
        F: FnOnce(&Self) -> Result<R, EmitError>,
    {
        let mut held = Vec::with_capacity(modifiers.len());
        for &modifier in modifiers {
            held.push(self.press(modifier)?);
        }
        let out = f(self);
        // Reverse order, the way a person lets go: the last pressed is the first
        // released. `Vec`'s own drop would go the other way.
        while held.pop().is_some() {}
        out
    }

    /// Hold a key down until the last [`Held`] for it drops. Ref-counted per key.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError`] if the key has no code on this OS or could not be posted.
    pub fn press(&self, key: Key) -> Result<Held, EmitError> {
        to_code(key).ok_or(EmitError::Unmappable(key))?;
        let mut held = self.0.held.borrow_mut();
        let count = held.entry(key).or_insert(0);
        if *count == 0 {
            self.0.emit(key, true)?;
        }
        *count += 1;
        drop(held);
        Ok(Held {
            state: Rc::clone(&self.0),
            key,
        })
    }
}

/// A held key. The key stays down while any `Held` for it is alive.
pub struct Held {
    state: Rc<EmitterState>,
    key: Key,
}

impl Clone for Held {
    fn clone(&self) -> Self {
        *self.state.held.borrow_mut().entry(self.key).or_insert(0) += 1;
        Self {
            state: Rc::clone(&self.state),
            key: self.key,
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut held = self.state.held.borrow_mut();
        if let Some(count) = held.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                let _ = self.state.emit(self.key, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MODIFIERS, flag_of, next_flags};

    #[test]
    fn only_modifiers_have_a_flag() {
        assert_eq!(
            flag_of(Key::ControlLeft),
            Some(CGEventFlags::CGEventFlagControl)
        );
        assert_eq!(
            flag_of(Key::ControlRight),
            Some(CGEventFlags::CGEventFlagControl)
        );
        assert_eq!(
            flag_of(Key::MetaLeft),
            Some(CGEventFlags::CGEventFlagCommand)
        );
        assert_eq!(flag_of(Key::KeyA), None);
        assert_eq!(flag_of(Key::KeyP), None);
    }

    #[test]
    fn a_modifier_sets_its_flag_while_it_is_down() {
        let none = CGEventFlags::CGEventFlagNull;
        let down = next_flags(none, Key::ControlLeft, true);
        assert!(down.contains(CGEventFlags::CGEventFlagControl));
        let up = next_flags(down, Key::ControlLeft, false);
        assert!(!up.contains(CGEventFlags::CGEventFlagControl));
    }

    #[test]
    fn an_ordinary_key_leaves_the_flags_alone() {
        let ctrl = next_flags(CGEventFlags::CGEventFlagNull, Key::ControlLeft, true);
        assert_eq!(next_flags(ctrl, Key::KeyA, true), ctrl);
        assert_eq!(next_flags(ctrl, Key::KeyA, false), ctrl);
    }

    /// The whole point: through `ctrl` down, `a` down, `a` up, `ctrl` up, `p` down,
    /// the `a` carries control and the `p` does not. Relying on the event source to
    /// notice the posted `ctrl` leaves the `a` bare, because a chord is posted far
    /// faster than the source is updated.
    #[test]
    fn the_tmux_prefix_carries_control_and_the_command_does_not() {
        let mut flags = CGEventFlags::CGEventFlagNull;
        let ctrl = CGEventFlags::CGEventFlagControl;

        flags = next_flags(flags, Key::ControlLeft, true);
        assert!(flags.contains(ctrl), "ctrl down carries control");

        flags = next_flags(flags, Key::KeyA, true);
        assert!(flags.contains(ctrl), "a is pressed with control held");
        flags = next_flags(flags, Key::KeyA, false);
        assert!(
            flags.contains(ctrl),
            "a is released with control still held"
        );

        flags = next_flags(flags, Key::ControlLeft, false);
        assert!(!flags.contains(ctrl), "ctrl up clears control");

        flags = next_flags(flags, Key::KeyP, true);
        assert!(!flags.contains(ctrl), "p is a bare p, not ctrl-p");
    }

    #[test]
    fn several_modifiers_stack_and_unstack_independently() {
        let mut flags = CGEventFlags::CGEventFlagNull;
        flags = next_flags(flags, Key::MetaLeft, true);
        flags = next_flags(flags, Key::ShiftLeft, true);
        assert!(flags.contains(CGEventFlags::CGEventFlagCommand));
        assert!(flags.contains(CGEventFlags::CGEventFlagShift));

        flags = next_flags(flags, Key::ShiftLeft, false);
        assert!(flags.contains(CGEventFlags::CGEventFlagCommand));
        assert!(!flags.contains(CGEventFlags::CGEventFlagShift));
    }

    #[test]
    fn the_modifier_mask_covers_every_flag_a_key_can_set() {
        for key in [
            Key::ShiftLeft,
            Key::ShiftRight,
            Key::ControlLeft,
            Key::ControlRight,
            Key::AltLeft,
            Key::AltRight,
            Key::MetaLeft,
            Key::MetaRight,
            Key::CapsLock,
        ] {
            let flag = flag_of(key).expect("a modifier");
            assert!(MODIFIERS.contains(flag), "{key:?} escapes the mask");
        }
    }

    use super::{Decision, decide, flag_for, from_code, to_code};
    use core_graphics::event::{CGEventFlags, KeyCode};
    use freddie_keys::{Key, KeyEvent, PressType};

    fn ev(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            press: PressType::Down,
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
