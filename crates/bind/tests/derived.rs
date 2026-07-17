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

use bind::{Bind, Node, accumulate, dispatch};
use common::{Demo, DemoEvent, KeyEvent, Keyboard, kb};
use laserbeam::PathMut;
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
#[bind(Keyboard("esc") => on_esc)]
pub struct Shell {
    pub log: String,
}

/// A derived level. Not in the tree; `app_data` builds it.
#[derive(Bind)]
#[derived_node(parent = ShellPath)]
#[binds(Demo)]
#[derived_child(tab_data)]
#[bind(Keyboard("r") => on_r)]
pub struct AppData {
    pub tab: String,
}

/// A derived level UNDER a derived level. Its parent is a `Node`, not a `PathMut`.
#[derive(Bind)]
#[derived_node(parent = AppNode)]
#[binds(Demo)]
#[bind(Keyboard("g") => on_g)]
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

/// Its own data, and the layer, through `parent`.
fn on_r(ev: &KeyEvent, mut node: AppNode) -> usize {
    let tab = node.data.tab.clone();
    node.parent.get_mut().log.push_str(&tab);
    ev.key.len()
}

/// Two levels down: its own data, the parent LEVEL's data, and the layer.
fn on_g(ev: &KeyEvent, mut node: TabNode) -> usize {
    let thread = node.data.thread;
    let tab = node.parent.data.tab.clone();
    let _ = write!(node.parent.parent.get_mut().log, "{tab}{thread}");
    ev.key.len()
}

fn on_esc(ev: &KeyEvent, mut node: Node<ShellPath, ()>) -> usize {
    node.parent.get_mut().log.push('e');
    ev.key.len()
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
    assert_eq!(dispatch::<Demo, Root>(&mut r, &key("r")), Some(1));
    assert_eq!(r.layer.log, "inbox"); // the LAYER's real state, written from the derived level
    assert_eq!(r.app.as_ref().unwrap().tab, "inbox"); // the tree is untouched by `data`
}

#[test]
fn a_derived_level_can_have_a_derived_child() {
    let mut r = root(Some("gmail"));
    assert_eq!(dispatch::<Demo, Root>(&mut r, &key("g")), Some(1));
    assert_eq!(r.layer.log, "gmail7"); // own data, the parent level's data, and the layer
}

#[test]
fn a_miss_hands_the_parent_back_at_every_level() {
    // The tab level misses `r`, so the app level's bind runs with its data intact.
    let mut r = root(Some("gmail"));
    assert_eq!(dispatch::<Demo, Root>(&mut r, &key("r")), Some(1));
    assert_eq!(r.layer.log, "gmail");

    // Both derived levels miss `esc`, so the LAYER's bind runs with its path intact.
    let mut r = root(Some("gmail"));
    assert_eq!(dispatch::<Demo, Root>(&mut r, &key("esc")), Some(3));
    assert_eq!(r.layer.log, "e");
}

#[test]
fn with_no_app_there_is_no_level_and_the_layer_still_works() {
    let mut r = root(None);
    assert_eq!(dispatch::<Demo, Root>(&mut r, &key("r")), None);
    assert_eq!(dispatch::<Demo, Root>(&mut r, &key("esc")), Some(3));
    assert_eq!(r.layer.log, "e");
}

#[test]
fn the_check_sees_a_derived_levels_binds() {
    // Why accumulate had to take a path: with &self it cannot call a derived child fn, so
    // these triggers would be invisible to the trigger set.
    let mut r = root(Some("gmail"));
    let set: HashSet<_> = accumulate::<Demo, Root>(&mut r).unwrap();
    assert_eq!(set, HashSet::from([kb("esc"), kb("r"), kb("g")]));

    // The tab level only exists on gmail, so its trigger is not claimed elsewhere.
    let mut r = root(Some("inbox"));
    let set: HashSet<_> = accumulate::<Demo, Root>(&mut r).unwrap();
    assert_eq!(set, HashSet::from([kb("esc"), kb("r")]));

    // And with no app at all, only the layer's.
    let mut r = root(None);
    let set: HashSet<_> = accumulate::<Demo, Root>(&mut r).unwrap();
    assert_eq!(set, HashSet::from([kb("esc")]));
}
