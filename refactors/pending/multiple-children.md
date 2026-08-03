# Multiple children

Downstream of `refactors/past/invalidation.md`, which has landed; the shapes below are the ones in laserbeam and bind today, and the one laserbeam addition (`descend`, change 2) is a method over machinery that landed with `completed-ancestors.md`. The concrete target is figaro's typing layer: the russian-doll chain `TypingLayer<KinesisRemaps<NumberRemaps<SymbolRemaps>>>` becomes one flat struct with three child fields, and the doll pattern for typing is deleted. The other consumers are `ideas.md`'s two-keyboards case ("Two keyboards, two independent layers. ... Wants multiple `#[resolve_into]`") and the overlay that binds keys while open (`mercury-post-patterns.md`, which `invalidation-granularity.md` also leans on this doc for); both are root branches, and only they are gated by the open questions at the bottom. Change 1 is a shippable prefactor; the typing flattening waits only on changes 2 and 3.

"Multiple children" means a place node with several child edges live at once, each an independently active subtree with its own leaf. Both kinds of edge participate:

- Real children: two or more `#[child]` fields.
- Derived children: several data fns in one attribute, `#[derived_children(app_data, app_foo)]` (today's `#[derived_child]`, renamed by change 1).
- Mixed: one place node carrying both.

The children are visited one at a time — dispatch still holds one `&mut` — in a documented order: field children first, in declaration order, then derived children, in listed order. That order is also claim order. Several children binding the same trigger is expected, not an error: the claim arbitrates the bind rows exactly as it does within one node today (first take wins, the rest complete where they stand), and a row meant to run beside the claimant — a logger — is a post, which ignores the claim. Enum nodes stay single-child: a variant is exclusive by construction. Every place node emits one body shape: the state initialized standing, then one `descend` block per child edge. A leaf is the zero-block case, which is today's leaf body already; a single-child node's emitted code changes shape but not behavior, so the existing trees behave exactly as they do today.

Derived levels themselves are unchanged in one respect: a derived level still cannot carry a `#[child]` field (its `data` dies with the dispatch; the existing rejection stands). A derived level may carry `#[derived_children]`, as it can today.

## What invalidation bought

The retired protocol broke a second branch on arrival. `Dispatch` returned `Option<Path>` and a claim early-returned `None`, so whenever branch 1 claimed, branch 2's subtree was skipped for that event: its binds never got a chance (acceptable, the claim is exclusive) and its posts never ran (not acceptable — branch 2's return-home deadline did not get pushed out, its timers leaked). There was no fix inside that protocol, because the early return was the protocol.

The landed protocol removes exactly this: `Dispatch::dispatch` returns `Completed<Self::Path<'a>>`, and the generated body is a linear fold ending in `MaybeInvalidated::complete(state)`. Three of its properties are the ones a second descent needs:

- Nothing early-returns. The body is a linear fold over `state`; a leave is data. A second descent is one more block in that fold, structurally identical to the first.
- Posts run whether or not anything claimed. Branch 2's schedule runs after branch 1 claimed, which is the property whose absence broke the old protocol. A claim and a leave are different things: a claim never skips a sibling.
- The claim already arbitrates any number of claimants: reborrowed per item, first take wins, everything else completes where it stands. Two branches need no new exclusivity mechanism.

So every piece is landed; what remains is the derive.

## The second descent, precisely

Mid-tree parent `P` at `PPath = PathMut<N, Q>`, children `C1`, `C2`. The generated body wants:

```rust
let mut state = C1::dispatch(c1_path, event, effs, claim).into_inner().to_maybe_invalidated();
// state: MaybeInvalidated<PPath>. C2's descent needs PPath back.
```

Three cases:

