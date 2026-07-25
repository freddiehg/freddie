# Derived levels: ascend at the place

Not done. Depends on `invalidation.md` changes 0–4 (landed). Change 2 below is invalidation.md's change 5: the codegen flip and the workspace-global handler migration land together there. Not yet compile-checked.

## Model

A derived level keeps its two sides distinct:

- Descent, unchanged: `f(&parent)` produces `Data`, the derive builds `Node { parent, data }`, and triggers and pres read `&Node` — own data, ancestor data, and the tree through the path, all shared.
- Ascent: the place path at the bottom of the `Node` chain. A derived level's scheduled items receive `AscendState<Place>` and return `Completed<Place>`; the `Node` is consumed by the descent and never comes back. Data a handler needs on the way up is snapped by its pre.

Place per level: mercury's `AppData` / `ChromeApp` / `GhosttyApp` under `Node<AppLayerPath, _>` ascend at `AppLayerPath` (site mirrors it); the test tree's `AppData` under `Node<ShellPath, AppData>` ascends at `ShellPath`, and the nested `TabData` under `Node<AppNode, TabData>` ascends at `ShellPath` too.

The state a derived item receives means what it means everywhere: has the place beneath me been destroyed. The derived data is not state — it is rebuilt from the tree on every dispatch — so it rides in the snap, never in `MaybeInvalidated`. Whether the level existed at all is likewise a pre's question (call `app_data(&path)`, or read the root fields it reads), not the fold's.

Because `Place` is a real place path, one handler shape serves places and derived levels: a generic unit handler bound as `P: HasAncestor<MercuryPath<'a>> + HasStop + Complete<P>` binds at a derived leaf exactly as at a layer, which is what `mercury-post-patterns.md`'s Chrome sketches assume. `Node<P, D>` never appears in an ascent signature.

## bind additions

The projection from a parent chain to its place path. A path is its own place; a derived node's is its parent's:

```rust
pub trait HasPlace {
    type Place;
    fn into_place(self) -> Self::Place;
}

impl<'a, R> HasPlace for &'a mut R {
    type Place = &'a mut R;
    fn into_place(self) -> Self {
        self
    }
}

impl<N, P> HasPlace for ::laserbeam::PathMut<N, P> {
    type Place = ::laserbeam::PathMut<N, P>;
    fn into_place(self) -> Self {
        self
    }
}

impl<Parent: HasPlace, Data> HasPlace for Node<Parent, Data> {
    type Place = Parent::Place;
    fn into_place(self) -> Parent::Place {
        self.parent.into_place()
    }
}
```

`DispatchIntoParent` is deleted and replaced. The place-side `dispatch_into_parent_impl` emission is deleted with it and gets no replacement: its only caller has always been the derived-child descent, which calls `Node` impls, so places emit nothing and a route-parented node needs no marker to skip anything.

```rust
/// Consumes a derived node, dispatches at that level, and surfaces at the
/// place path beneath it.
pub trait DispatchIntoPlace<M: Bindings>: HasPlace + Sized
where
    Self::Place: ::laserbeam::HasStop,
{
    fn dispatch_into_place(
        self,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'_>,
    ) -> ::laserbeam::Completed<Self::Place>;
}
```

`HasParent` stays: pres walk `node.parent`, and the route fold uses it. The check halves (`DerivedHandler`, `EventHandler`) are untouched: feature-gated, ignored by this design, and they read only triggers, which still take `&Node`.

## Generated (targets, on the test tree in `tests/derived.rs`)

A place with `#[derived_child(app_data)]` — Shell's `#state`, the same fold shape as a place child's:

```rust
let mut state = match app_data(&path) {
    ::core::option::Option::Some(data) => ::laserbeam::Completed::to_maybe_invalidated(
        ::bind::DispatchIntoPlace::<Demo>::dispatch_into_place(
            ::bind::Node { parent: path, data },
            event,
            effs,
            claim,
        ),
    ),
    ::core::option::Option::None => ::laserbeam::MaybeInvalidated::NotInvalidated(
        ::bind::HasPlace::into_place(path),
    ),
};
```

