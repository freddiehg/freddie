//! A full laserbeam + bind tree shared by the accumulate and dispatch tests.
//!
//! Every node derives `Bind` (the path type the generated `Dispatch`
//! needs) and `Bind`. Handlers mutate their node's `hits` where it has one and
//! return the fired key's length, so a dispatch test can see which handler ran.
#![expect(dead_code)]

use bind::{AscendState, Bind, Bindings, EventTrigger};
use laserbeam::{Above, Complete, Completed, HasStop, MaybeInvalidated, PathMut};

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
pub enum DemoTrigger {
    Keyboard(Keyboard),
    Foreground(Foreground),
    WaitingFor(WaitingFor),
}
impl From<Keyboard> for DemoTrigger {
    fn from(k: Keyboard) -> Self {
        Self::Keyboard(k)
    }
}
impl From<Foreground> for DemoTrigger {
    fn from(f: Foreground) -> Self {
        Self::Foreground(f)
    }
}

pub enum DemoEvent {
    Keyboard(KeyEvent),
    Foreground(FgEvent),
}
impl<'a> TryFrom<&'a DemoEvent> for &'a KeyEvent {
    type Error = ();
    fn try_from(e: &'a DemoEvent) -> Result<Self, ()> {
        match e {
            DemoEvent::Keyboard(k) => Ok(k),
            DemoEvent::Foreground(_) => Err(()),
        }
    }
}
impl<'a> TryFrom<&'a DemoEvent> for &'a FgEvent {
    type Error = ();
    fn try_from(e: &'a DemoEvent) -> Result<Self, ()> {
        match e {
            DemoEvent::Foreground(f) => Ok(f),
            DemoEvent::Keyboard(_) => Err(()),
        }
    }
}

pub struct Demo;
impl Bindings for Demo {
    type Trigger = DemoTrigger;
    type Event = DemoEvent;
    type Output = Vec<usize>;
}

