//! A level that is not in the tree.
//!
//! `#[derived_child(f)]` on a node whose child is no field; `#[derived_node(parent = ..)]` on
//! the struct that child fn returns. `f` is `fn(&Parent) -> Option<Data>`: a shared reference,
//! so it cannot mutate, and it never holds the parent, so it cannot lose it.
//!
//! The tree here has TWO derived levels, one under the other, to pin that a derived level can
//! itself have a derived child and that a miss hands the parent back at every level.

mod common;

use std::fmt::Write as _;

use bind::{AscendState, Bind, Node, accumulate, dispatch, exclusive};
use common::{Demo, DemoEvent, KeyEvent, Keyboard, kb};
use laserbeam::{Complete, Completed, MaybeInvalidated, PathMut};
use std::collections::HashSet;

#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
pub struct Root {
    /// The only copy. The layer stores no app.
    pub app: Option<Chrome>,
    #[resolve_into]
    pub layer: Shell,
}

pub struct Chrome {
    pub tab: String,
}

#[derive(Bind)]
#[node(parent = RootPath)]
#[binds(Demo)]
#[derived_child(app_data)]
#[post(Keyboard("q") => log_leave)]
#[bind(Keyboard("esc") => on_esc)]
pub struct Shell {
    pub log: String,
}

/// A derived level. Not in the tree; `app_data` builds it.
#[derive(Bind)]
#[derived_node(parent = ShellPath)]
#[binds(Demo)]
#[derived_child(tab_data)]
#[pre_post(Keyboard("r") => (snap_tab, exclusive(on_r)))]
#[bind(Keyboard("q") => app_home)]
pub struct AppData {
    pub tab: String,
}

/// A derived level UNDER a derived level. Its parent is a `Node`, not a `PathMut`.
#[derive(Bind)]
#[derived_node(parent = AppNode)]
#[binds(Demo)]
#[pre_post(Keyboard("g") => (snap_tab_thread, exclusive(on_g)))]
pub struct TabData {
    pub thread: u32,
}

pub type RootPath<'a> = &'a mut Root;
pub type ShellPath<'a> = PathMut<Shell, RootPath<'a>>;
pub type AppNode<'a> = Node<ShellPath<'a>, AppData>;
pub type TabNode<'a> = Node<AppNode<'a>, TabData>;

pub enum R<'a> {
    Shell(ShellPath<'a>),
}

/// `#[derived_child]`. It reads root state that is not on its path, and returns only the DATA.
fn app_data(path: &ShellPath) -> Option<AppData> {
    let chrome = path.parent().app.as_ref()?;
    Some(AppData {
        tab: chrome.tab.clone(),
    })
}

/// A derived child fn on a DERIVED level. Same shape; `&Parent` is a `&Node`.
fn tab_data(node: &AppNode) -> Option<TabData> {
    (node.data.tab == "gmail").then_some(TabData { thread: 7 })
}

/// The pre takes the level's own data while the node is whole, since the descent consumes it
/// and the ascent holds only the place beneath.
fn snap_tab(_ev: &KeyEvent, node: &AppNode) -> String {
    node.data.tab.clone()
}

/// What the pre took, written into the layer, which is where the ascent stands.
///
/// The snap arrives by value because its type is whatever the pre returned, and this one had to
/// clone: the node it read is consumed by the descent before the ascent runs.
#[expect(clippy::needless_pass_by_value)]
fn on_r<'x>(
    ev: &KeyEvent,
    tab: String,
    st: AscendState<'_, ShellPath<'x>>,
) -> (Vec<usize>, Completed<ShellPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut shell) => {
            shell.get_mut().log.push_str(&tab);
            (vec![ev.key.len()], shell.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}

/// Two levels down: its own data and the parent LEVEL's, both taken before the descent.
fn snap_tab_thread(_ev: &KeyEvent, node: &TabNode) -> (String, u32) {
    (node.parent.data.tab.clone(), node.data.thread)
}

fn on_g<'x>(
    ev: &KeyEvent,
    (tab, thread): (String, u32),
    st: AscendState<'_, ShellPath<'x>>,
) -> (Vec<usize>, Completed<ShellPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut shell) => {
            let _ = write!(shell.get_mut().log, "{tab}{thread}");
            (vec![ev.key.len()], shell.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}

fn on_esc<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ShellPath<'x>>,
) -> (Vec<usize>, Completed<ShellPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut shell) => {
            shell.get_mut().log.push('e');
            (vec![3], shell.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![3], c),
    }
}

