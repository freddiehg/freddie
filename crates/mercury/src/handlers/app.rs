//! In-app units: Chrome's refresh, address bar and copies, and Ghostty's tmux window navigation.
//!
//! The taps here emit and nothing else. What follows a tap is the bind site's business: Chrome's
//! `l` is `and!(tap_cmd_l, enter_typing)`, because a focused address bar is somewhere you type
//! and the in-app layer would swallow it; Chrome's `r` is `tap_cmd_r` alone, because refreshing
//! repeats and the layer stays.

use bind::AscendState;
use freddie_keys::{Key, ModifierFlags};
use laserbeam::{Complete, Completed, HasAncestor, HasStop, MaybeInvalidated};

use crate::MercuryEffect;
use crate::effect::{Copied, UrlPart, tap};
use crate::sources::host;
use crate::state::{Mercury, MercuryPath};

/// Chrome's refresh: cmd-r.
pub(crate) fn tap_cmd_r<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (vec![tap(Key::KeyR, ModifierFlags::COMMAND)], st.complete())
}

/// Chrome's address bar: cmd-l.
pub(crate) fn tap_cmd_l<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (vec![tap(Key::KeyL, ModifierFlags::COMMAND)], st.complete())
}

/// claude.ai's new chat: cmd-shift-o, the site's own shortcut.
///
/// A remap rather than an automation: nothing reaches into the page. The modifiers ride as flags
/// on the one key event, which is what keeps a modifier the user is really holding from being
/// stranded.
pub(crate) fn tap_cmd_shift_o<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (
        vec![tap(
            Key::KeyO,
            ModifierFlags::COMMAND | ModifierFlags::SHIFT,
        )],
        st.complete(),
    )
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

/// `j` in Ghostty: tmux's previous window. Bound alone, because walking windows repeats.
pub(crate) fn tmux_prev<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (tmux(ModifierFlags::empty(), Key::KeyP), st.complete())
}

/// `k` in Ghostty: tmux's next window.
pub(crate) fn tmux_next<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (tmux(ModifierFlags::empty(), Key::KeyN), st.complete())
}

/// Jump to a tmux window by its digit.
///
/// The window is chosen with the digit's SHIFTED symbol, because that is what the tmux config
/// binds: `!` through `)` select windows 1 through 10, while the bare digits select window
/// *indices* and so cannot reach the tenth. `1` sends `ctrl-a !` and `0` sends `ctrl-a )`.
///
/// Parameterized, so one unit serves all ten digits: `and!(tmux_window(Key::Num1), go_home)`.
/// Jumping is a choice rather than something you repeat, which is why `go_home` composes after
/// it while `tmux_prev` and `tmux_next` are bound alone.
pub(crate) fn tmux_window<E, P: HasStop + Complete<P>>(
    digit: Key,
) -> impl Fn(&E, (), AscendState<'_, P>) -> (Vec<MercuryEffect>, Completed<P>) {
    move |_ev, (), st| (tmux(ModifierFlags::SHIFT, digit), st.complete())
}