// Handlers. Each is the one scheduled shape: the event, what its pre snapped, and the state the
// descent left of its path. They return the fired key's length, so a dispatch test can see which
// one ran, and the leave they completed to.
//
// A stayer completes where it stands; the `Invalidated` arm forwards the leave it was handed and
// is unreachable for a leaf, whose state starts standing.
pub fn on_esc<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, AppPath<'x>>,
) -> (Vec<usize>, Completed<AppPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(app) => {
            app.hits += 1;
            (vec![ev.key.len()], app.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}
pub fn on_f1<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, LayerPath<'x>>,
) -> (Vec<usize>, Completed<LayerPath<'x>>) {
    (vec![ev.key.len()], st.complete())
}
pub fn on_g<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, NavPath<'x>>,
) -> (Vec<usize>, Completed<NavPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut nav) => {
            nav.get_mut().hits += 1;
            (vec![ev.key.len()], nav.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}
pub fn on_slack<'x>(
    ev: &FgEvent,
    _snap: (),
    st: AscendState<'_, NavPath<'x>>,
) -> (Vec<usize>, Completed<NavPath<'x>>) {
    (vec![ev.app.len()], st.complete())
}
pub fn on_bksp<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, TypingPath<'x>>,
) -> (Vec<usize>, Completed<TypingPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut typing) => {
            typing.get_mut().hits += 1;
            (vec![ev.key.len()], typing.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}
pub fn on_d<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, DeepPath<'x>>,
) -> (Vec<usize>, Completed<DeepPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut deep) => {
            deep.get_mut().hits += 1;
            (vec![ev.key.len()], deep.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}
/// A handler for the armed node: clears what it was waiting on, so a test can see it ran.
pub fn on_armed<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ArmedPath<'x>>,
) -> (Vec<usize>, Completed<ArmedPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(armed) => {
            armed.waiting_for = None;
            (vec![ev.key.len()], armed.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}

/// A handler for nodes a dispatch test never fires. It reads nothing, so it binds at any place:
/// the bounds are the two every stayer needs, and neither names a node.
pub fn ignore<P: HasStop + Complete<P>>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<usize>, Completed<P>) {
    (vec![ev.key.len()], st.complete())
}

// App -> Layer (enum) -> { Nav (leaf), Typing -> Box<Deep> (leaf) }.
#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
#[bind(Keyboard("esc") => on_esc)]
pub struct App {
    pub hits: u32,
    #[resolve_into]
    pub layer: Layer,
}

#[derive(Bind)]
#[node(parent = AppPath)]
#[binds(Demo)]
#[bind(Keyboard("f1") => on_f1)]
pub enum Layer {
    Nav(Nav),
    Typing(Typing),
}

#[derive(Bind)]
#[node(parent = LayerPath)]
#[binds(Demo)]
#[bind(Keyboard("g") => on_g, Foreground("Slack") => on_slack)]
pub struct Nav {
    pub hits: u32,
}

#[derive(Bind)]
#[node(parent = LayerPath)]
#[binds(Demo)]
#[bind(Keyboard("bksp") => on_bksp)]
pub struct Typing {
    pub hits: u32,
    #[resolve_into]
    pub deep: Box<Deep>,
}

#[derive(Bind)]
#[node(parent = TypingPath)]
#[binds(Demo)]
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
#[binds(Demo)]
#[bind(Keyboard("dup") => ignore)]
pub struct Clash {
    #[resolve_into]
    pub child: ClashChild,
}

#[derive(Bind)]
#[node(parent = ClashPath)]
#[binds(Demo)]
#[bind(Keyboard("dup") => ignore)]
pub struct ClashChild;

pub type ClashPath<'a> = &'a mut Clash;
pub type ClashChildPath<'a> = PathMut<ClashChild, ClashPath<'a>>;
// A no-binds leaf root.
#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
pub struct Empty;

// A multi-parent tree: `Title` is reached from both `Album` and `Song` through
// the `TitleParent` route enum.
#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
pub enum Media {
    Album(Album),
    Song(Song),
}

#[derive(Bind)]
#[node(parent = MediaPath)]
#[binds(Demo)]
#[bind(Keyboard("a") => ignore)]
pub struct Album {
    #[resolve_into(parent = TitleParent, up = TitleParentUp)]
    pub title: Title,
}

#[derive(Bind)]
#[node(parent = MediaPath)]
#[binds(Demo)]
#[bind(Keyboard("s") => ignore)]
pub struct Song {
    #[resolve_into(parent = TitleParent, up = TitleParentUp)]
    pub title: Title,
}

#[derive(Bind)]
#[node(parent = TitleParent)]
#[binds(Demo)]
#[bind(Keyboard("t") => on_title, Keyboard("home") => title_home)]
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

/// What a leave from `Title` hands upward once it has peeled past the route: which route it
/// took, and how far it went from there.
///
/// The consumer writes this half, as it writes the route enum itself. A route enum is the one
/// parent slot laserbeam cannot build a path through, so it cannot build the `Up` payload
/// either: only the consumer knows which parents the slot can hold.
pub enum TitleParentUp<'a> {
    Album(Completed<AlbumPath<'a>>),
    Song(Completed<SongPath<'a>>),
}

impl<'a> Above for TitleParent<'a> {
    type Up = TitleParentUp<'a>;
}

pub type TitlePath<'a> = PathMut<Title, TitleParent<'a>>;
pub fn on_title<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, TitlePath<'x>>,
) -> (Vec<usize>, Completed<TitlePath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut title) => {
            title.get_mut().hits += 1;
            (vec![ev.key.len()], title.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}

/// Title's leave, on `home`: out through whichever route is live, to the root.
///
/// `into_parent()` on a `TitlePath` yields the route enum, which has no `into_parent` of its
/// own, so the leave matches it and wraps one `Up` level by hand. Both arms are live, one per
/// route, and `IntoAncestor` does not cross a route enum, so no generic go-home handler could
/// stand in for this.
pub fn title_home<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, TitlePath<'x>>,
) -> (Vec<usize>, Completed<TitlePath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(title) => {
            let up = match title.into_parent() {
                TitleParent::Album(album) => TitleParentUp::Album(album.into_parent().complete()),
                TitleParent::Song(song) => TitleParentUp::Song(song.into_parent().complete()),
            };
            (vec![], Completed::up(up))
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}

/// A keyboard trigger, for accumulate assertions.
pub const fn kb(s: &'static str) -> DemoTrigger {
    DemoTrigger::Keyboard(Keyboard(s))
}
/// A foreground trigger, for accumulate assertions.
pub const fn fg(s: &'static str) -> DemoTrigger {
    DemoTrigger::Foreground(Foreground(s))
}
/// A fired keyboard event, for dispatch.
pub const fn key(s: &'static str) -> DemoEvent {
    DemoEvent::Keyboard(KeyEvent { key: s })
}
/// A `WaitingFor` trigger, for an accumulate assertion.
#[must_use]
pub const fn waiting(k: Option<&'static str>) -> DemoTrigger {
    DemoTrigger::WaitingFor(WaitingFor(k))
}

/// A fired foreground event, for dispatch.
pub const fn foreground(s: &'static str) -> DemoEvent {
    DemoEvent::Foreground(FgEvent { app: s })
}

// A trigger whose value is read from the node it is bound on: it matches a key only while the node
// is waiting for that key. The closure form is what supplies the value.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct WaitingFor(pub Option<&'static str>);

impl EventTrigger for WaitingFor {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool {
        self.0 == Some(ev.key)
    }
}

impl From<WaitingFor> for DemoTrigger {
    fn from(w: WaitingFor) -> Self {
        Self::WaitingFor(w)
    }
}

pub type ArmedPath<'a> = &'a mut Armed;
pub type ArmedChildPath<'a> = PathMut<ArmedChild, ArmedPath<'a>>;

/// A root whose binding reads its own state, beside a constant one, so the two forms coexist.
#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
#[bind(
    |armed_path| WaitingFor(armed_path.waiting_for) => on_armed,
    Keyboard("esc") => on_esc_armed,
)]
pub struct Armed {
    pub waiting_for: Option<&'static str>,
    /// What the CHILD's parent-reading binding watches for, kept separate so it cannot collide
    /// with this node's own trigger.
    pub for_child: Option<&'static str>,
    #[resolve_into]
    pub child: ArmedChild,
}

