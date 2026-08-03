# Multiple children

Downstream of `refactors/past/invalidation.md`, which has landed; the shapes below are the ones in laserbeam and bind today. The concrete target is figaro's typing layer: the russian-doll chain `TypingLayer<KinesisRemaps<NumberRemaps<SymbolRemaps>>>` becomes one flat struct with three child fields, and the doll pattern for typing is deleted. The other consumers are `ideas.md`'s two-keyboards case ("Two keyboards, two independent layers. ... Wants multiple `#[resolve_into]`") and the overlay that binds keys while open (`mercury-post-patterns.md`, which `invalidation-granularity.md` also leans on this doc for). The typing changes are gated on the compile-check the cap section names; the open questions at the bottom gate the root-branch consumers, not the flattening.

"Multiple children" means a place node with several child edges live at once, each an independently active subtree with its own leaf. Both kinds of edge participate:

- Real children: two or more `#[resolve_into]` fields.
- Derived children: several data fns in one attribute, `#[derived_children(app_data, app_foo)]`.
- Mixed: one place node carrying both.

The children are visited one at a time — dispatch still holds one `&mut` — in a documented order: field children first, in declaration order, then derived children, in listed order. That order is also claim order. Enum nodes stay single-child: a variant is exclusive by construction. A node with exactly one child edge keeps today's generated body unchanged, so the existing trees are untouched until a node grows a second child.

Derived levels themselves are unchanged in one respect: a derived level still cannot carry a `#[resolve_into]` field (its `data` dies with the dispatch; the existing rejection stands). A derived level may carry `#[derived_children]`, as it can today.

## What invalidation bought

The retired protocol broke a second branch on arrival. `Dispatch` returned `Option<Path>` and a claim early-returned `None`, so whenever branch 1 claimed, branch 2's subtree was skipped for that event: its binds never got a chance (acceptable, the claim is exclusive) and its posts never ran (not acceptable — branch 2's return-home deadline did not get pushed out, its timers leaked). There was no fix inside that protocol, because the early return was the protocol.

The landed protocol removes exactly this: `Dispatch::dispatch` returns `Completed<Self::Path<'a>>`, and the generated body is a linear fold ending in `MaybeInvalidated::complete(state)`. Three of its properties are the ones a second descent needs:

- Nothing early-returns. The body is a linear fold over `state`; a leave is data. A second descent is one more block in that fold, structurally identical to the first.
- Posts run whether or not anything claimed. Branch 2's schedule runs after branch 1 claimed, which is the property whose absence broke the old protocol.
- The claim already arbitrates any number of claimants: reborrowed per item, first take wins, everything else completes where it stands. Two branches need no new exclusivity mechanism.

So the fold shape is most of the way there. The gap is the leave protocol.

## The second descent, precisely

Mid-tree parent `P` at `PPath = PathMut<N, Q>`, children `C1`, `C2`. The generated body would want:

```rust
let mut state = C1::dispatch(c1_path, event, effs, claim).into_inner().to_maybe_invalidated();
// state: MaybeInvalidated<PPath>. C2's descent needs PPath back.
```

Three cases:

- `NotInvalidated(p)` — branch 1 stayed. Descend `C2` with `p`.
- `Invalidated(c)` with `c.into_inner() = Stop::Here(p)` — the leave stopped at `P` itself. `p` is recoverable; `C2` can still be descended.
- `Invalidated(c)` with `c.into_inner() = Stop::Up(above)` — the leave peeled past `P`. `PPath` went by value into `above` (a `Completed<Q>`, eventually the bare root). `C2`'s path cannot be built. Skipping the descent violates "every scheduled item runs": branch 2's whole schedule, all the way down, silently does not happen.

`completed-ancestors.md` landed this case split as `TryIntoAncestor`: `state.try_into_ancestor::<PPath>()` is `Ok` in the first two cases and `Err` in the third, so the recover is one call rather than a destructure — and the `Err` arm is still the same dead end.

A derived child's block is the same fold, one conversion shorter. `dispatch_into_tree_path` already returns its leave at the place beneath the derived chain — `Completed<TreePath>`, and at a place node `TreePath` is the place's own path — so the block folds `Completed<PPath>` directly through `Completed::to_maybe_invalidated`, with no `Stop` unwrap. The recover between blocks is the same `TryIntoAncestor` question with the same dead `Err` arm.

