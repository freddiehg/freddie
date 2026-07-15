//! A full laserbeam + bind tree shared by the accumulate and dispatch tests.
//!
//! Every node derives `Bind` (the path type the generated `Dispatch`
//! needs) and `Bind`. Handlers mutate their node's `hits` where it has one and
//! return the fired key's length, so a dispatch test can see which handler ran.
#![allow(dead_code)]

use bind::{Bind, Bindings, EventTrigger, Node};
use laserbeam::PathMut;

// Two sources: a keyboard and the foregrounded app.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Keyboard(pub &'static str);
pub struct KeyEvent {
    pub key: &'static str,
}
impl EventTrigger for Keyboard {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool {
        self.0 == ev.key
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Foreground(pub &'static str);
pub struct FgEvent {
    pub app: &'static str,
}
impl EventTrigger for Foreground {
    type Event = FgEvent;
    fn is_matching(&self, ev: &FgEvent) -> bool {
        self.0 == ev.app
    }
}

// The unified trigger (accumulate) and event (dispatch).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MercuryTrigger {
    Keyboard(Keyboard),
    Foreground(Foreground),
}
impl From<Keyboard> for MercuryTrigger {
    fn from(k: Keyboard) -> Self {
        Self::Keyboard(k)
    }
}
impl From<Foreground> for MercuryTrigger {
    fn from(f: Foreground) -> Self {
        Self::Foreground(f)
    }
}

pub enum MercuryEvent {
    Keyboard(KeyEvent),
    Foreground(FgEvent),
}
impl<'a> TryFrom<&'a MercuryEvent> for &'a KeyEvent {
    type Error = ();
    fn try_from(e: &'a MercuryEvent) -> Result<Self, ()> {
        match e {
            MercuryEvent::Keyboard(k) => Ok(k),
            MercuryEvent::Foreground(_) => Err(()),
        }
    }
}
impl<'a> TryFrom<&'a MercuryEvent> for &'a FgEvent {
    type Error = ();
    fn try_from(e: &'a MercuryEvent) -> Result<Self, ()> {
        match e {
            MercuryEvent::Foreground(f) => Ok(f),
            MercuryEvent::Keyboard(_) => Err(()),
        }
    }
}

pub struct MercuryStruct;
impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
    type Event = MercuryEvent;
    type Output = usize;
}

// Handlers. Each takes its node's path and returns the fired key's length.
pub fn on_esc(ev: &KeyEvent, node: Node<&mut App, ()>) -> usize {
    node.parent.hits += 1;
    ev.key.len()
}
pub fn on_f1(ev: &KeyEvent, _node: Node<LayerPath, ()>) -> usize {
    ev.key.len()
}
pub fn on_g(ev: &KeyEvent, mut node: Node<NavPath, ()>) -> usize {
    node.parent.get_mut().hits += 1;
    ev.key.len()
}
pub fn on_slack(ev: &FgEvent, _node: Node<NavPath, ()>) -> usize {
    ev.app.len()
}
pub fn on_bksp(ev: &KeyEvent, mut node: Node<TypingPath, ()>) -> usize {
    node.parent.get_mut().hits += 1;
    ev.key.len()
}
pub fn on_d(ev: &KeyEvent, mut node: Node<DeepPath, ()>) -> usize {
    node.parent.get_mut().hits += 1;
    ev.key.len()
}
/// A handler for nodes a dispatch test never fires (any path, ignored).
pub fn ignore<P>(ev: &KeyEvent, _node: Node<P, ()>) -> usize {
    ev.key.len()
}

// App -> Layer (enum) -> { Nav (leaf), Typing -> Box<Deep> (leaf) }.
#[derive(Bind)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(Keyboard("esc") => on_esc)]
pub struct App {
    pub hits: u32,
    #[resolve_into]
    pub layer: Layer,
}

#[derive(Bind)]
#[node(parent = AppPath)]
#[binds(MercuryStruct)]
#[bind(Keyboard("f1") => on_f1)]
pub enum Layer {
    Nav(Nav),
    Typing(Typing),
}

