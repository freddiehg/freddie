//! The `KeySequence` state machine, over `[j, k]` unless a case needs another shape.
//!
//! Every case is a stream of key events fed to one sequence, asserting the outcome of each. The
//! machine is a pure function of what it has swallowed and the event, so the table is checkable.

use freddie_keys::{Key, KeyEvent, KeySequence, KeySequenceOutcome, ModifierFlags, PressType};

const JK: &[Key] = &[Key::KeyJ, Key::KeyK];

const fn jk() -> KeySequence {
    KeySequence::new(JK)
}

const fn down(key: Key) -> KeyEvent {
    KeyEvent {
        key,
        press: PressType::Down,
        flags: ModifierFlags::empty(),
    }
}

const fn up(key: Key) -> KeyEvent {
    KeyEvent {
        key,
        press: PressType::Up,
        flags: ModifierFlags::empty(),
    }
}

const fn with_flags(mut ev: KeyEvent, flags: ModifierFlags) -> KeyEvent {
    ev.flags = flags;
    ev
}

/// The presses a broken run replays, or `None` when the outcome was not a break.
fn replayed(outcome: KeySequenceOutcome) -> Option<Vec<KeyPressed>> {
    match outcome {
        KeySequenceOutcome::Passed(presses) => Some(
            presses
                .into_iter()
                .map(|p| KeyPressed(p.key, p.press))
                .collect(),
        ),
        _ => None,
    }
}

/// A `KeyPress` that prints readably in a failed assertion.
#[derive(PartialEq, Eq, Debug)]
struct KeyPressed(Key, PressType);

const fn d(key: Key) -> KeyPressed {
    KeyPressed(key, PressType::Down)
}

const fn u(key: Key) -> KeyPressed {
    KeyPressed(key, PressType::Up)
}

#[test]
fn deliberate_completes() {
    let mut s = jk();
    assert_eq!(s.advance(&down(Key::KeyJ)), KeySequenceOutcome::Advanced);
    assert!(!s.is_idle());
    assert_eq!(s.advance(&up(Key::KeyJ)), KeySequenceOutcome::Advanced);
    assert_eq!(s.advance(&down(Key::KeyK)), KeySequenceOutcome::Completed);
    assert!(s.is_idle(), "completing drops what it swallowed");
}

#[test]
fn rolled_completes() {
    // k goes down before j comes up, which is what typing it at speed produces.
    let mut s = jk();
    assert_eq!(s.advance(&down(Key::KeyJ)), KeySequenceOutcome::Advanced);
    assert_eq!(s.advance(&down(Key::KeyK)), KeySequenceOutcome::Completed);
    assert!(s.is_idle());
}

#[test]
fn an_idle_run_passes_everything_with_an_empty_replay() {
    let mut s = jk();
    for ev in [down(Key::KeyA), up(Key::KeyA), up(Key::KeyJ), up(Key::KeyK)] {
        assert_eq!(replayed(s.advance(&ev)), Some(vec![]), "{ev:?}");
        assert!(s.is_idle());
    }
}

#[test]
fn breaking_after_a_held_j_replays_its_down_only() {
    let mut s = jk();
    let _ = s.advance(&down(Key::KeyJ));
    assert_eq!(
        replayed(s.advance(&down(Key::KeyA))),
        Some(vec![d(Key::KeyJ)]),
    );
    assert!(s.is_idle());
}

#[test]
fn breaking_after_a_full_j_tap_replays_both_halves() {
    let mut s = jk();
    let _ = s.advance(&down(Key::KeyJ));
    let _ = s.advance(&up(Key::KeyJ));
    assert_eq!(
        replayed(s.advance(&down(Key::KeyA))),
        Some(vec![d(Key::KeyJ), u(Key::KeyJ)]),
    );
}

#[test]
fn a_modifier_flag_breaks_the_run() {
    let mut s = jk();
    let _ = s.advance(&down(Key::KeyJ));
    // The k that would have completed it does not, because it arrived under cmd.
    assert_eq!(
        replayed(s.advance(&with_flags(down(Key::KeyK), ModifierFlags::COMMAND))),
        Some(vec![d(Key::KeyJ)]),
    );
    assert!(s.is_idle());
}

#[test]
fn a_modifier_flag_stops_the_run_opening() {
    let mut s = jk();
    assert_eq!(
        replayed(s.advance(&with_flags(down(Key::KeyJ), ModifierFlags::COMMAND))),
        Some(vec![]),
    );
    assert!(s.is_idle());
}

#[test]
fn fn_alone_stops_the_run_opening() {
    // `fn` never arrives as a key, only as a flag on another, and it counts like any other.
    let mut s = jk();
    assert_eq!(
        replayed(s.advance(&with_flags(down(Key::KeyJ), ModifierFlags::FN))),
        Some(vec![]),
    );
}

