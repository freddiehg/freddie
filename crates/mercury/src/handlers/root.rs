//! The root's two `AnyKey` items: tracking the held modifiers, and what an unbound key does.
//!
//! They are split because they answer to different things. Tracking is a concern: the sweeps need
//! `held` to be true for every key, including the ones a deeper layer claimed, so it is a post and
//! takes no claim. Passthrough is the policy for a key nothing else wanted, so it is the bind, and
//! it is the last one on the root's schedule.

use bind::AscendState;
use freddie::KeySequenceOutcome;
use freddie_keys::KeyEvent;
use laserbeam::{Complete, Completed};

use freddie::TimerFired;

use crate::MercuryEffect;
use crate::effect::{emit, replay};
use crate::state::{HomeLayer, Mercury, MercuryPath, arm_jk_timeout};

/// Every key, claimed or not: keep `held` true.
///
/// `held` feeds the open and close sweeps a layer change runs, so it has to see a modifier
/// pressed in a command layer, where a deeper binding claimed the key and the passthrough bind
/// below never runs. That is what makes this a post: it is scheduled by the trigger alone.
///
/// The flags on the event stay authoritative for what a key carries; `held` is for the sweeps.
pub(crate) fn track_held_modifiers<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    if ev.key.is_modifier() {
        root.typing_state.held.apply(ev);
    }
    (vec![], root.complete())
}

/// What a key nothing else bound does in this layer.
///
/// Outside a passthrough layer it is swallowed and that is all. In a passthrough layer it goes to
/// the `jk` run first, which either takes it, hands back what it had swallowed for a key that
/// broke it, or completes and leaves for home. A key the run does not want is passed through
/// carrying exactly the flags it arrived with, so a baked-on modifier (an injected `cmd`-`v`, or
/// `fn`) rides along.
///
/// Three outcomes of the one policy, so one handler.
pub(crate) fn pass_or_swallow<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    (pass_through(ev, root), root.complete())
}

/// The key's own effects, with the root in hand.
fn pass_through(ev: &KeyEvent, root: &mut Mercury) -> Vec<MercuryEffect> {
    if !root.layer().is_passthrough() {
        return Vec::new();
    }
    // The run is idle before this key iff this key opens it, which is when its window is armed.
    // Every other outcome ends the run, which drops the guard and cancels the wait.
    let opening = root.typing_state.jk.is_idle();
    match root.typing_state.jk.advance(ev) {
        KeySequenceOutcome::Advanced if opening => match root.typing_state.jk.window() {
            Some(window) => {
                let (guard, timer) = arm_jk_timeout(window);
                root.typing_state.jk.hold(guard);
                vec![timer]
            }
            None => Vec::new(),
        },
        KeySequenceOutcome::Advanced => Vec::new(),
        KeySequenceOutcome::Passed(presses) => {
            let mut out = replay(presses);
            out.push(emit(ev.key, ev.press, ev.flags));
            out
        }
        KeySequenceOutcome::Completed => root.set_layer(HomeLayer::new()),
    }
}

/// The window elapsed with no next key: what the run swallowed types itself, exactly as a key that
/// broke the run would have made it.
pub(crate) fn jk_timeout<'x>(
    _ev: &TimerFired,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    (replay(root.typing_state.jk.interrupt()), root.complete())
}