- `NotInvalidated(p)` — branch 1 stayed. Descend `C2` with `p`.
- `Invalidated(c)` with `c.into_inner() = Stop::Here(p)` — the leave stopped at `P` itself. `p` is recoverable; `C2` is still descended. This is what today's kinesis rows produce: they ascend to typing and complete there, so under the flat layer they leave their own branch and the laptop branches still run.
- `Invalidated(c)` with `c.into_inner() = Stop::Up(above)` — the leave peeled past `P`. `PPath` went by value into `above` (a `Completed<Q>`, eventually the bare root), and `C2`'s path cannot be built. The remaining siblings are skipped for this event, and the leave forwards upward exactly as a single-child node forwards it today.

`completed-ancestors.md` landed this case split as `TryIntoAncestor`: `state.try_into_ancestor::<PPath>()` is `Ok` in the first two cases and `Err` in the third, so each sibling block is a recover followed by a descent, and the `Err` arm is the skip.

The recover must not erase what it recovered from. Not because of rival bind rows — several children binding the same key is expected, and the claim already arbitrates it (`#[bind]` desugars through `exclusive`, so a row that loses the claim completes where it stands; a row that should run beside the claimant, a logger, is a post, and posts ignore the claim). What observes the invalidation is the other half of the node's own schedule: posts match on stayed-versus-left — the return-home deadline post pushes the deadline out over a standing path and cancels over a leave, so erasing a stopped-here leave would push a deadline the leave should have cancelled — and a leave produced by a post takes no claim at all, so a bind row at the branch point would otherwise win the claim and run on post-leave state with its pre-descent snap. So the fold keeps the join: the path is lent to each remaining sibling, and the node's final state is invalidated iff any branch's leave reached it — restored by `complete()` when the sibling stays (a zero-peel complete is `Stop::Here`), replaced when a later sibling's leave goes higher. That join is one small laserbeam method over what already exists:

```rust
impl<P: HasStop + CompletesTo<P>> MaybeInvalidated<P>
where
    Completed<P>: TryIntoAncestor<P>,
{
    /// Descend the next child subtree with this node's path, if the path can
    /// still be built: lent from a standing state, or recovered from a leave that
    /// stopped exactly here — in which case the node stays invalidated whatever
    /// the child does. A leave that went above skips the descent and forwards.
    pub fn descend(self, f: impl FnOnce(P) -> Self) -> Self {
        match self {
            Self::NotInvalidated(p) => f(p),
            Self::Invalidated(completed) => match completed.try_into_ancestor() {
                Ok(p) => Self::Invalidated(f(p).complete()),
                Err(completed) => Self::Invalidated(completed),
            },
        }
    }
}
```


The skip is semantically honest, not a compromise. A leave that peels past the branch point is leaving the region that contains every sibling, not just the branch it came from: the handler that produced it is replacing an ancestor (`set_layer` swapping the `Layer` variant), and the siblings' subtrees are dropped with the region, their `TimerGuard`s cancelling on drop — the same contract every leave already has with the subtree it exits. So "every scheduled item runs" sharpens to "every scheduled item on a surviving path runs", which is what it always meant: a dropped subtree has nothing left to schedule for. The one edge to know: a handler that consumes to an ancestor and completes there without changing state leaves the siblings standing but unvisited for that event. That is accepted.

A derived child's block is the same fold, one conversion shorter. `dispatch_into_tree_path` already returns its leave at the place beneath the derived chain — `Completed<TreePath>`, and at a place node `TreePath` is the place's own path — so the block folds `Completed<PPath>` directly through `Completed::to_maybe_invalidated`, with no `Stop` unwrap.

At the root the skip case does not exist. A child-of-root's `Up` payload is the bare `&'a mut R`, and a leave from the root never invalidates it — `HasStop for &'a mut R` always folds `NotInvalidated`, and the distance-one `TryIntoAncestor<&'a mut R>` impl is `Ok` on both arms (the root is always alive) — so the recover is total and every sibling always runs. The body shape is the same either way; the root is the case where `Err` is uninhabited in practice.

## The generated body at a branch point

Mid-tree branch point with fields `b1: C1`, `b2: C2` and `#[derived_children(d1, d2)]`. One body shape at every arity: the state starts standing — a leaf's whole body today — and every child edge is one `descend` block, the first included (its match always takes the standing arm):

