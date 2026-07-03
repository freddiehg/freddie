//! The macOS keyboard backend, on `core-graphics`.
//!
//! `run` installs an active `CGEventTap` and swallows every key. `emit` posts key
//! events from a private source, stamped so the tap passes them through instead of
//! re-dispatching them. All of this is safe `core-graphics`, so the crate stays
//! inside the workspace `forbid(unsafe_code)`.

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CGKeyCode, CallbackResult, EventField, KeyCode,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use freddie_keys::{KeyEvent, Keyboard};

use crate::{CaptureError, EmitError};

// Stamped on every key we emit so our own tap passes it through instead of
// re-dispatching it. rdev's counter guessed the next two callbacks were ours; a
// real field is exact.
const TAG: i64 = 0x4652_4544; // "FRED"

// Keyboard to macOS virtual key code. Keys with no macOS code (F21-F24, Insert)
// are absent, so they map to `None` both ways.
const TABLE: &[(Keyboard, CGKeyCode)] = &[
    (Keyboard::KeyA, KeyCode::ANSI_A),
    (Keyboard::KeyB, KeyCode::ANSI_B),
    (Keyboard::KeyC, KeyCode::ANSI_C),
    (Keyboard::KeyD, KeyCode::ANSI_D),
    (Keyboard::KeyE, KeyCode::ANSI_E),
    (Keyboard::KeyF, KeyCode::ANSI_F),
    (Keyboard::KeyG, KeyCode::ANSI_G),
    (Keyboard::KeyH, KeyCode::ANSI_H),
    (Keyboard::KeyI, KeyCode::ANSI_I),
    (Keyboard::KeyJ, KeyCode::ANSI_J),
    (Keyboard::KeyK, KeyCode::ANSI_K),
    (Keyboard::KeyL, KeyCode::ANSI_L),
    (Keyboard::KeyM, KeyCode::ANSI_M),
    (Keyboard::KeyN, KeyCode::ANSI_N),
    (Keyboard::KeyO, KeyCode::ANSI_O),
    (Keyboard::KeyP, KeyCode::ANSI_P),
    (Keyboard::KeyQ, KeyCode::ANSI_Q),
    (Keyboard::KeyR, KeyCode::ANSI_R),
    (Keyboard::KeyS, KeyCode::ANSI_S),
    (Keyboard::KeyT, KeyCode::ANSI_T),
    (Keyboard::KeyU, KeyCode::ANSI_U),
    (Keyboard::KeyV, KeyCode::ANSI_V),
    (Keyboard::KeyW, KeyCode::ANSI_W),
    (Keyboard::KeyX, KeyCode::ANSI_X),
    (Keyboard::KeyY, KeyCode::ANSI_Y),
    (Keyboard::KeyZ, KeyCode::ANSI_Z),
    (Keyboard::Num0, KeyCode::ANSI_0),
    (Keyboard::Num1, KeyCode::ANSI_1),
    (Keyboard::Num2, KeyCode::ANSI_2),
    (Keyboard::Num3, KeyCode::ANSI_3),
    (Keyboard::Num4, KeyCode::ANSI_4),
    (Keyboard::Num5, KeyCode::ANSI_5),
    (Keyboard::Num6, KeyCode::ANSI_6),
    (Keyboard::Num7, KeyCode::ANSI_7),
    (Keyboard::Num8, KeyCode::ANSI_8),
    (Keyboard::Num9, KeyCode::ANSI_9),
    (Keyboard::F1, KeyCode::F1),
    (Keyboard::F2, KeyCode::F2),
    (Keyboard::F3, KeyCode::F3),
    (Keyboard::F4, KeyCode::F4),
    (Keyboard::F5, KeyCode::F5),
    (Keyboard::F6, KeyCode::F6),
    (Keyboard::F7, KeyCode::F7),
    (Keyboard::F8, KeyCode::F8),
    (Keyboard::F9, KeyCode::F9),
    (Keyboard::F10, KeyCode::F10),
    (Keyboard::F11, KeyCode::F11),
    (Keyboard::F12, KeyCode::F12),
    (Keyboard::F13, KeyCode::F13),
    (Keyboard::F14, KeyCode::F14),
    (Keyboard::F15, KeyCode::F15),
    (Keyboard::F16, KeyCode::F16),
    (Keyboard::F17, KeyCode::F17),
    (Keyboard::F18, KeyCode::F18),
    (Keyboard::F19, KeyCode::F19),
    (Keyboard::F20, KeyCode::F20),
    (Keyboard::Escape, KeyCode::ESCAPE),
    (Keyboard::Return, KeyCode::RETURN),
    (Keyboard::Space, KeyCode::SPACE),
    (Keyboard::Tab, KeyCode::TAB),
    (Keyboard::Backspace, KeyCode::DELETE),
    (Keyboard::Delete, KeyCode::FORWARD_DELETE),
    (Keyboard::CapsLock, KeyCode::CAPS_LOCK),
    (Keyboard::UpArrow, KeyCode::UP_ARROW),
    (Keyboard::DownArrow, KeyCode::DOWN_ARROW),
    (Keyboard::LeftArrow, KeyCode::LEFT_ARROW),
    (Keyboard::RightArrow, KeyCode::RIGHT_ARROW),
    (Keyboard::Home, KeyCode::HOME),
    (Keyboard::End, KeyCode::END),
    (Keyboard::PageUp, KeyCode::PAGE_UP),
    (Keyboard::PageDown, KeyCode::PAGE_DOWN),
    (Keyboard::ShiftLeft, KeyCode::SHIFT),
    (Keyboard::ShiftRight, KeyCode::RIGHT_SHIFT),
    (Keyboard::ControlLeft, KeyCode::CONTROL),
    (Keyboard::ControlRight, KeyCode::RIGHT_CONTROL),
    (Keyboard::AltLeft, KeyCode::OPTION),
    (Keyboard::AltRight, KeyCode::RIGHT_OPTION),
    (Keyboard::MetaLeft, KeyCode::COMMAND),
    (Keyboard::MetaRight, KeyCode::RIGHT_COMMAND),
    (Keyboard::Grave, KeyCode::ANSI_GRAVE),
    (Keyboard::Minus, KeyCode::ANSI_MINUS),
    (Keyboard::Equal, KeyCode::ANSI_EQUAL),
    (Keyboard::LeftBracket, KeyCode::ANSI_LEFT_BRACKET),
    (Keyboard::RightBracket, KeyCode::ANSI_RIGHT_BRACKET),
    (Keyboard::BackSlash, KeyCode::ANSI_BACKSLASH),
    (Keyboard::SemiColon, KeyCode::ANSI_SEMICOLON),
    (Keyboard::Quote, KeyCode::ANSI_QUOTE),
    (Keyboard::Comma, KeyCode::ANSI_COMMA),
    (Keyboard::Dot, KeyCode::ANSI_PERIOD),
    (Keyboard::Slash, KeyCode::ANSI_SLASH),
];

