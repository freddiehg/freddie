# resolution

One shape for every handler: a parent, plus the immutable data that level produced. Some levels produce `()`. Some produce an object, built by a derived child fn and stored nowhere.

`Path` is not a kind of `Node`. A `Path` ADDRESSES a place in the tree, and `get_mut()` is how you reach it. A `Node` CARRIES data that is not in the tree. Both sit next to a parent, and that is the whole of the resemblance. A `Path` appears in this design as a `Node`'s `parent`, never as its `data`.

Not implemented.

## The problem, today

Mercury stores the foregrounded app twice. `root.foregrounded` holds it, and the in-app layer holds it AGAIN as an enum variant whose entire content is the discriminant:

```rust
// crates/mercury/src/lib.rs
pub enum AppLayer { Chrome(ChromeApp), Ghostty(GhosttyApp), Other(OtherApp) }
pub struct ChromeApp {}          // empty. The variant IS the data.
```

`on_foregrounded` re-derives the duplicate by hand on every foreground event:

```rust
root.foregrounded = ev.app;
if let Layer::InApp(in_app) = &mut root.layer {
    *in_app = AppLayer::for_app(ev.app);
}
```

`AppLayer`, the three empty structs, and that resync all get deleted.

## The node

```rust
pub struct Node<Parent, Data> {
    /// What the level above handed down.
    pub parent: Parent,
    /// The immutable data this level produced.
    pub data: Data,
}
```

Every handler takes one. The two fields never change meaning:

- `parent` is a `laserbeam::Path` when the level above is a place, and a `Node` when it is derived. `get_mut()` on a `Path` gives exactly what that path's type says. There is no `get_mut()` on a `Node`, because there is nothing to project into.
- `data` is `()` for a `#[resolve_into]` child, and an object for a derived one. It is never a projection.

## The hierarchy

It bottoms out at the root, and each level names the one above it.

```
&mut Root                                    the root
Node<Path<InAppLayer, &mut Root>, ()>        InAppLayer   a PLACE:   parent is its own path
Node<InAppLayerPath, ChromeInfo>             Chrome       DERIVED:   parent is the layer's path
Node<ChromeNode, GmailInfo>                  Gmail        DERIVED:   parent is Chrome's node
```

A place's `parent` is its own `Path`, which is already parent-plus-projection, so it encodes both where it is and what is above it. A derived level has no place of its own, so its `parent` is whatever the level above holds.

Every level of the hierarchy is reachable from any handler:

```rust
inapp.parent.get_mut()           // &mut InAppLayer
inapp.data                       // ()

chrome.data.tab                  // String, the clone. No match, no unwrap.
chrome.parent.get_mut()          // &mut InAppLayer

gmail.data.thread                // u32
gmail.parent.data.tab            // String, Chrome's data
gmail.parent.parent.get_mut()    // &mut InAppLayer
```

## The two attributes

Every child edge is declared twice, once on the parent and once on the child. The derived kind mirrors the place kind exactly.

```
a PLACE child
  parent side   #[resolve_into] pub layer: InAppLayer      my child IS this field
  child side    #[laserbeam(path = InAppLayerPath)]        my node type is this

a DERIVED child
  parent side   #[derived_child(chrome)]                   my child is what `chrome` returns
  child side    #[derived_node(parent = InAppLayerPath)]   my parent is this
```

`#[derived_node]` is the counterpart of `#[laserbeam(path = ..)]`, not of `#[resolve_into]`. Both sit on the CHILD, and both exist to tell the derive its PARENT, which is the one thing a derive cannot see.

A place says it indirectly, through an alias that has the parent inside it:

```rust
pub type InAppLayerPath<'a> = Path<InAppLayer, LayerPath<'a>>;
//                                             ^^^^^^^^^ the parent
```

A derived level says it directly, because the derive already knows its own name and can build the node type itself:

```rust
impl<'a> Descend<M> for Node<InAppLayerPath<'a>, ChromeInfo> { .. }
//                          ^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^
//                          from the attribute  from the struct
```

So no alias is required. `pub type ChromeNode<'a> = Node<InAppLayerPath<'a>, ChromeInfo>;` is a convenience the user writes only to shorten handler signatures.

## Why a derived level is not just `#[derive(Laserbeam)]`

Laserbeam emits `impl Resolve` with `type Path<'a> = Path<ChromeInfo, ..>`, and a `Path` is a projection into memory that exists:

```rust
pub fn from_fn(parent: Parent, projection: fn(&mut Parent) -> &mut Node) -> Self
```

There is no `fn(&mut InAppLayer) -> &mut ChromeInfo` to give it. `ChromeInfo` lives nowhere: the derived child fn built it and the node owns it.

