//! The macOS backend, on `core-graphics`. The pure parts (the keycode table, the
//! pass/remap/drop decision, the modifier flags) are unit-tested below; the tap
//! and the posting are FFI that needs a real keyboard to exercise.

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

/// A keyboard event for `key`, carrying exactly `flags`, built from a source of its own.
///
/// The source is created here and dropped with the event, and is deliberately not stored
/// anywhere. Posting through a source mutates it: an arrow key leaves `NumericPad` in its
/// state, every event built from that source afterwards is born carrying that bit, and posting
/// one writes the bit back, which reaffirms it. One arrow key would otherwise stop `cmd`-`space`
/// from being the Spotlight hotkey for the rest of the run. A source with no history has nothing
/// to leak, so the event is born with exactly the flags its own key carries, and `untouched`
/// keeps only those.
///
/// Not a `NULL` source, which means the shared session state rather than no state, and so
/// inherits bits other processes have left there.
///
/// # Errors
///
/// Returns [`EmitError::Unmappable`] if the key has no code on this OS, and [`EmitError::Post`]
/// if the OS refused to build the source or the event.
fn keyboard_event(key: Key, press: PressType, flags: ModifierFlags) -> Result<CGEvent, EmitError> {
    let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| EmitError::Post)?;
    let event = CGEvent::new_keyboard_event(source, code, press == PressType::Down)
        .map_err(|_| EmitError::Post)?;
    let untouched = event.get_flags() & !MODIFIERS;
    event.set_flags(untouched | to_cg(flags));
    // What actually goes on the wire, which the portable `KeyEvent` cannot show: the raw flag
    // bits, what the source supplied, and the type the OS chose from the keycode
    // (`FlagsChanged` for a modifier, `KeyDown`/`KeyUp` otherwise). At `debug` so the log file
    // keeps it, since two presses that dispatch identically can still post differently.
    tracing::debug!(
        ?key,
        ?press,
        raw_flags = %format!("{:#010x}", event.get_flags().bits()),
        kept_from_source = %format!("{:#010x}", untouched.bits()),
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
    let tag = Tag::new();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<CFRunLoop, ()>>();
    let signal = ready_tx.clone();

    let thread = std::thread::spawn(move || {
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
                if tag.marks(event) {
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
                    Decision::Remap(out) => match keyboard_event(out.key, out.press, out.flags) {
                        Ok(event) => CallbackResult::Replace(event),
                        Err(e) => {
                            tracing::warn!(key = ?out.key, error = %e, "dropped a remapped key");
                            CallbackResult::Drop
                        }
                    },
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
    let emitter = Emitter { tag };
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

/// Synthesizes keys through the interceptor's tag, so they are not re-handled.
pub struct Emitter {
    tag: Tag,
}

impl Emitter {
    /// Post `key` going down or coming up, carrying exactly `flags`.
    ///
    /// The event states its own modifiers rather than trusting a source: whoever built it said
    /// what it carries, and we apply exactly that. See [`keyboard_event`].
    fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        let event = keyboard_event(key, press, flags)?;
        self.tag.stamp(&event);
        event.post(CGEventTapLocation::Session);
        Ok(())
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
        Decision, EmitError, MODIFIERS, Tag, decide, flag_for, from_code, keyboard_event, to_code,
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

    // The bug this constructor exists to prevent: an arrow key leaves NumericPad in a source's
    // state, so a `cmd`-`space` built from that source later posts 0x00300000 rather than
    // 0x00100000 and is no longer the Spotlight hotkey. A source per event cannot carry it.
    #[test]
    fn a_chord_carries_its_modifier_and_nothing_else() {
        let space =
            keyboard_event(Key::Space, PressType::Down, ModifierFlags::COMMAND).expect("a space");
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

    // A key's own flags survive: `MODIFIERS` deliberately does not name NumericPad, so an arrow
    // keeps the bit it is born with while a space never gains one.
    #[test]
    fn a_keys_own_flags_survive_and_others_do_not_appear() {
        let arrow = keyboard_event(Key::UpArrow, PressType::Down, ModifierFlags::empty())
            .expect("an arrow");
        let space =
            keyboard_event(Key::Space, PressType::Down, ModifierFlags::empty()).expect("a space");
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

    // `encode` dropped the flags it was handed, so a remapped key carried whatever the source
    // had baked in. Both paths share the constructor now.
    #[test]
    fn a_remapped_key_carries_the_flags_it_was_given() {
        let event =
            keyboard_event(Key::KeyR, PressType::Down, ModifierFlags::COMMAND).expect("a key");
        assert!(event.get_flags().contains(CGEventFlags::CGEventFlagCommand));
    }

    // The OS picks the type from the keycode; the emitter neither asks for this nor needs to.
    #[test]
    fn a_modifier_is_a_flags_changed_and_a_key_is_not() {
        let cmd =
            keyboard_event(Key::MetaLeft, PressType::Down, ModifierFlags::COMMAND).expect("cmd");
        let space =
            keyboard_event(Key::Space, PressType::Down, ModifierFlags::empty()).expect("a space");
        assert!(matches!(cmd.get_type(), CGEventType::FlagsChanged));
        assert!(matches!(space.get_type(), CGEventType::KeyDown));
    }

    #[test]
    fn a_key_with_no_code_is_unmappable() {
        assert!(matches!(
            keyboard_event(Key::F24, PressType::Down, ModifierFlags::empty()),
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
        assert_eq!(flag_for(Key::KeyA), None);
        assert_eq!(flag_for(Key::Escape), None);
    }
}