/// A leave FROM a derived level: it ascends at Shell, so it leaves by walking off Shell's path.
fn app_home<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ShellPath<'x>>,
) -> (Vec<usize>, Completed<ShellPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(shell) => (vec![9], shell.into_parent().complete()),
        MaybeInvalidated::Invalidated(c) => (vec![9], c),
    }
}

/// Shell's post, scheduled whatever claimed: it sees what the derived level below did, so a leave
/// from there reaches it as `Invalidated` and it reports that rather than touching the path.
fn log_leave<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ShellPath<'x>>,
) -> (Vec<usize>, Completed<ShellPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut shell) => {
            shell.get_mut().log.push('s');
            (vec![], shell.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![7], c),
    }
}

const fn key(k: &'static str) -> DemoEvent {
    DemoEvent::Keyboard(KeyEvent { key: k })
}

fn root(tab: Option<&str>) -> Root {
    Root {
        app: tab.map(|t| Chrome { tab: t.to_owned() }),
        layer: Shell { log: String::new() },
    }
}

#[test]
fn a_derived_level_binds_its_own_keys_and_reaches_the_tree_through_parent() {
    let mut r = root(Some("inbox"));
    assert_eq!(dispatch::<Demo, Root, _>(&mut r, &key("r")), vec![1]);
    assert_eq!(r.layer.log, "inbox"); // the LAYER's real state, written from the derived level
    assert_eq!(r.app.as_ref().unwrap().tab, "inbox"); // the tree is untouched by `data`
}

#[test]
fn a_derived_level_can_have_a_derived_child() {
    let mut r = root(Some("gmail"));
    assert_eq!(dispatch::<Demo, Root, _>(&mut r, &key("g")), vec![1]);
    assert_eq!(r.layer.log, "gmail7"); // own data, the parent level's data, and the layer
}

#[test]
fn a_miss_hands_the_parent_back_at_every_level() {
    // The tab level misses `r`, so the app level's bind runs with its data intact.
    let mut r = root(Some("gmail"));
    assert_eq!(dispatch::<Demo, Root, _>(&mut r, &key("r")), vec![1]);
    assert_eq!(r.layer.log, "gmail");

    // Both derived levels miss `esc`, so the LAYER's bind runs with its path intact.
    let mut r = root(Some("gmail"));
    assert_eq!(dispatch::<Demo, Root, _>(&mut r, &key("esc")), vec![3]);
    assert_eq!(r.layer.log, "e");
}

#[test]
fn with_no_app_there_is_no_level_and_the_layer_still_works() {
    let mut r = root(None);
    assert_eq!(dispatch::<Demo, Root, _>(&mut r, &key("r")), vec![]);
    assert_eq!(dispatch::<Demo, Root, _>(&mut r, &key("esc")), vec![3]);
    assert_eq!(r.layer.log, "e");
}

#[test]
fn the_check_sees_a_derived_levels_binds() {
    // Why accumulate had to take a path: with &self it cannot call a derived child fn, so
    // the app level's `q` would be invisible to the trigger set.
    //
    // `r` and `g` are not in it. They are `#[pre_post]`s, whose rhs claims by naming
    // `exclusive` itself, and the macro looks inside no rhs: what a node CLAIMS is what it
    // writes as a `#[bind]`. Shell's `q` post is absent for the same reason, which is what
    // lets it share a trigger with the app level's bind.
    let mut r = root(Some("gmail"));
    let set: HashSet<_> = accumulate::<Demo, Root>(&mut r).unwrap();
    assert_eq!(set, HashSet::from([kb("esc"), kb("q")]));

    let mut r = root(Some("inbox"));
    let set: HashSet<_> = accumulate::<Demo, Root>(&mut r).unwrap();
    assert_eq!(set, HashSet::from([kb("esc"), kb("q")]));

    // And with no app at all, only the layer's.
    let mut r = root(None);
    let set: HashSet<_> = accumulate::<Demo, Root>(&mut r).unwrap();
    assert_eq!(set, HashSet::from([kb("esc")]));
}