fn to_code(key: Keyboard) -> Option<CGKeyCode> {
    TABLE.iter().find(|(k, _)| *k == key).map(|(_, code)| *code)
}

fn from_code(code: CGKeyCode) -> Option<Keyboard> {
    TABLE.iter().find(|(_, c)| *c == code).map(|(key, _)| *key)
}

const fn flag_for(key: Keyboard) -> Option<CGEventFlags> {
    Some(match key {
        Keyboard::MetaLeft | Keyboard::MetaRight => CGEventFlags::CGEventFlagCommand,
        Keyboard::ShiftLeft | Keyboard::ShiftRight => CGEventFlags::CGEventFlagShift,
        Keyboard::AltLeft | Keyboard::AltRight => CGEventFlags::CGEventFlagAlternate,
        Keyboard::ControlLeft | Keyboard::ControlRight => CGEventFlags::CGEventFlagControl,
        _ => return None,
    })
}

pub fn run(on_key: impl Fn(KeyEvent) + Send + 'static) -> Result<(), CaptureError> {
    CGEventTap::with_enabled(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![CGEventType::KeyDown, CGEventType::KeyUp],
        |_proxy, event_type, event| {
            let down = match event_type {
                CGEventType::KeyDown => true,
                CGEventType::KeyUp => false,
                // TapDisabled* and anything else: we cannot act on it here, so
                // leave it alone.
                _ => return CallbackResult::Keep,
            };
            // Our own emitted key: pass it, do not re-dispatch.
            if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == TAG {
                return CallbackResult::Keep;
            }
            let Ok(code) =
                u16::try_from(event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE))
            else {
                return CallbackResult::Keep;
            };
            // A key we do not name: pass it rather than silently eat it.
            from_code(code).map_or(CallbackResult::Keep, |key| {
                on_key(KeyEvent { key, down });
                CallbackResult::Drop
            })
        },
        CFRunLoop::run_current,
    )
    .map_err(|()| CaptureError)
}

pub fn emit(key: Keyboard) -> Result<(), EmitError> {
    let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
    let source = source()?;
    post(&source, code, CGEventFlags::empty(), true)?;
    post(&source, code, CGEventFlags::empty(), false)
}

pub fn emit_chord(mods: &[Keyboard], key: Keyboard) -> Result<(), EmitError> {
    let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
    let mut flags = CGEventFlags::empty();
    for &m in mods {
        flags |= flag_for(m).ok_or(EmitError::Unmappable(m))?;
    }
    let source = source()?;
    post(&source, code, flags, true)?;
    post(&source, code, flags, false)
}

fn source() -> Result<CGEventSource, EmitError> {
    CGEventSource::new(CGEventSourceStateID::Private).map_err(|()| EmitError::Source)
}

fn post(source: &CGEventSource, code: CGKeyCode, flags: CGEventFlags, down: bool) -> Result<(), EmitError> {
    let event = CGEvent::new_keyboard_event(source.clone(), code, down).map_err(|()| EmitError::Post)?;
    if !flags.is_empty() {
        event.set_flags(flags);
    }
    event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, TAG);
    event.post(CGEventTapLocation::Session);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{from_code, to_code};
    use core_graphics::event::KeyCode;
    use freddie_keys::Keyboard;

    #[test]
    fn round_trips_a_mapped_key() {
        assert_eq!(to_code(Keyboard::KeyR), Some(KeyCode::ANSI_R));
        assert_eq!(from_code(KeyCode::ANSI_R), Some(Keyboard::KeyR));
    }

    #[test]
    fn keys_without_a_mac_code_are_none() {
        assert_eq!(to_code(Keyboard::F24), None);
        assert_eq!(to_code(Keyboard::Insert), None);
    }
}