And `Dispatch` requires `Resolve`:

```rust
pub trait Dispatch<M: Bindings>: ::laserbeam::Resolve { .. }
```

So a level with no place in the tree cannot have `Dispatch`, which is why `Descend` exists. `Path` and `Node` are the two payloads a node can carry, a projection or owned data, and laserbeam is the projection half.

## What the user writes

```rust
#[derive(Laserbeam, Bind)]
#[laserbeam_root(resolved = R)]
#[binds(MercuryStruct)]
pub struct Root {
    pub app: App,                     // the ONLY copy of the foregrounded app, AND its state
    #[resolve_into]
    pub layer: InAppLayer,
}

/// The app, and whatever mercury knows about it. `on_foregrounded` writes this, and it is the
/// only writer. A derived child fn reads it and clones out of it.
pub enum App {
    Chrome(ChromeState),              // { tab: String }
    Ghostty(GhosttyState),            // { pane: u8 }
    Other,                            // no bindings, so no state and no struct
}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = InAppLayerPath, resolved = R)]
#[binds(MercuryStruct)]
#[derived_child(chrome)]                   // its child is not a field
#[bind(Key::Escape.down() => on_escape)]
pub struct InAppLayer {
    pub log: String,                  // and it stores no app
}

/// The immutable data a derived level produces. Never empty: a level with no data is a level
/// with no bindings, and it gets no struct.
#[derive(Bind)]
#[derived_node(parent = InAppLayerPath)]
#[binds(MercuryStruct)]
#[derived_child(gmail)]                    // a derived level can have its own child
#[bind(Key::KeyR.down() => on_chrome_r)]
pub struct ChromeInfo {
    pub tab: String,
}

#[derive(Bind)]
#[derived_node(parent = ChromeNode)]
#[binds(MercuryStruct)]
#[bind(Key::KeyJ.down() => on_gmail_j)]
pub struct GmailInfo {
    pub thread: u32,
}

pub type InAppLayerPath<'a> = Path<InAppLayer, &'a mut Root>;
pub type InAppNode<'a> = Node<InAppLayerPath<'a>, ()>;
pub type ChromeNode<'a> = Node<InAppLayerPath<'a>, ChromeInfo>;
pub type GmailNode<'a> = Node<ChromeNode<'a>, GmailInfo>;
```

## The derived child fn

`fn(&Parent) -> Option<Data>`.

It takes a SHARED reference, so it cannot mutate. A derived child fn derives; it does not act. It also cannot lose the parent, because it never had it.

It returns only the DATA. The derive builds the node.

```rust
fn chrome(path: &InAppLayerPath) -> Option<ChromeInfo> {
    match &path.parent().app {                       // the only match on root.app
        App::Chrome(s) => Some(ChromeInfo { tab: s.tab.clone() }),
        _ => None,                                   // no bindings: no node, no struct
    }
}

/// A derived child fn on a DERIVED level. Same shape; `&Parent` is a `&Node`.
fn gmail(node: &ChromeNode) -> Option<GmailInfo> {
    (node.data.tab == "gmail.com").then(|| GmailInfo { thread: 0 })
}
```

`Option` and not `ControlFlow`, because nothing moves. An earlier signature took the parent by value, which meant it had to hand the parent back on absence, which meant `ControlFlow`. Taking `&Parent` deletes that.

Everything a derived child fn returns is derived from the tree. It has a shared reference and nothing else: no OS query, no clock, no side channel. It runs on every dispatch, so anything it reaches would be read on every keystroke, and anything it could not reach from the tree would make its result unreproducible from state. `on_foregrounded` is what puts the app's state in the tree; the derived child fn only clones out of it.

## Several possible children

The DATA is an enum. There is no separate mechanism.

```rust
pub enum AppData {
    Chrome(ChromeInfo),
    Ghostty(GhosttyInfo),
}

fn app_data(path: &InAppLayerPath) -> Option<AppData> {
    match &path.parent().app {
        App::Chrome(s) => Some(AppData::Chrome(ChromeInfo { tab: s.tab.clone() })),
        App::Ghostty(s) => Some(AppData::Ghostty(GhosttyInfo { pane: s.pane })),
        App::Other => None,
    }
}
```

The derive destructures and rebuilds the node per variant, so each variant's handler gets its own `Data`:

```rust
match node.data {
    AppData::Chrome(data)  => Descend::<M>::dispatch(Node { parent: node.parent, data }, event),
    AppData::Ghostty(data) => Descend::<M>::dispatch(Node { parent: node.parent, data }, event),
}
```

## The handlers