This is the "invalidates too high, you're SOL" case, and it is unrecoverable by construction: no fold conjures back a path that was peeled, and re-resolving from the root is exactly the blind re-projection the typed leave exists to prevent (the leave may have swapped an ancestor enum out from under the projection).

At the root the case does not exist. A child-of-root's `Up` payload is the bare `&'a mut R`, and a leave from the root never invalidates it — laserbeam states it twice, in `HasStop for &'a mut R` (always folds `NotInvalidated`) and in the distance-one `TryIntoAncestor<&'a mut R>` impl (`Ok` on both arms: the root is always alive) — so both arms hand the root back and the second descent is always buildable. Root-level multiple children — the two-keyboards case, and the overlay-with-keys case — need only the derive changes, no new laserbeam machinery.

There is also a semantic reading of the same fact. Invalidation is single-focus vocabulary: "the leave went above `P`" is a statement about the tree's one active path. With two live branches that statement is false on arrival — branch 2's path runs through `P` and nothing in branch 1 touched it. A leave from inside a branch that peels past the branch point is not a protocol gap to engineer around; it is a claim the type system should refuse to express.

## The cap: a branch point is a sub-root

Leaves from inside a branch terminate at the branch point. Not a runtime rule — a type: children of a multi-child node are rooted at something root-shaped, so `Above` and `CompletesTo` give their leaves nowhere higher to go, and the parent path provably survives every descent.

Two mechanisms can produce that shape.

Reborrowing is the cheap one: hand each child a path rooted at `&'b mut N` (the branch point's node, via `p.get_mut()`), so the child path is `PathMut<C1, &'b mut N>` and the existing root impls cap the leave for free; the parent path never leaves the frame, and each branch gets a fresh reborrow. For the flat typing layer as it stands it would even suffice — no doll handler today reads anything above typing. Its cost is that the wall is permanent: `HasAncestor`/`IntoAncestor` top out at `&'b mut N`, so no trigger closure or handler inside a branch can ever read the front app or `Figaro::held`, and the root-branch consumers (two keyboards, the overlay) do read the root. One mechanism serves every branch point, so it is the one that keeps ancestor reads open.

That version is a newtype that owns the parent path and mirrors the root's impl family:

```rust
/// A branch point's stand-in root: owns the parent path, so leaves from the
/// branch bottom out here and the path survives to seed the next branch.
pub struct SubRoot<P> {
    parent: P,
}

impl<P> Above for SubRoot<P> {
    type Up = SubRoot<P>; // mirrors `Above for &'a mut R`
}

impl<P> HasStop for SubRoot<P> {
    type Stop = SubRoot<P>; // mirrors `HasStop for &'a mut R`: a leave from a sub-root never invalidates it
    fn to_maybe_invalidated(completed: Completed<Self>) -> MaybeInvalidated<Self> {
        MaybeInvalidated::NotInvalidated(completed.into_inner())
    }
}

/// A child of a sub-root, unwrapped: mirrors the child-of-root `Stop` impl.
impl<N, P> Stop<PathMut<N, SubRoot<P>>, SubRoot<P>> {
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<SubRoot<P>> { .. }
}