One template serves both sides of the edge: the `None` arm is always `NotInvalidated(into_place(..))`, which is the identity for a place.

A derived level with its own derived child — `AppData`:

```rust
impl<'a> ::bind::DispatchIntoPlace<Demo> for ::bind::Node<ShellPath<'a>, AppData> {
    fn dispatch_into_place(
        self,
        event: &DemoEvent,
        effs: &mut Vec<usize>,
        claim: &mut ::bind::Claim<'_>,
    ) -> ::laserbeam::Completed<ShellPath<'a>> {
        let node = self;

        let opt_0 = match ::core::convert::TryFrom::try_from(event) {
            ::core::result::Result::Ok(ev) => {
                let trigger = Keyboard("r");
                if ::bind::EventTrigger::is_matching(&trigger, ev) {
                    ::core::option::Option::Some((ev, (snap_tab)(ev, &node)))
                } else {
                    ::core::option::Option::None
                }
            }
            ::core::result::Result::Err(_) => ::core::option::Option::None,
        };

        let mut state = match tab_data(&node) {
            ::core::option::Option::Some(data) => ::laserbeam::Completed::to_maybe_invalidated(
                ::bind::DispatchIntoPlace::<Demo>::dispatch_into_place(
                    ::bind::Node { parent: node, data },
                    event,
                    effs,
                    claim,
                ),
            ),
            ::core::option::Option::None => ::laserbeam::MaybeInvalidated::NotInvalidated(
                ::bind::HasPlace::into_place(node),
            ),
        };

        if let ::core::option::Option::Some((ev, snap)) = opt_0 {
            let (e, completed) = (::bind::exclusive(on_r))(
                ev,
                snap,
                ::bind::AscendState::new(state, ::bind::Claim::reborrow(claim)),
            );
            ::core::iter::Extend::extend(effs, e);
            state = ::laserbeam::Completed::to_maybe_invalidated(completed);
        }

        state.complete()
    }
}
```

The opts snap while the node is whole; the descent then consumes it. The `None` arm still holds the node and flattens it; the `Some` arm gets the place back inside the child's `Completed`.

The nested leaf — `TabData`:

```rust
impl<'a> ::bind::DispatchIntoPlace<Demo> for ::bind::Node<AppNode<'a>, TabData> {
    fn dispatch_into_place(
        self,
        event: &DemoEvent,
        effs: &mut Vec<usize>,
        claim: &mut ::bind::Claim<'_>,
    ) -> ::laserbeam::Completed<ShellPath<'a>> {
        let node = self;

        let opt_0 = match ::core::convert::TryFrom::try_from(event) {
            ::core::result::Result::Ok(ev) => {
                let trigger = Keyboard("g");
                if ::bind::EventTrigger::is_matching(&trigger, ev) {
                    ::core::option::Option::Some((ev, (snap_tab_thread)(ev, &node)))
                } else {
                    ::core::option::Option::None
                }
            }
            ::core::result::Result::Err(_) => ::core::option::Option::None,
        };

        let mut state = ::laserbeam::MaybeInvalidated::NotInvalidated(
            ::bind::HasPlace::into_place(node),
        );

        if let ::core::option::Option::Some((ev, snap)) = opt_0 {
            let (e, completed) = (::bind::exclusive(on_g))(
                ev,
                snap,
                ::bind::AscendState::new(state, ::bind::Claim::reborrow(claim)),
            );
            ::core::iter::Extend::extend(effs, e);
            state = ::laserbeam::Completed::to_maybe_invalidated(completed);
        }

        state.complete()
    }
}
```

The enum level — mercury's `AppData`; every arm returns the same `Completed<AppLayerPath>`, so the match is total with no dead arms. The existing rule stands: an enum of derived levels binds nothing itself (the derive keeps its error), so the enum impl has no opts or scheduled items of its own, only the arms:

