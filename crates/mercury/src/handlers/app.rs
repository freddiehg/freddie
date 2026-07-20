//! In-app handlers: Chrome's refresh, address bar and copies, and Ghostty's tmux window
//! navigation.

use bind::Node;
use freddie_keys::{Key, ModifierFlags};
use laserbeam::Ascend;

use super::{and_go_home, to_typing};
use crate::MercuryEffect;
use crate::effect::{Copied, UrlPart, tap};
use crate::sources::host;
use crate::state::MercuryPath;

/// `r` in Chrome: cmd-r, a refresh. Touches neither event nor node, so both are generic.
pub(crate) fn refresh<E, N>(_ev: &E, _node: N) -> MercuryEffect {
    tap(Key::KeyR, ModifierFlags::COMMAND)
}

/// `l` in Chrome: cmd-l, focusing the address bar, and then typing.
///
/// A focused text field is somewhere you type, and the in-app layer would swallow what you typed
/// at it, so this leaves for typing the way nav's `space` does.
pub(crate) fn focus_address_bar<'a, E, P: Ascend<MercuryPath<'a>>, D>(
    ev: &E,
    node: Node<P, D>,
) -> Vec<MercuryEffect> {
    let mut effects = vec![tap(Key::KeyL, ModifierFlags::COMMAND)];
    effects.extend(to_typing(ev, node));
    effects
}

/// `shift-l` in Chrome: the front tab's whole URL, onto the clipboard.
pub(crate) fn copy_url<'a, E, P: Ascend<MercuryPath<'a>>, D>(
    _ev: &E,
    node: Node<P, D>,
) -> Vec<MercuryEffect> {
    copy(node.parent, UrlPart::Whole)
}

/// `cmd-l` in Chrome: the front tab's host, onto the clipboard.
pub(crate) fn copy_host<'a, E, P: Ascend<MercuryPath<'a>>, D>(
    _ev: &E,
    node: Node<P, D>,
) -> Vec<MercuryEffect> {
    copy(node.parent, UrlPart::Host)
}

/// Copy `part` of the front tab's URL.
///
/// The extension reports that URL as it changes, so the text is normally already here and the
/// effect carries it. Nothing typed at Chrome and nothing read back out of it: the copy does not
/// touch the address bar, so what you were part-way through typing there survives it.
///
/// Without a reported URL there is nothing to take a host from, and asking Chrome is the only way
/// to answer at all, so that case falls back to [`Copied::FrontTabUrl`]. A URL with no host
/// (`about:blank`, `file:///...`) has no answer either way, and copies nothing.
fn copy<'a, P: Ascend<MercuryPath<'a>>>(path: P, part: UrlPart) -> Vec<MercuryEffect> {
    let root: MercuryPath<'_> = path.ascend();
    let Some(url) = root
        .foreground
        .confirmed_chrome()
        .and_then(|chrome| chrome.url.as_deref())
    else {
        return vec![MercuryEffect::Copy(Copied::FrontTabUrl(part))];
    };
    let text = match part {
        UrlPart::Whole => Some(url),
        UrlPart::Host => host(url),
    };
    text.map(|text| MercuryEffect::Copy(Copied::Text(text.to_owned())))
        .into_iter()
        .collect()
}

/// A tmux command: the `ctrl-a` prefix, then the command key.
///
/// Two taps rather than one chord, because the prefix has to be let go before the command or
/// tmux sees `ctrl-p` rather than `p`. Which is now what the shape says, rather than something
/// the order of six raw events has to get right.
fn tmux(flags: ModifierFlags, command: Key) -> Vec<MercuryEffect> {
    vec![tap(Key::KeyA, ModifierFlags::CONTROL), tap(command, flags)]
}

/// `j` in Ghostty: tmux's previous window. Stays, because walking windows repeats.
pub(crate) fn previous_window<E, N>(_ev: &E, _node: N) -> Vec<MercuryEffect> {
    tmux(ModifierFlags::empty(), Key::KeyP)
}

/// `k` in Ghostty: tmux's next window.
pub(crate) fn next_window<E, N>(_ev: &E, _node: N) -> Vec<MercuryEffect> {
    tmux(ModifierFlags::empty(), Key::KeyN)
}

/// The digits in Ghostty: jump straight to a tmux window, then go home.
///
/// The window is chosen with the digit's *shifted* symbol, because that is what the tmux config
/// binds: `!` through `)` select windows 1 through 10, while the bare digits select window
/// *indices* and so cannot reach the tenth. `1` sends `ctrl-a !` and `0` sends `ctrl-a )`.
///
/// Jumping to a window is a choice rather than something you repeat, so it leaves the layer.
/// Generic over the event, the path, and the node's data, since it only reaches `node.parent`.
/// See [`and_go_home`].
macro_rules! select_window {
    ($($handler:ident => $digit:ident),* $(,)?) => {$(
        pub(crate) fn $handler<'a, E, P: Ascend<MercuryPath<'a>>, D>(
            _ev: &E,
            node: Node<P, D>,
        ) -> Vec<MercuryEffect> {
            and_go_home(node.parent, tmux(ModifierFlags::SHIFT, Key::$digit))
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

/// `n` on claude.ai: start a new chat, and then type.
///
/// `cmd-shift-o` is the site's own shortcut, so this is a remap and not an automation: nothing has
/// to reach into the page. The modifiers ride as flags on the one key event, which is what keeps a
/// modifier the user is really holding from being stranded.
///
/// A new chat lands in its prompt box, which is somewhere you type, so this leaves for typing the
/// way Chrome's `l` does.
pub(crate) fn new_chat<'a, E, P: Ascend<MercuryPath<'a>>, D>(
    ev: &E,
    node: Node<P, D>,
) -> Vec<MercuryEffect> {
    let mut effects = vec![tap(
        Key::KeyO,
        ModifierFlags::COMMAND | ModifierFlags::SHIFT,
    )];
    effects.extend(to_typing(ev, node));
    effects
}
