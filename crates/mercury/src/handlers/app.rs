//! In-app handlers: Chrome's refresh, and Ghostty's tmux window navigation.

use bind::Node;
use freddie_keys::{Key, KeyEvent};

use super::and_go_home;
use crate::effect::tap;
use crate::state::{ChromeAppNode, GhosttyAppNode};
use crate::MercuryEffect;

/// `r` in Chrome: cmd-r, a refresh.
pub(crate) fn refresh(_ev: &KeyEvent, _node: ChromeAppNode) -> Vec<MercuryEffect> {
    vec![tap(&[Key::MetaLeft], Key::KeyR)]
}

/// A tmux command: the `ctrl-a` prefix, then the command key.
///
/// Two taps rather than one chord, because the prefix has to be let go before the command or
/// tmux sees `ctrl-p` rather than `p`. Which is now what the shape says, rather than something
/// the order of six raw events has to get right.
fn tmux(modifiers: &[Key], command: Key) -> Vec<MercuryEffect> {
    vec![tap(&[Key::ControlLeft], Key::KeyA), tap(modifiers, command)]
}

/// `j` in Ghostty: tmux's previous window. Stays, because walking windows repeats.
pub(crate) fn previous_window(_ev: &KeyEvent, _node: GhosttyAppNode) -> Vec<MercuryEffect> {
    tmux(&[], Key::KeyP)
}

/// `k` in Ghostty: tmux's next window.
pub(crate) fn next_window(_ev: &KeyEvent, _node: GhosttyAppNode) -> Vec<MercuryEffect> {
    tmux(&[], Key::KeyN)
}

/// The digits in Ghostty: jump straight to a tmux window, then go home.
///
/// The window is chosen with the digit's *shifted* symbol, because that is what the tmux config
/// binds: `!` through `)` select windows 1 through 10, while the bare digits select window
/// *indices* and so cannot reach the tenth. `1` sends `ctrl-a !` and `0` sends `ctrl-a )`.
///
/// Jumping to a window is a choice rather than something you repeat, so it leaves the layer.
/// See [`and_go_home`].
macro_rules! select_window {
    ($($handler:ident => $digit:ident),* $(,)?) => {$(
        pub(crate) fn $handler(_ev: &KeyEvent, node: GhosttyAppNode) -> Vec<MercuryEffect> {
            and_go_home(node.parent, tmux(&[Key::ShiftLeft], Key::$digit))
        }
    )*};
}

select_window! {
    window_1 => Num1,
    window_2 => Num2,
    window_3 => Num3,
    window_4 => Num4,
    window_5 => Num5,
    window_6 => Num6,
    window_7 => Num7,
    window_8 => Num8,
    window_9 => Num9,
    window_0 => Num0,
}