```rust
impl<'a> ::bind::DispatchIntoPlace<MercuryStruct> for ::bind::Node<AppLayerPath<'a>, AppData> {
    fn dispatch_into_place(
        self,
        event: &MercuryEvent,
        effs: &mut Vec<MercuryEffect>,
        claim: &mut ::bind::Claim<'_>,
    ) -> ::laserbeam::Completed<AppLayerPath<'a>> {
        let ::bind::Node { parent, data } = self;
        match data {
            AppData::Chrome(data) => ::bind::DispatchIntoPlace::<MercuryStruct>::dispatch_into_place(
                ::bind::Node { parent, data },
                event,
                effs,
                claim,
            ),
            AppData::Ghostty(data) => ::bind::DispatchIntoPlace::<MercuryStruct>::dispatch_into_place(
                ::bind::Node { parent, data },
                event,
                effs,
                claim,
            ),
        }
    }
}
```

## bind_macro (before / after)

`derived_child_descent`, before (early return, parent handed back on a miss):

```rust
let #place = match #f(&#place) {
    ::core::option::Option::Some(data) => {
        match ::bind::DispatchIntoParent::<#marker>::dispatch_into_parent(
            ::bind::Node { parent: #place, data },
            event,
            effs,
            claim,
        ) {
            ::core::option::Option::None => return ::core::option::Option::None,
            ::core::option::Option::Some(p) => p,
        }
    }
    ::core::option::Option::None => #place,
};
```

After: the `#state` fold shown under Generated, one template for a place's `#[derived_child]` (over `path`) and a derived level's own descent (over `node`); the `None` arm is always `NotInvalidated(into_place(..))`, the identity for a place.

`derived_node_impl`, before: `impl DispatchIntoParent`, old-form checks (claim, collect, `return None`), fallthrough `Some(into_parent(node))`. After, the emitted template — `dispatch_impl`'s linear shape over `node`:

```rust
#[automatically_derived]
impl<'a> ::bind::DispatchIntoPlace<#marker> for ::bind::Node<#parent<'a>, #name> {
    fn dispatch_into_place(
        self,
        event: &<#marker as ::bind::Bindings>::Event,
        effs: &mut <#marker as ::bind::Bindings>::Output,
        claim: &mut ::bind::Claim<'_>,
    ) -> ::laserbeam::Completed<
        <::bind::Node<#parent<'a>, #name> as ::bind::HasPlace>::Place,
    > {
        let node = self;
        #(#opts)*
        let mut state = #state;
        #(#scheduled)*
        state.complete()
    }
}
```

