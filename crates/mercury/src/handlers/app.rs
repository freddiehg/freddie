//! In-app handlers: Chrome's refresh, address bar and copies, and Ghostty's tmux window
//! navigation.

use bind::AscendState;
use freddie_keys::{Key, ModifierFlags};
use laserbeam::{Complete, Completed, HasAncestor, HasStop, IntoAncestor, MaybeInvalidated};

use super::and_go_home_from;
use crate::MercuryEffect;
use crate::effect::{Copied, UrlPart, tap};
use crate::sources::host;
use crate::state::{Mercury, MercuryPath, TypingLayer};

/// `r` in Chrome: cmd-r, a refresh.
///
/// A pure effect: it reads no state and changes none, so it stays where it stands and its two
/// arms differ only in whether there is still a path to stand on.
pub(crate) fn refresh<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (vec![tap(Key::KeyR, ModifierFlags::COMMAND)], st.complete())
}

/// `l` in Chrome: cmd-l, focusing the address bar, and then typing.
///
/// A focused text field is somewhere you type, and the in-app layer would swallow what you typed
/// at it, so this leaves for typing the way nav's `space` does.
pub(crate) fn focus_address_bar<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let mut effects = vec![tap(Key::KeyL, ModifierFlags::COMMAND)];
    effects.extend(root.set_layer(TypingLayer::new()));
    (effects, root.complete())
}

/// `shift-l` in Chrome: the front tab's whole URL, onto the clipboard.
///
/// A stayer that reads the tree: it reaches the root by shared reference, so the path it was
/// handed is still there to complete at.
pub(crate) fn copy_url<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasAncestor<MercuryPath<'a>> + HasStop + Complete<P>,
{
    match st.state {
        MaybeInvalidated::NotInvalidated(path) => {
            let effects = copy(path.ancestor(), UrlPart::Whole);
            (effects, path.complete())
        }
        MaybeInvalidated::Invalidated(c) => (Vec::new(), c),
    }
}

/// `cmd-l` in Chrome: the front tab's host, onto the clipboard.
pub(crate) fn copy_host<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasAncestor<MercuryPath<'a>> + HasStop + Complete<P>,
{
    match st.state {
        MaybeInvalidated::NotInvalidated(path) => {
            let effects = copy(path.ancestor(), UrlPart::Host);
            (effects, path.complete())
        }
        MaybeInvalidated::Invalidated(c) => (Vec::new(), c),
    }
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
fn copy(root: &Mercury, part: UrlPart) -> Vec<MercuryEffect> {
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
pub(crate) fn previous_window<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (tmux(ModifierFlags::empty(), Key::KeyP), st.complete())
}

/// `k` in Ghostty: tmux's next window.
pub(crate) fn next_window<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (tmux(ModifierFlags::empty(), Key::KeyN), st.complete())
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
        pub(crate) fn $handler<'a, E, P>(
            _ev: &E,
            _snap: (),
            st: AscendState<'_, P>,
        ) -> (Vec<MercuryEffect>, Completed<P>)
        where
            P: HasStop,
            MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
            MercuryPath<'a>: Complete<P>,
        {
            let root: MercuryPath<'a> = st.state.into_ancestor();
            let effects = and_go_home_from(root, tmux(ModifierFlags::SHIFT, Key::$digit));
            (effects, root.complete())
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
pub(crate) fn new_chat<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let mut effects = vec![tap(
        Key::KeyO,
        ModifierFlags::COMMAND | ModifierFlags::SHIFT,
    )];
    effects.extend(root.set_layer(TypingLayer::new()));
    (effects, root.complete())
}