/// A deeper node, so a closure reads through a `PathMut` rather than a `&mut Root`.
///
/// Its second binding reads the level ABOVE through `parent`, which is what a shared path buys:
/// the child answers with what its root is waiting for.
#[derive(Bind)]
#[node(parent = ArmedPath)]
#[binds(Demo)]
#[bind(
    // An `Option` trigger: absent when the node holds nothing, and absent matches nothing.
    |armed_child_path| armed_child_path.get().wants.map(Keyboard) => on_child_armed,
    |armed_child_path| Keyboard(armed_child_path.parent().for_child.unwrap_or("none")) => on_parents_key,
)]
pub struct ArmedChild {
    pub wants: Option<&'static str>,
}

/// Fires for the key the child's PARENT is waiting for, read through `parent()`.
pub fn on_parents_key<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ArmedChildPath<'x>>,
) -> (Vec<usize>, Completed<ArmedChildPath<'x>>) {
    (vec![ev.key.len() + 100], st.complete())
}

pub fn on_esc_armed<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ArmedPath<'x>>,
) -> (Vec<usize>, Completed<ArmedPath<'x>>) {
    (vec![ev.key.len()], st.complete())
}

pub fn on_child_armed<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, ArmedChildPath<'x>>,
) -> (Vec<usize>, Completed<ArmedChildPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut child) => {
            child.get_mut().wants = None;
            (vec![ev.key.len()], child.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![ev.key.len()], c),
    }
}