/// Reads pass upward, so triggers and handlers inside a branch still see
/// ancestors. No `IntoAncestor` counterpart: a consuming walk stops at the
/// sub-root, which is the cap.
impl<T, P: HasAncestor<T>> HasAncestor<T> for SubRoot<P> {
    fn ancestor(&self) -> &T {
        self.parent.ancestor()
    }
}
```

Plus the `CompletesTo` peel-distance impls for a `SubRoot` terminal and the `TryIntoAncestor` terminal impls (a sub-root, like the root, is always alive), each mirroring the `&'a mut R` family; a `From<SubRoot<P>> for Completed<SubRoot<P>>`; inherent accessors on `SubRoot<PathMut<N, P>>` (`node_mut`/`node`, projecting through `parent.get_mut()`/`parent.get()`) for the derive's child projections to go through; and, in bind, the identity `HasTreePath for SubRoot<P>` (mirrors `HasTreePath for &mut R`), so a derived child hangs off a sub-root the way one hangs off the root. All additive, all in the style the root already uses. The `HasAncestor` forwarding does not overlap the identity impl for the same reason the depth impls do not: unifying them needs `P: HasAncestor<SubRoot<P>>`, a type containing itself, which the occurs check rejects. (These claims want the same compile-check treatment invalidation's types got before anything is committed to.)

Derived children cap identically. At a branch point the data fn receives the sub-rooted path (`fn(&SubRoot<PPath>) -> Option<Data>` — ancestor reads go through the forwarding impl), the level is built as `DerivedLevel { parent: sub, data }`, and `dispatch_into_tree_path` returns `Completed<SubRoot<PPath>>`, whose `into_inner` is the sub-root itself. The existing derived fns (`app_data`, `site_data` in both mercury and figaro) hang under single-child places and do not change.

## The generated body at a branch point

Mid-tree branch point with fields `b1: C1`, `b2: C2` and `#[derived_children(d1, d2)]`:

```rust
// opts: all snapped here, before ANY descent, source order — unchanged.

let sub = SubRoot::new(path);

// field children, declaration order
let c1 = PathMut::from_fn(sub, |s| &mut s.node_mut().b1, |s| &s.node().b1);
let sub = match C1::dispatch(c1, event, effs, claim).into_inner() {
    Stop::Here(child) => child.into_parent(),
    Stop::Up(sub) => sub, // the Up payload IS the sub-root: the cap, doing its job
};
let c2 = PathMut::from_fn(sub, |s| &mut s.node_mut().b2, |s| &s.node().b2);
let sub = match C2::dispatch(c2, event, effs, claim).into_inner() {
    Stop::Here(child) => child.into_parent(),
    Stop::Up(sub) => sub,
};

// derived children, listed order
let sub = match d1(&sub) {
    Some(data) => DispatchIntoTreePath::dispatch_into_tree_path(
        DerivedLevel { parent: sub, data },
        event,
        effs,
        claim,
    )
    .into_inner(), // HasStop for SubRoot: the Stop IS the sub-root
    None => sub,
};
let sub = match d2(&sub) { /* same shape */ };

let path = sub.into_inner();
let mut state = MaybeInvalidated::NotInvalidated(path); // children cannot invalidate a branch point
// scheduled items and MaybeInvalidated::complete(state) as today
```

The shape states the guarantee: after every descent the branch point's path is always in hand. `MaybeInvalidated` at a branch point is degenerate with respect to the descents — only the node's own earlier scheduled items can flip it, which is the existing fold. The branch point's own protocol upward is untouched; its own binds leave normally.

At the root the same blocks run over the bare `&'a mut R` with no `SubRoot`, through the impls that already exist. One field block:

```rust
let c1 = PathMut::from_fn(path, |r| &mut r.b1, |r| &r.b1);
let path = match C1::dispatch(c1, event, effs, claim).into_inner() {
    Stop::Here(child) => child.into_parent(),
    Stop::Up(root) => root, // the root is always alive
};
```

and a derived block's `dispatch_into_tree_path` returns `Completed<&'a mut R>`, whose `into_inner` is the root back.

Branch-internal machinery needs nothing new. A branch's return-home timer story lives at the branch's top node exactly as the `A → B` demo (`crates/bind/tests/schedule.rs`) puts it at `A`: `C1`'s own dispatch sees its child's leave as `MaybeInvalidated<C1Path>` and its posts run inside `C1::dispatch`. "Go home" inside a branch is a leave to the branch point, which is today's leave-to-root one level down.

## The typing layer, flat

The dolls exist purely to sequence claims: `KinesisRemaps` does not contain laptop number remaps in any meaningful sense, it just runs before them. Multiple children make the honest shape representable, and the three tables are disjoint by construction (each row is device- and key-scoped), so the field order is free; it reads in overlay order.

`src/model/typing/mod.rs`, before:

```rust
pub struct TypingLayer<Next> {
    pub jk: DeviceSequence,
    pub pending_double: Option<PendingDouble>,
    #[resolve_into]
    pub next: Next,
}

/// The typing stack as figaro composes it. The one place the composition is spelled.
pub type TypingStack = TypingLayer<KinesisRemaps<NumberRemaps<SymbolRemaps>>>;

impl TypingStack {
    pub(crate) fn new() -> Self {
        Self {
            jk: DeviceSequence::new(DeviceClass::Laptop, JK, JK_TIMEOUT),
            pending_double: None,
            next: KinesisRemaps::new(NumberRemaps::new(SymbolRemaps)),
        }
    }
}
```

after (the `#[bind]` tables on every node are unchanged throughout):

```rust
pub struct TypingLayer {
    pub jk: DeviceSequence,
    pub pending_double: Option<PendingDouble>,
    #[resolve_into]
    pub kinesis: KinesisRemaps,
    #[resolve_into]
    pub numbers: NumberRemaps,
    #[resolve_into]
    pub symbols: SymbolRemaps,
}

impl TypingLayer {
    pub(crate) fn new() -> Self {
        Self {
            jk: DeviceSequence::new(DeviceClass::Laptop, JK, JK_TIMEOUT),
            pending_double: None,
            kinesis: KinesisRemaps::new(),
            numbers: NumberRemaps,
            symbols: SymbolRemaps,
        }
    }
}
```

The children lose their parameters and their tails:

```rust
// kinesis.rs — was KinesisRemaps<Next> { shift_mirror, motion_hold, #[resolve_into] next: Next }
#[node(parent_path = TypingSubRoot)]
pub struct KinesisRemaps {
    pub shift_mirror: ShiftMirror,
    pub motion_hold: MotionHold,
}

// laptop.rs — was NumberRemaps<Next> { #[resolve_into] next: Next } and SymbolRemaps
#[node(parent_path = TypingSubRoot)]
pub struct NumberRemaps;

#[node(parent_path = TypingSubRoot)]
pub struct SymbolRemaps;
```

The path aliases, before:

```rust
pub type TypingPath<'a> = PathMut<TypingStack, LayerPath<'a>>;
pub type KinesisRemapsPath<'a> = PathMut<KinesisRemaps<NumberRemaps<SymbolRemaps>>, TypingPath<'a>>;
pub type NumberRemapsPath<'a> = PathMut<NumberRemaps<SymbolRemaps>, KinesisRemapsPath<'a>>;
```

after:

```rust
pub type TypingPath<'a> = PathMut<TypingLayer, LayerPath<'a>>;
pub type TypingSubRoot<'a> = SubRoot<TypingPath<'a>>;
pub type KinesisRemapsPath<'a> = PathMut<KinesisRemaps, TypingSubRoot<'a>>;
pub type NumberRemapsPath<'a> = PathMut<NumberRemaps, TypingSubRoot<'a>>;
pub type SymbolRemapsPath<'a> = PathMut<SymbolRemaps, TypingSubRoot<'a>>;
```

A kinesis row's ascent to typing state becomes a walk to the sub-root, which holds the typing path. `fn_key`, before:

```rust
type KPath<'a, Next> = PathMut<KinesisRemaps<Next>, TypingPath<'a>>;
type KDone<'a, Next> = (Vec<FigaroEffect>, Completed<KPath<'a, Next>>);

pub(crate) fn fn_key<'x, Next: 'static>(
    out: Out,
) -> impl Fn(&DeviceKey, (), KPath<'x, Next>) -> KDone<'x, Next> {
    move |ev, _snap, p| {
        let mut tp = p.into_parent();
        let mut effects = if ev.key.press == PressType::Down {
            flush_pending(tp.get_mut())
        } else {
            Vec::new()
        };
        effects.push(out.event(ev.key.press));
        (effects, tp.complete())
    }
}
```

after:

```rust
type KPath<'a> = KinesisRemapsPath<'a>;
type KDone<'a> = (Vec<FigaroEffect>, Completed<KPath<'a>>);

pub(crate) fn fn_key<'x>(out: Out) -> impl Fn(&DeviceKey, (), KPath<'x>) -> KDone<'x> {
    move |ev, _snap, p| {
        let mut sub = p.into_parent();
        let mut effects = if ev.key.press == PressType::Down {
            flush_pending(sub.node_mut())
        } else {
            Vec::new()
        };
        effects.push(out.event(ev.key.press));
        (effects, sub.complete())
    }
}
```

`sub.complete()` completes the leave that began at `KPath` with the focus at the sub-root, through the mirrored distance-one `CompletesTo` impl; the call site's `KDone` return type pins the origin, as everywhere else. Every other kinesis row (`kinesis`, `motion_hold_toggle`, `motion`, `kinesis_ending_hold`, `shift_mirror_toggle`, `double`) makes the same two-token change: `Next: 'static` deleted, `tp.get_mut()` → `sub.node_mut()`. `flush_pending<Next>(typing: &mut TypingLayer<Next>)` becomes `flush_pending(typing: &mut TypingLayer)`. The laptop handlers (`invert_shift`, `invert_command`) are generic over `P: HasStop + CompletesTo<P>` and do not change at all. Typing's own handlers (`pass_through`, `swallow_kinesis`, `jk_timeout`, `double_timeout`) drop `<Next>` and keep their paths; `pass_through`'s `into_ancestor::<FigaroPath>()` leave is the branch point's own bind and is untouched.

What this deletes, which is the point: `TypingStack`, the `<Next>` parameter on `TypingLayer`/`KinesisRemaps`/`NumberRemaps`, the `Next: 'static` bound on every kinesis row constructor and typing handler, the two-parameter `KPath`/`KDone`, and the nesting diagram. `Layer::Typing(TypingStack)` becomes `Layer::Typing(TypingLayer)` and `TypingStack::new()` call sites become `TypingLayer::new()`. The generic-doll pattern (`refactors/past/generic-doll-nodes.md`) stops being how typing composes; it remains for shells that genuinely wrap one child (`AlwaysOnRemaps<Layer>`, mercury's `AndReturnHome<Next>`).