The derive cannot name the place path (`TabData`'s attribute names only `AppNode`), so the return type spells it through the `HasPlace` projection, which resolves because `#parent` is concrete at the impl. Opts, trigger closures, and pres emit over `&node` where a place emits over `&path`; the scheduled list is the same source-ordered list across `#[bind]` / `#[post]` / `#[pre_post]` that a place assembles; `#state` is the `into_place` flatten for a level with no derived child, or the fold above for one with a `#[derived_child]`.

`derived_enum_node_impl`, before/after: the arms call `dispatch_into_place` and the impl returns `Completed<Place>`; otherwise unchanged.

`dispatch_into_parent_impl`: deleted, for every place. Nothing calls it, and its `Self::Parent: HasStop` bound is unsatisfiable for route-parented nodes; invalidation.md's route section loses the `#[node(parent = .., route)]` marker accordingly.

## Handlers

One signature everywhere: `FnOnce(&Ev, Snap, AscendState<'a, Place>) -> (Vec<E>, Completed<Place>)`. A derived `#[bind]` synthesizes `|_, _| ()`, so it carries no data; a derived handler that reads data is a `#[pre_post]` whose pre snaps what it needs off `&Node`, with the rhs wrapped in `exclusive` when it claims. Handwritten claimers add `exclusive` to their `use bind::...;` line.

`tests/derived.rs` migrates to:

```rust
#[derive(Bind)]
#[derived_node(parent = ShellPath)]
#[binds(Demo)]
#[derived_child(tab_data)]
#[pre_post(Keyboard("r") => (snap_tab, exclusive(on_r)))]
pub struct AppData {
    pub tab: String,
}

#[derive(Bind)]
#[derived_node(parent = AppNode)]
#[binds(Demo)]
#[pre_post(Keyboard("g") => (snap_tab_thread, exclusive(on_g)))]
pub struct TabData {
    pub thread: u32,
}

fn snap_tab(_ev: &KeyEvent, node: &AppNode) -> String {
    node.data.tab.clone()
}

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
```

`on_esc` (Shell's, a place bind) migrates under invalidation change 5's global migration and is not listed here.

Mercury: no derived handler reads `data` (`ChromeApp` and `GhosttyApp` are units; `ClaudeAiSite` matches), so every derived-level handler migrates exactly like a place handler — the `Node<P, D>` parameter disappears and the bounds move onto the place path `P`. Affected: `refresh`, `focus_address_bar`, `copy_url`, `copy_host`, the tmux window handlers in `handlers/app.rs`, `new_chat` on the site side, and the layer handlers (`to_home`, `to_nav`, `to_site`, `to_typing`, `toggle_overlay`) where bound on derived levels.

Two conventions the migration applies uniformly:

- A stayer that reads the tree reads through `HasAncestor::ancestor` and completes where it stands; only a leaver consumes through `IntoAncestor`. `copy` therefore takes the root by shared reference instead of consuming a path; `focus_address_bar`, which leaves into typing, keeps its `IntoAncestor` walk.
- A handler whose effect needs the path emits nothing on `Invalidated` and forwards `c`. On a derived leaf the arm cannot fire in practice, since a leaf's `#state` starts `NotInvalidated`, but the match is total.

`copy_url` is the model for mercury's stayers. Before:

```rust
pub(crate) fn copy_url<'a, E, P: IntoAncestor<MercuryPath<'a>>, D>(
    _ev: &E,
    node: Node<P, D>,
) -> Vec<MercuryEffect> {
    copy(node.parent, UrlPart::Whole)
}

fn copy<'a, P: IntoAncestor<MercuryPath<'a>>>(path: P, part: UrlPart) -> Vec<MercuryEffect> {
    let root: MercuryPath<'_> = path.into_ancestor();
    // ... reads root.foreground ...
}
```

After — `copy` takes `&MercuryStruct` and its body loses only the `into_ancestor` line:

```rust
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
            let effs = copy(path.ancestor(), UrlPart::Whole);
            (effs, path.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}

fn copy(root: &MercuryStruct, part: UrlPart) -> Vec<MercuryEffect> {
    // ... reads root.foreground, unchanged ...
}
```

`copy_host` follows it with `UrlPart::Host`; `refresh` and the tmux handlers are pure effects and need no match at all (`(vec![tap(..)], st.complete())`).

## Tests

- `tests/derived.rs`: the four dispatch tests and the accumulate test keep their exact assertions; the handlers migrate as above.
- HasPlace units (change 1): `into_place` at each shape — root path, `PathMut`, `Node` one and two levels deep.
- A derived leave invalidates for the place's later items (change 2): `AppData` gains `#[bind(Keyboard("q") => app_home)]`, Shell gains `#[post(Keyboard("q") => log_leave)]`; the walk asserts `(vec![9, 7], true)`:

```rust
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
```

## Ordered changes

### 1 — bind: `HasPlace` + `DispatchIntoPlace`, pure additions with unit tests; nothing consumes them

### 2 — the codegen flip, `DispatchIntoParent` deleted, handler migration

This is invalidation.md's change 5, not a separate change: the handler signature change is workspace-global, and the derived deltas above are its derived portion. The migrated `tests/derived.rs` handlers need `#[pre_post]`, which is why that parsing is part of change 5 rather than change 6.

## Rules

1. Descent reads the `Node`; ascent holds the place. Data rides in snaps.
2. `DispatchIntoPlace` exists only for `Node`s; no place emits an into-parent dispatch impl.
3. One handler signature everywhere; `Node<P, D>` never appears in an ascent signature; bounds go on the place path.
4. A derived edge folds exactly like a place edge: `Completed<Place>` through `to_maybe_invalidated`.
5. Whether a derived level exists is a pre's question, never the fold's.
