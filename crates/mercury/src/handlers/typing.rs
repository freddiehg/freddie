//! Typing's catch-all: the `jk` run, and every key it does not take passed through to the app.

use bind::AscendState;
use freddie::{KeySequenceOutcome, TimerFired};
use freddie_keys::KeyEvent;
use laserbeam::{Complete, Completed, MaybeInvalidated};

use crate::MercuryEffect;
use crate::effect::{emit, replay};
use crate::state::{HomeLayer, MercuryPath, TypingLayerPath, arm_jk_timeout};

/// Any key in typing, which binds nothing else: it goes to the `jk` run first, which either takes
/// it, hands back what it had swallowed for a key that broke it, or completes and leaves for
/// home. A key the run does not want is passed through carrying exactly the flags it arrived
/// with, so a baked-on modifier (an injected `cmd`-`v`, or `fn`) rides along.
///
/// Three outcomes of the one policy, so one handler.
pub(crate) fn pass_through<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, TypingLayerPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<TypingLayerPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut p) => {
            // The run is idle before this key iff this key opens it, which is when its window is
            // armed. Every other outcome ends the run, which drops the guard and cancels the
            // wait.
            let opening = p.get().jk.is_idle();
            match p.get_mut().jk.advance(ev) {
                KeySequenceOutcome::Advanced if opening => match p.get().jk.window() {
                    Some(window) => {
                        let (guard, timer) = arm_jk_timeout(window);
                        p.get_mut().jk.hold(guard);
                        (vec![timer], p.complete())
                    }
                    None => (vec![], p.complete()),
                },
                KeySequenceOutcome::Advanced => (vec![], p.complete()),
                KeySequenceOutcome::Passed(presses) => {
                    let mut out = replay(presses);
                    out.push(emit(ev.key, ev.press, ev.flags));
                    (out, p.complete())
                }
                KeySequenceOutcome::Completed => {
                    let root: MercuryPath<'x> = p.into_ancestor();
                    (root.set_layer(HomeLayer::new()), root.complete())
                }
            }
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}

/// The window elapsed with no next key: what the run swallowed types itself, exactly as a key
/// that broke the run would have made it.
pub(crate) fn jk_timeout<'x>(
    _ev: &TimerFired,
    _snap: (),
    st: AscendState<'_, TypingLayerPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<TypingLayerPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut p) => {
            let presses = p.get_mut().jk.interrupt();
            (replay(presses), p.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}