All the same shape.

```rust
fn on_escape(_ev: &KeyEvent, mut node: InAppNode) -> Out {
    node.parent.get_mut().log.push('e');
    let () = node.data;
    vec![]
}

fn on_chrome_r(_ev: &KeyEvent, mut node: ChromeNode) -> Out {
    let tab = node.data.tab.clone();
    node.parent.get_mut().log.push_str(&tab);
    vec![]
}

fn on_gmail_j(_ev: &KeyEvent, mut node: GmailNode) -> Out {
    let thread = node.data.thread;
    let tab = node.parent.data.tab.clone();
    node.parent.parent.get_mut().log.push_str(&format!("{tab}:{thread}"));
    vec![]
}
```

`data` is owned, so writing it changes nothing outside the node. It cannot be a reference into the tree: the node holds an `&mut Root` at the bottom of its parent chain, so a reference would alias it. E0505 and E0515.

## What the derive emits

For a PLACE, this is `cargo expand` output, with the trigger written in mercury's spelling rather than the spike's. The only change from what the derive emits today is that the handler is handed a `Node` rather than a bare path.

The trigger is whatever expression the `#[bind]` carried, verbatim, because the derive has tokens. `Key::KeyR.down()` is a `KeyPress { key, press }` constructor from `freddie_keys`.

```rust
impl ::bind::Dispatch<MercuryStruct> for InAppLayer {
    fn dispatch<'a>(
        path: <Self as ::laserbeam::Resolve>::Path<'a>,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> ::core::ops::ControlFlow<
        <MercuryStruct as ::bind::Bindings>::Output,
        <Self as ::laserbeam::Resolve>::Path<'a>,
    >
    where
        Self: 'a,
    {
        if let ::core::option::Option::Some(ev) = ::core::result::Result::ok(
            ::core::convert::TryFrom::try_from(event),
        ) {
            let trigger = Key::Escape.down();
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return ::core::ops::ControlFlow::Break(
                    on_escape(
                        ev,
                        ::bind::Node { parent: path, data: () },
                    ),
                );
            }
        }
        ::core::ops::ControlFlow::Continue(path)
    }
}
```

For `#[derived_child(f)]`, the descent. The derive builds the node.

```rust
let path = match chrome(&path) {
    ::core::option::Option::Some(data) => {
        ::bind::Descend::<MercuryStruct>::dispatch(
            ::bind::Node { parent: path, data },
            event,
        )?
    }
    ::core::option::Option::None => path,      // never moved
};
// then this node's own binds, unchanged
```

The derive names no type it cannot see. It has the token `chrome` and nothing else; `data`'s type comes from that fn's return, and inference resolves `Descend` from the `Node` it builds.

For `#[derived_node]`, this level's binds, then hand the parent back.

```rust
impl<'a> ::bind::Descend<MercuryStruct> for ChromeNode<'a> {
    fn dispatch(
        self,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> ::core::ops::ControlFlow<
        <MercuryStruct as ::bind::Bindings>::Output,
        <Self as ::bind::HasParent>::Parent,
    > {
        // its own #[derived_child(gmail)] descent first, then:
        if let ::core::option::Option::Some(ev) = ::core::result::Result::ok(
            ::core::convert::TryFrom::try_from(event),
        ) {
            let trigger = Key::KeyR.down();
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return ::core::ops::ControlFlow::Break(on_chrome_r(ev, self));
            }
        }
        ::core::ops::ControlFlow::Continue(::bind::HasParent::into_parent(self))
    }
}
```

### Why `#[derived_node]` needs the parent

That impl header is `Node<InAppLayerPath<'a>, ChromeInfo>`. The derive is on `ChromeInfo`, so it has the second half. Nothing in `ChromeInfo`'s tokens mentions `InAppLayer`, so it does not have the first, and `parent = InAppLayerPath` is the telling.

It could be avoided with a blanket impl over the parent, which compiles:

```rust
fn on_chrome_r<'a, P: Ascend<InAppLayerPath<'a>>>(node: Node<P, ChromeInfo>) -> Out {
    let tab = node.data.tab.clone();
    let mut layer: InAppLayerPath = node.parent.ascend();
    layer.get_mut().log.push_str(&tab);
    vec![]
}

impl<'a, M, P: Ascend<InAppLayerPath<'a>>> Descend<M> for Node<P, ChromeInfo> { .. }
```

The handler stops naming its parent and says what it needs instead: something that ascends to the layer. The derive then never needs the parent's type, and `ChromeInfo` hangs under any parent that reaches `InAppLayerPath`, which is the composition case in `derived-child-persistence.md`.

Not taken, for two reasons.

