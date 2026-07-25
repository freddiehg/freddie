# Multiple children

Thoughts on feasibility, not a plan. Downstream of `invalidation.md`, which lands first and whose shapes this assumes. Background: `derived-child-iterator.md` records the derived-fn arity generalization; `ideas.md` names the motivating case ("Two keyboards, two independent layers. ... Wants multiple `#[resolve_into]`").

"Multiple children" means a struct node with two `#[resolve_into]` fields live at once, each an independently active subtree with its own leaf:

```rust
#[derive(Bind)]
#[node(root)]
#[binds(MercuryStruct)]
struct Mercury {
    #[resolve_into]
    internal: KeyboardLayers, // the laptop keyboard's layer machine
    #[resolve_into]
    external: KeyboardLayers, // the external keyboard's
}
```

The tree stops having one active path and has one per branch. Dispatch still holds one `&mut`, so the branches are visited one at a time, in field order, each descent consuming the parent and handing it back. Enum nodes stay single-child: a variant is exclusive by construction.

## What invalidation already buys

The pre-invalidation protocol breaks a second branch on arrival. `Dispatch` returns `Option<Path>` and a claim early-returns `None`, so whenever branch 1 claims, branch 2's subtree is skipped for that event: its binds never get a chance (acceptable, the claim is exclusive) and its posts never run (not acceptable — branch 2's return-home deadline does not get pushed out, its timers leak). There is no fix inside that protocol, because the early return is the protocol.

Invalidation removes exactly this. Three of its properties are the ones a second descent needs:

- Nothing early-returns. The body is a linear fold over `state`; a leave is data. A second descent is one more block in that fold, structurally identical to the first.
- Posts run whether or not anything claimed. Branch 2's schedule runs after branch 1 claimed, which is the property whose absence breaks the old protocol.
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
- `Invalidated(c)` with `c.into_inner() = Stop::Here(p)` — the leave stopped at `P` itself. `p` is recoverable by destructuring; `C2` can still be descended.
- `Invalidated(c)` with `c.into_inner() = Stop::Up(above)` — the leave peeled past `P`. `PPath` went by value into `above` (a `Completed<Q>`, eventually the bare root). `C2`'s path cannot be built. Skipping the descent violates "every scheduled item runs": branch 2's whole schedule, all the way down, silently does not happen.

This is the "invalidates too high, you're SOL" case, and it is unrecoverable by construction: no fold conjures back a path that was peeled, and re-resolving from the root is exactly the blind re-projection the typed leave exists to prevent (the leave may have swapped an ancestor enum out from under the projection).

At the root the case does not exist. A child-of-root's `Up` payload is the bare `&'a mut R`, and a leave from the root never invalidates it (`HasStop for &'a mut R` always folds `NotInvalidated`), so both arms hand the root back and the second descent is always buildable. Root-level multiple children — the two-keyboards case — are mechanically fine under `invalidation.md` as written, needing only the derive to allow two fields and emit two descent blocks.

There is also a semantic reading of the same fact. Invalidation is single-focus vocabulary: "the leave went above `P`" is a statement about the tree's one active path. With two live branches that statement is false on arrival — branch 2's path runs through `P` and nothing in branch 1 touched it. A leave from inside a branch that peels past the branch point is not a protocol gap to engineer around; it is a claim the type system should refuse to express.

## The cap: a branch point is a sub-root

Leaves from inside a branch terminate at the branch point. Not a runtime rule — a type: children of a multi-child node are rooted at something root-shaped, so `Above` and `Complete` give their leaves nowhere higher to go, and the parent path provably survives both descents.

Two mechanisms can produce that shape.

Reborrowing is the cheap one: hand each child a path rooted at `&'b mut N` (the branch point's node, via `p.get_mut()`), so the child path is `PathMut<C1, &'b mut N>` and the existing root impls cap the leave for free; the parent path never leaves the frame, and each branch gets a fresh reborrow. Its cost kills it: `HasAncestor`/`IntoAncestor` top out at `&'b mut N`, so trigger closures and handlers inside the branch cannot read anything above the branch point — not the front app, not the root — and mercury's handlers read the root constantly.

The version that keeps ancestor reads is a newtype that owns the parent path and mirrors the root's impl family:

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

Plus the `Complete` peel-distance family for a `SubRoot` terminal, mirroring the `&'a mut R` family; a `From<SubRoot<P>> for Completed<SubRoot<P>>`; and inherent accessors on `SubRoot<PathMut<N, P>>` (`node_mut`/`node`, projecting through `parent.get_mut()`/`parent.get()`) for the derive's child projections to go through. All additive, all in the style the root already uses. The `HasAncestor` forwarding does not overlap the identity impl for the same reason the depth impls do not: unifying them needs `P: HasAncestor<SubRoot<P>>`, a type containing itself, which the occurs check rejects. (These claims want the same compile-check treatment `invalidation.md`'s types got before anything is committed to.)