## Changes, ordered

1. Prefactor, shippable now: rename `#[derived_child(f)]` to `#[derived_children(f)]`, parsing a comma-separated list of data-fn paths. `derived_child_fn` becomes `derived_children_fns`, returning `Vec<syn::Path>`; a repeated attribute is still an error; the emitted body still handles exactly one element (a second is rejected until change 3). Call sites: `crates/mercury/src/state/app.rs`, `crates/mercury/src/state/site.rs`, figaro's `src/model/app.rs` and `src/model/site.rs`, `crates/bind/tests/derived.rs`, and the compile-fail stderr text that names the attribute. Behavior-preserving.
2. laserbeam, additive: `SubRoot` and its impl family as written above, with unit tests in the style the root impls have; bind's `HasTreePath for SubRoot<P>` identity rides along.
3. bind_macro: `find_resolve_into` lifts to return every `#[resolve_into]` field in declaration order; a place node may carry both fields and `#[derived_children]`. A node with exactly one child edge emits today's body. A node with two or more emits the branch-point body: `SubRoot` wrapping mid-tree, bare-root blocks at the root, field blocks then derived blocks. Two fields of the same type are legal (the two-keyboards case): `HasPath` is per-type, so both children share one path type and one `parent_path` declaration, and only the projections distinguish them, which dispatch is fine with. The derived-level rejection of `#[resolve_into]` stays.
4. figaro: flatten the typing layer as written above.