#[derive(Bind)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(Keyboard("g") => on_g, Foreground("Slack") => on_slack)]
pub struct Nav {
    pub hits: u32,
}

#[derive(Bind)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(Keyboard("bksp") => on_bksp)]
pub struct Typing {
    pub hits: u32,
    #[resolve_into]
    pub deep: Box<Deep>,
}

#[derive(Bind)]
#[node(parent = TypingPath)]
#[binds(MercuryStruct)]
#[bind(Keyboard("d") => on_d)]
pub struct Deep {
    pub hits: u32,
}

pub type AppPath<'a> = &'a mut App;
pub type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
pub type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;
pub type TypingPath<'a> = PathMut<Typing, LayerPath<'a>>;
pub type DeepPath<'a> = PathMut<Deep, TypingPath<'a>>;

// A tiny second tree for the duplicate-trigger error: parent and child both bind
// `dup`.
#[derive(Bind)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(Keyboard("dup") => ignore)]
pub struct Clash {
    #[resolve_into]
    pub child: ClashChild,
}

#[derive(Bind)]
#[node(parent = ClashPath)]
#[binds(MercuryStruct)]
#[bind(Keyboard("dup") => ignore)]
pub struct ClashChild {}

pub type ClashPath<'a> = &'a mut Clash;
pub type ClashChildPath<'a> = PathMut<ClashChild, ClashPath<'a>>;
// A no-binds leaf root.
#[derive(Bind)]
#[node(root)]
#[binds(MercuryStruct)]
pub struct Empty {}

// A multi-parent tree: `Title` is reached from both `Album` and `Song` through
// the `TitleParent` route enum.
#[derive(Bind)]
#[node(root)]
#[binds(MercuryStruct)]
pub enum Media {
    Album(Album),
    Song(Song),
}

#[derive(Bind)]
#[node(parent = MediaPath)]
#[binds(MercuryStruct)]
#[bind(Keyboard("a") => ignore)]
pub struct Album {
    #[resolve_into(parent = TitleParent)]
    pub title: Title,
}

#[derive(Bind)]
#[node(parent = MediaPath)]
#[binds(MercuryStruct)]
#[bind(Keyboard("s") => ignore)]
pub struct Song {
    #[resolve_into(parent = TitleParent)]
    pub title: Title,
}

#[derive(Bind)]
#[node(parent = TitleParent)]
#[binds(MercuryStruct)]
#[bind(Keyboard("t") => on_title)]
pub struct Title {
    pub hits: u32,
}

pub type MediaPath<'a> = &'a mut Media;
pub type AlbumPath<'a> = PathMut<Album, MediaPath<'a>>;
pub type SongPath<'a> = PathMut<Song, MediaPath<'a>>;
pub enum TitleParent<'a> {
    Album(AlbumPath<'a>),
    Song(SongPath<'a>),
}
pub type TitlePath<'a> = PathMut<Title, TitleParent<'a>>;
pub fn on_title(ev: &KeyEvent, mut node: Node<TitlePath, ()>) -> usize {
    node.parent.get_mut().hits += 1;
    ev.key.len()
}

/// A keyboard trigger, for accumulate assertions.
pub const fn kb(s: &'static str) -> MercuryTrigger {
    MercuryTrigger::Keyboard(Keyboard(s))
}
/// A foreground trigger, for accumulate assertions.
pub const fn fg(s: &'static str) -> MercuryTrigger {
    MercuryTrigger::Foreground(Foreground(s))
}
/// A fired keyboard event, for dispatch.
pub const fn key(s: &'static str) -> MercuryEvent {
    MercuryEvent::Keyboard(KeyEvent { key: s })
}
/// A fired foreground event, for dispatch.
pub const fn foreground(s: &'static str) -> MercuryEvent {
    MercuryEvent::Foreground(FgEvent { app: s })
}