#[test]
fn an_auto_repeat_breaks_the_run() {
    // A held key repeats its down with no up between, so the second down is a down of a key that
    // is already down. It breaks, and the swallowed down replays ahead of it.
    let mut s = jk();
    let _ = s.advance(&down(Key::KeyJ));
    assert_eq!(
        replayed(s.advance(&down(Key::KeyJ))),
        Some(vec![d(Key::KeyJ)]),
    );
    assert!(s.is_idle());
}

#[test]
fn an_up_for_a_key_the_run_never_took_breaks_it() {
    // `a` was held before the run started, so its up is not the run's to swallow: the app saw the
    // down and is owed the up.
    let mut s = jk();
    let _ = s.advance(&down(Key::KeyJ));
    assert_eq!(
        replayed(s.advance(&up(Key::KeyA))),
        Some(vec![d(Key::KeyJ)]),
    );
}

#[test]
fn a_second_up_for_a_key_already_released_breaks_it() {
    let mut s = jk();
    let _ = s.advance(&down(Key::KeyJ));
    let _ = s.advance(&up(Key::KeyJ));
    assert_eq!(
        replayed(s.advance(&up(Key::KeyJ))),
        Some(vec![d(Key::KeyJ), u(Key::KeyJ)]),
    );
}

#[test]
fn the_breaking_key_does_not_open_a_new_run() {
    // j, j: the second j breaks the first run rather than starting a second, so a k after it is an
    // ordinary k.
    let mut s = jk();
    let _ = s.advance(&down(Key::KeyJ));
    let _ = s.advance(&up(Key::KeyJ));
    let _ = s.advance(&down(Key::KeyJ));
    assert!(s.is_idle());
    assert_eq!(replayed(s.advance(&up(Key::KeyJ))), Some(vec![]));
    assert_eq!(replayed(s.advance(&down(Key::KeyK))), Some(vec![]));
}

#[test]
fn interrupt_hands_back_what_was_swallowed() {
    let mut s = jk();
    let _ = s.advance(&down(Key::KeyJ));
    let _ = s.advance(&up(Key::KeyJ));
    let presses: Vec<KeyPressed> = s
        .interrupt()
        .into_iter()
        .map(|p| KeyPressed(p.key, p.press))
        .collect();
    assert_eq!(presses, vec![d(Key::KeyJ), u(Key::KeyJ)]);
    assert!(s.is_idle());
    assert!(
        s.interrupt().is_empty(),
        "an idle run has nothing to hand back"
    );
}

#[test]
fn a_repeated_key_sequence_fires_on_a_double_tap_but_not_on_a_hold() {
    const JJ: &[Key] = &[Key::KeyJ, Key::KeyJ];

    let mut s = KeySequence::new(JJ);
    assert_eq!(s.advance(&down(Key::KeyJ)), KeySequenceOutcome::Advanced);
    assert_eq!(s.advance(&up(Key::KeyJ)), KeySequenceOutcome::Advanced);
    assert_eq!(s.advance(&down(Key::KeyJ)), KeySequenceOutcome::Completed);

    // Held, the repeat arrives with j still down and breaks it instead.
    let mut s = KeySequence::new(JJ);
    let _ = s.advance(&down(Key::KeyJ));
    assert_eq!(
        replayed(s.advance(&down(Key::KeyJ))),
        Some(vec![d(Key::KeyJ)]),
    );
}

#[test]
fn a_longer_run_replays_interleaved_ups_in_arrival_order() {
    // Three keys can be down at once, so the ups interleave with the downs and only the order they
    // arrived in reproduces the stream.
    const JKL: &[Key] = &[Key::KeyJ, Key::KeyK, Key::KeyL];

    let mut s = KeySequence::new(JKL);
    assert_eq!(s.advance(&down(Key::KeyJ)), KeySequenceOutcome::Advanced);
    assert_eq!(s.advance(&down(Key::KeyK)), KeySequenceOutcome::Advanced);
    assert_eq!(s.advance(&up(Key::KeyJ)), KeySequenceOutcome::Advanced);
    assert_eq!(
        replayed(s.advance(&down(Key::KeyA))),
        Some(vec![d(Key::KeyJ), d(Key::KeyK), u(Key::KeyJ)]),
    );
}

#[test]
fn a_longer_run_completes_rolled() {
    const JKL: &[Key] = &[Key::KeyJ, Key::KeyK, Key::KeyL];

    let mut s = KeySequence::new(JKL);
    let _ = s.advance(&down(Key::KeyJ));
    let _ = s.advance(&down(Key::KeyK));
    assert_eq!(s.advance(&down(Key::KeyL)), KeySequenceOutcome::Completed);
    assert!(s.is_idle());
}

#[test]
#[should_panic(expected = "a sequence needs at least one key")]
fn an_empty_sequence_is_rejected() {
    let _ = KeySequence::new(&[]);
}