/// The derived-leave walk: `q` fires at the app level, which ascends at Shell and leaves from
/// there; Shell's post is scheduled by the same key and sees what that leave did.
#[test]
fn a_leave_from_a_derived_level_reaches_the_place_as_invalidated() {
    let mut r = root(Some("gmail"));
    // 9 from the leave, then 7 from the post's `Invalidated` arm: the post ran after it, on the
    // state it left behind, and did not touch the path.
    assert_eq!(dispatch::<Demo, Root, _>(&mut r, &key("q")), vec![9, 7]);
    assert_eq!(r.layer.log, "", "the post's staying arm never ran");
}

/// With no level below it, nothing leaves, so the same post takes its other arm.
#[test]
fn the_post_marks_the_layer_when_nothing_left() {
    let mut r = root(None);
    assert_eq!(dispatch::<Demo, Root, _>(&mut r, &key("q")), vec![]);
    assert_eq!(r.layer.log, "s");
}

// ---- a derived level whose data is an enum ----

/// Two variants bind the same key, so which handler ran says which level was live. Their
/// triggers do not collide: only one variant exists per dispatch.
#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
pub struct Modes {
    pub mode: Option<bool>,
    #[resolve_into]
    pub shell: ModeShell,
}

#[derive(Bind)]
#[node(parent = ModesPath)]
#[binds(Demo)]
#[derived_child(mode_data)]
pub struct ModeShell {
    pub log: String,
}

#[derive(Bind)]
#[derived_node(parent = ModeShellPath)]
#[binds(Demo)]
pub enum ModeData {
    On(OnMode),
    Off(OffMode),
}

#[derive(Bind)]
#[derived_node(parent = ModeShellPath)]
#[binds(Demo)]
#[bind(Keyboard("m") => on_mode_on)]
pub struct OnMode;

#[derive(Bind)]
#[derived_node(parent = ModeShellPath)]
#[binds(Demo)]
#[bind(Keyboard("m") => on_mode_off)]
pub struct OffMode;

pub type ModesPath<'a> = &'a mut Modes;
pub type ModeShellPath<'a> = PathMut<ModeShell, ModesPath<'a>>;

fn mode_data(path: &ModeShellPath) -> Option<ModeData> {
    let on = path.parent().mode?;
    Some(if on {
        ModeData::On(OnMode)
    } else {
        ModeData::Off(OffMode)
    })
}

fn on_mode_on<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ModeShellPath<'x>>,
) -> (Vec<usize>, Completed<ModeShellPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut shell) => {
            shell.get_mut().log.push_str("on");
            (vec![ev.key.len()], shell.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}

fn on_mode_off<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ModeShellPath<'x>>,
) -> (Vec<usize>, Completed<ModeShellPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut shell) => {
            shell.get_mut().log.push_str("off");
            (vec![ev.key.len()], shell.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}

const fn modes(mode: Option<bool>) -> Modes {
    Modes {
        mode,
        shell: ModeShell { log: String::new() },
    }
}

#[test]
fn the_live_variant_of_a_derived_enum_handles_the_key() {
    let mut m = modes(Some(true));
    assert_eq!(dispatch::<Demo, Modes, _>(&mut m, &key("m")), vec![1]);
    assert_eq!(m.shell.log, "on");

    let mut m = modes(Some(false));
    assert_eq!(dispatch::<Demo, Modes, _>(&mut m, &key("m")), vec![1]);
    assert_eq!(m.shell.log, "off");
}

#[test]
fn no_mode_is_no_level_at_all() {
    let mut m = modes(None);
    assert_eq!(dispatch::<Demo, Modes, _>(&mut m, &key("m")), vec![]);
    assert_eq!(m.shell.log, "");
}

/// A derived level's trigger is claimed only while that level is live, which is what the check
/// walking the tree by path buys.
#[test]
fn the_check_sees_only_the_live_variants_trigger() {
    let mut m = modes(Some(true));
    let set: HashSet<_> = accumulate::<Demo, Modes>(&mut m).unwrap();
    assert_eq!(set, HashSet::from([kb("m")]));

    let mut m = modes(None);
    let set: HashSet<_> = accumulate::<Demo, Modes>(&mut m).unwrap();
    assert_eq!(set, HashSet::new());
}