## Open questions

1. Per-branch outcomes at the branch point. A post like the return-home cancel, generalized, keys on "did branch i leave", and that is visible only at the branch point. The pre cannot carry it (pres run before the descent), so it has to enter on the ascend side: either `AscendState` at a branch point carries the outcomes beside the state, or per-branch posts are a scheduled kind of their own. This changes the handler signature at branch points and needs its own design. The typing case does not need it (typing's posts key on its own state), so it does not gate changes 1–4.

2. Cross-branch triggers. Several children binding the same trigger resolves by claim order: earlier child's leaf first, the parent last. Nothing enforces disjointness — the check does not ship (`check` feature) and is slated for retirement — so the declared order is the documented rule, like source order for scheduled items. Typing's three tables are disjoint by device and key; the per-keyboard branches are disjoint because the event carries its device.

3. Cross-branch writes. A handler inside branch 1 cannot mutate branch 2 or anything above the branch point: `IntoAncestor` stops at the sub-root. That is the cap working, and typing never hits it (no doll handler reaches above typing today). Where it binds, "reset both keyboards" lives on the branch point (which holds both fields) or arrives as a follow-up event, and a handler bounded `P: IntoAncestor<FigaroPath<'a>>` will not bind inside a branch — the bound is unsatisfiable there, so the restriction surfaces at compile time rather than at dispatch.

4. Reads above the cap take a hop. The shape impls give a branch path `HasAncestor<SubRoot<PPath>>`; the forwarding impl continues from there, so reaching the root reads as `path.ancestor::<TypingSubRoot>().ancestor::<FigaroPath>()`. No typing child reads an ancestor today, so no call site pays it in change 4; collapsing to one hop needs a composing impl family with its own coherence argument, deferred until a call site wants it.

5. The single active leaf, for the root-branch consumers only. `set_layer`, the menu bar title, and the overlay all speak of "the" layer; two keyboards mean two leaves, and which one the title shows is mercury's (or figaro's) question to answer before that case ships. The flat typing layer does not touch it: the `Layer` enum sits above the branch point and stays single.

6. The derive's other edges under a branch: a routed (multi-parent) child (`#[resolve_into(route = .., up = ..)]`, whose fold matches the consumer's `Up` enum) and a generic child (the `for<'q>` path-equality bound from `generic-doll-nodes.md`) at a branch point are unexamined. Neither occurs in the typing case: the flattening removes typing's generic children rather than adding any.