The generated body at a mid-tree branch point:

```rust
// opts: all snapped here, before ANY descent, source order — unchanged.

let sub = SubRoot::new(path);
let c1 = PathMut::from_fn(sub, |s| &mut s.node_mut().b1, |s| &s.node().b1);
let (sub, b1_left) = match C1::dispatch(c1, event, effs, claim).into_inner() {
    Stop::Here(child) => (child.into_parent(), false),
    Stop::Up(sub) => (sub, true), // the Up payload IS the sub-root: the cap, doing its job
};
let c2 = PathMut::from_fn(sub, |s| &mut s.node_mut().b2, |s| &s.node().b2);
let (sub, b2_left) = match C2::dispatch(c2, event, effs, claim).into_inner() { .. };
let path = sub.into_inner();

let mut state = MaybeInvalidated::NotInvalidated(path); // children cannot invalidate a branch point
// scheduled items and state.complete() as today
```

The shape states the guarantee: after both descents the branch point's path is always in hand. `MaybeInvalidated` at a branch point is degenerate with respect to the descent — only the node's own earlier scheduled items can flip it, which is the existing fold. The branch point's own protocol upward is untouched; its own binds leave normally.

Branch-internal machinery needs nothing new. A branch's return-home timer story lives at the branch's top node exactly as the `A → B` demo puts it at `A`: `C1`'s own dispatch sees its child's leave as `MaybeInvalidated<C1Path>` and its posts run inside `C1::dispatch`. "Go home" inside a branch is a leave to the branch point, which is today's leave-to-root one level down.

## Open questions

1. Per-branch outcomes at the branch point. A post like the return-home cancel, generalized, keys on "did branch i leave", and that is visible only at the branch point. The pre cannot carry it (pres run before the descent), so it has to enter on the ascend side: either `AscendState` at a branch point carries the outcomes beside the state, or per-branch posts are a scheduled kind of their own. This changes the handler signature at branch points and needs its own design.

2. Cross-branch triggers. Both branches binding the same trigger resolves by claim order: branch 1's leaf beats branch 2's leaf beats the parent, in field order. For per-keyboard branches the triggers are disjoint by construction (the event carries its device), but nothing enforces disjointness and the check is being retired, so field order would be the documented rule, like source order for scheduled items.

3. Cross-branch writes. A handler inside branch 1 cannot mutate branch 2 or the root: `IntoAncestor` stops at the sub-root. That is the cap working, but it moves some bindings: "reset both keyboards" lives on the branch point (which holds both fields), or arrives as a follow-up event. Likewise a handler bounded `P: IntoAncestor<MercuryPath<'a>>` will not bind inside a branch — the bound is unsatisfiable there, so the restriction surfaces at compile time rather than at dispatch.

4. Reads above the cap take a hop. The shape impls give a branch path `HasAncestor<SubRoot<PPath>>`; the forwarding impl continues from there, so reaching the root reads as `path.ancestor::<SubRoot<PPath>>().ancestor::<MercuryPath>()`. Collapsing that to one hop needs a composing impl family with its own coherence argument; whether the two-hop spelling is acceptable is a judgment call for the design.

5. The single active leaf. `set_layer`, the menu bar title, and the overlay all speak of "the" layer. Two branches have two leaves; which one the title shows, and whether the overlay is per-device, is mercury's question rather than laserbeam's, but it has to be answered before the two-keyboards case ships.

6. The derive. `find_resolve_into` errors on a second `#[resolve_into]` field today; lifting that, emitting one descent block per field, and handling two fields of the same type (both paths share a type; only the projection distinguishes them — `Place` and the parent alias are per-type and identical for both, which dispatch is fine with) is macro work. Derived-child edges under a branch, and a branch point that is itself a derived level, are unexamined.

## Answer

- At the root: yes, `invalidation.md` gets us essentially all the way. A root child's leave always hands the root back, so two root descents compose under the landed protocol once the derive allows them. The two-keyboards case is a root branch, so the motivating case is in this half.
- Mid-tree: the SOL diagnosis is correct and unrecoverable by design — the path goes by value into the leave, and no fold gets it back. The resolution is the cap, not recovery: `SubRoot` makes a past-the-branch-point leave unrepresentable from inside a branch, in additive laserbeam machinery mirroring the root's existing impl family.
- Nothing in `invalidation.md` needs to change to keep this door open. The linear body, leaves-as-data, and posts-run-regardless are exactly the prerequisites; the branch-point body replaces one `#state` expression with a per-child fold and layers on after.