```rust
// opts: all snapped here, before ANY descent, source order — unchanged.

let mut state = MaybeInvalidated::NotInvalidated(path); // today's leaf body: the zero-block case

// field children, declaration order
state = state.descend(|p| {
    let c1 = PathMut::from_fn(p, |p| &mut p.get_mut().b1, |p| &p.get().b1);
    C1::dispatch(c1, event, effs, claim).into_inner().to_maybe_invalidated()
});
state = state.descend(|p| {
    let c2 = PathMut::from_fn(p, |p| &mut p.get_mut().b2, |p| &p.get().b2);
    C2::dispatch(c2, event, effs, claim).into_inner().to_maybe_invalidated()
});

// derived children, listed order
state = state.descend(|p| match d1(&p) {
    Some(data) => DispatchIntoTreePath::dispatch_into_tree_path(
        DerivedLevel { parent: p, data },
        event,
        effs,
        claim,
    )
    .to_maybe_invalidated(),
    None => MaybeInvalidated::NotInvalidated(p),
});
state = state.descend(|p| match d2(&p) {
    Some(data) => DispatchIntoTreePath::dispatch_into_tree_path(
        DerivedLevel { parent: p, data },
        event,
        effs,
        claim,
    )
    .to_maybe_invalidated(),
    None => MaybeInvalidated::NotInvalidated(p),
});

// scheduled items and MaybeInvalidated::complete(state) as today
```