Every handler on a derived level becomes generic with an `Ascend` bound, and `node.parent.get_mut()` becomes `node.parent.ascend().get_mut()`. The simple case pays for the general one.

It only works while the parent chain is all `Path`. `Ascend` does not reach through a `Node` (`ascend-through-derived.md`), so `GmailInfo` under `ChromeInfo` cannot use it. The blanket would work at the first derived level and break at the second, which is a worse rule than "name your parent".

Both objections fall away under Fix B in `ascend-through-derived.md`, where `Path` becomes a case of `Node` and `Ascend` reaches through everything. That is the version in which `#[derived_node]` dies.

## Framework code

```rust
/// How a generated impl reaches the parent's type without naming it.
pub trait HasParent {
    type Parent;
    fn into_parent(self) -> Self::Parent;
}

impl<Parent, Data> HasParent for Node<Parent, Data> { /* Parent = Parent */ }
impl<N, P> HasParent for ::laserbeam::Path<N, P> { /* Parent = P */ }

/// ONE descent, whatever the child is. A place implements it by delegating to its own
/// `Dispatch` and then `into_parent()`. A derived level implements it directly.
pub trait Descend<M: Bindings>: HasParent + Sized {
    fn dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>;
}
```

`Descend` is what lets one generated line handle a child that is a field and a child that is a fn. The derive emits a `Descend` impl per place node, because a blanket `impl<N, P> Descend<M> for Path<N, P>` is E0311: `Dispatch` carries `Self: 'a`, and the HRTB needed to state it does not hold.

## What to build

`bind`: `Node`, `HasParent`, `Descend`.

`bind_macro`: hand every handler a `Node` (a place gets `data: ()`); `#[derived_child(f)]`; `#[derived_node(parent = ..)]`; a `Descend` impl per place node; and, when a derived child fn's `Data` is an enum, the per-variant destructure above.

`mercury`: delete `AppLayer`, the three empty structs, and the resync in `on_foregrounded`. Migrate every handler's signature.

Migration is mechanical: `path: NavPath` becomes `node: Node<NavPath, ()>`, and `path.get_mut()` becomes `node.parent.get_mut()`. In `bind`'s own test harness that was 14 lines across 8 handlers.

A handler bound at several places keeps its `Ascend` bound, which moves from the parameter to `parent`. Nothing in laserbeam changes.

```rust
fn to_home<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Out {
    go_home(&mut node.parent.ascend());
    vec![]
}
```

A DERIVED level's handler cannot ascend: its `parent` is a `Node`, and `Node` has no `Ascend` impl. `ascend-through-derived.md` records why (E0119 against laserbeam's reflexive impl) and what the two fixes cost. A derived level reaches ancestors through `parent` instead.

## The cost

The derived child fn runs on every dispatch, including foreground events, and clones its data each time. The derive cannot guard it: skipping the derived child fn needs to know whether anything below binds this event's source, and the derive knows neither the subtree's types nor the sources.

## `data` dies with the dispatch

At every level, through any number of projections into it.

A `#[resolve_into]` child of a derived level therefore compiles, gets a real `Path` and a real `get_mut()`, and writes into `data`, which the derived child fn built. That is allowed. Forbidding it would mean a subtree legal under a place is illegal under a derived level, so every node would have to know whether its ancestors are derived and nothing would compose.

`derived-child-persistence.md` records why the data does not persist, and why a constructor-on-enter, destructor-on-leave derived child fn is rejected.

## Out of scope, each with its own doc

```
accumulate-takes-a-path.md          accumulate has no path, so it cannot run a derived
                                    child fn, so a derived level's binds never reach the
                                    trigger set. It is the clobber check and should not
                                    ship. Independent of this; can land first.

option-resolve-into.md              #[resolve_into] chrome: Option<ChromeApp>, recognized
                                    syntactically the way Box already is. Absence in the
                                    derived half is already solved by Option<Data>.

derived-level-multiple-parents.md   One level under several parents, and one level shared
                                    between two hosts. Both are "this level does not know
                                    its parent". Blocked on Ascend.

ascend-through-derived.md           A derived level's handler cannot Ascend: E0119. Fix B
                                    makes Path a case of Node, which also deletes
                                    #[derived_node], HasParent, and probably Descend.

derived-child-iterator.md           Option<T> is an IntoIterator<Item = T>, so one
                                    signature covers zero, one, and many.

derived-child-persistence.md        Rejected: a derived child fn as constructor-on-enter
                                    and destructor-on-leave.

resolved-is-dead-weight.md          Resolve::Resolved and resolve() have no callers. Only
                                    Resolve::Path is used.
```