At the root the blocks are identical with the root's projections (`|r| &mut r.b1`) and `&'a mut R` in `PPath`'s position; the recover is total there, per the impls above. The branch point's own protocol upward is untouched: its own scheduled items run over the final `state`, and its own binds leave normally.

Branch-internal machinery needs nothing new. A branch's return-home timer story lives at the branch's top node exactly as the `A → B` demo (`crates/bind/tests/schedule.rs`) puts it at `A`: the branch's own dispatch sees its child's leave as its own `MaybeInvalidated` and its posts run inside its own `dispatch`. An ascent that completes at the branch point is the second case above — the next sibling still runs.

## The typing layer, flat

The dolls exist purely to sequence claims: `KinesisRemaps` does not contain laptop number remaps in any meaningful sense, it just runs before them. Multiple children make the honest shape representable, and the three tables are disjoint by construction (each row is device- and key-scoped), so the field order is free; it reads in overlay order.

`src/model/typing/mod.rs`, before:

```rust
pub struct TypingLayer<Next> {
    pub jk: DeviceSequence,
    pub pending_double: Option<PendingDouble>,
    #[child]
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
    #[child]
    pub kinesis: KinesisRemaps,
    #[child]
    pub numbers: NumberRemaps,
    #[child]
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

The children lose their parameters and their tails, and all three parent themselves on `TypingPath`:

```rust
// kinesis.rs — was KinesisRemaps<Next> { shift_mirror, motion_hold, #[child] next: Next }
#[node(parent_path = TypingPath)]
pub struct KinesisRemaps {
    pub shift_mirror: ShiftMirror,
    pub motion_hold: MotionHold,
}

// laptop.rs — was NumberRemaps<Next> { #[child] next: Next } and SymbolRemaps
#[node(parent_path = TypingPath)]
pub struct NumberRemaps;

#[node(parent_path = TypingPath)]
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
pub type KinesisRemapsPath<'a> = PathMut<KinesisRemaps, TypingPath<'a>>;
pub type NumberRemapsPath<'a> = PathMut<NumberRemaps, TypingPath<'a>>;
pub type SymbolRemapsPath<'a> = PathMut<SymbolRemaps, TypingPath<'a>>;
```

The handler bodies do not change at all; only the doll generics leave the signatures. `fn_key`, before:

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

`tp.complete()` still completes the leave that began at `KPath` with the focus at `TypingPath`, through the same distance-one `CompletesTo` impl it uses today — under the flat layer that leave is the stopped-at-the-branch-point case, and the laptop branches still run after it. Every other kinesis row (`kinesis`, `motion_hold_toggle`, `motion`, `kinesis_ending_hold`, `shift_mirror_toggle`, `double`) drops `Next: 'static` the same way and keeps its body. `flush_pending<Next>(typing: &mut TypingLayer<Next>)` becomes `flush_pending(typing: &mut TypingLayer)`. The laptop handlers (`invert_shift`, `invert_command`) are generic over `P: HasStop + CompletesTo<P>` and do not change at all. Typing's own handlers (`pass_through`, `swallow_kinesis`, `jk_timeout`, `double_timeout`) drop `<Next>` and keep their paths; `pass_through`'s `into_ancestor::<FigaroPath>()` leave is the branch point's own bind and is untouched.

What this deletes, which is the point: `TypingStack`, the `<Next>` parameter on `TypingLayer`/`KinesisRemaps`/`NumberRemaps`, the `Next: 'static` bound on every kinesis row constructor and typing handler, the two-parameter `KPath`/`KDone`, and the nesting diagram. `Layer::Typing(TypingStack)` becomes `Layer::Typing(TypingLayer)` and `TypingStack::new()` call sites become `TypingLayer::new()`. The generic-doll pattern (`refactors/past/generic-doll-nodes.md`) stops being how typing composes; it remains for shells that genuinely wrap one child (`AlwaysOnRemaps<Layer>`, mercury's `AndReturnHome<Next>`).

## Changes, ordered

1. Prefactor, shippable now: rename `#[derived_child(f)]` to `#[derived_children(f)]`, parsing a comma-separated list of data-fn paths. `derived_child_fn` becomes `derived_children_fns`, returning `Vec<syn::Path>`; a repeated attribute is still an error; the emitted body still handles exactly one element (a second is rejected until change 3). Call sites: `crates/mercury/src/state/app.rs`, `crates/mercury/src/state/site.rs`, figaro's `src/model/app.rs` and `src/model/site.rs`, `crates/bind/tests/derived.rs`, and the compile-fail stderr text that names the attribute. Behavior-preserving.
2. laserbeam: `MaybeInvalidated::descend` as written above, with unit tests driving all three arms (standing, stopped-here with a staying and a leaving sibling, gone-above). Additive; everything it touches (`TryIntoAncestor`, the identity `IntoAncestor`, `MaybeInvalidated::complete`) is landed.
3. bind_macro: `find_child`'s at-most-one restriction lifts (every `#[child]` field, declaration order); a place node may carry both fields and `#[derived_children]`. Every place node emits the uniform body above — the standing init plus one `descend` block per child edge, field blocks then derived blocks; a leaf's emission is unchanged (the init is today's leaf body), and a single-child node's emission changes shape, not behavior. Two fields of the same type are legal (the two-keyboards case): `HasPath` is per-type, so both children share one path type and one `parent_path` declaration, and only the projections distinguish them, which dispatch is fine with. A routed or generic child at a multi-child node is rejected with a derive error, mirroring `reject_routed_generic`, until open question 2 is designed. The derived-level rejection of `#[child]` stays.
4. figaro: flatten the typing layer as written above.

## Open questions

1. Per-branch outcomes at the branch point. A post like the return-home cancel, generalized, keys on "did branch i leave", and that is visible only at the branch point. The join keeps whether any branch's leave reached the node, but not which branch, and the pre cannot carry it (pres run before the descent), so it has to enter on the ascend side: either `AscendState` at a branch point carries the outcomes beside the state, or per-branch posts are a scheduled kind of their own. This changes the handler signature at branch points and needs its own design. The typing case does not need it (typing's posts key on its own state), so it does not gate changes 1–4.

2. The derive's other edges under a branch: a routed (multi-parent) child (`#[child(route = .., up = ..)]`, whose fold matches the consumer's `Up` enum) and a generic child (the `for<'q>` path-equality bound from `generic-doll-nodes.md`) at a branch point are unexamined. Neither occurs in the typing case: the flattening removes typing's generic children rather than adding any.
