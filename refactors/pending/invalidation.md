# Invalidation: posts react to what the ascent reshaped

Not done. The model: the descent SCHEDULES and the ascent EXECUTES. On the way down, `pre`s read (and at most carry a value); nothing reshapes the tree. On the way up, `post`s run leaf-to-root, each handed an `Invalidated` telling it what the ascent so far reshaped, and each mutates only where that's still sound. The exclusive winner is just the post that reshapes; every other post finds out and reacts. This refines `handler-kinds.md` (its `Nested` becomes `Invalidated`) and `exclusive-as-post.md` (the winner is the reshaping post).

## The invariant: descent schedules, ascent executes

- Descent: `pre`s run, READ-ONLY. They may return a value to their post; they never reshape an ancestor. This fixes the SET of handlers that will run.
- Ascent: `post`s run leaf-to-root. Each gets `(its pre's value, Invalidated)`. A post mutates only where sound (see `Invalidated`). The winner is the post that reshapes the root (`set_layer`), which raises the flag for the posts above it.
- Invalidation happens ONLY on the ascent, so it can never change the schedule fixed on the descent. That is what rules out the change-the-layer-back-and-forth recompute loop: a reshape can't spawn or drop a handler, because the handler set was frozen before any reshape could happen.

## `Invalidated`

```rust
#[derive(Clone, Copy, PartialEq)]
enum Invalidated {
    Valid,           // nothing at or below me reshaped
    SubtreeChanged,  // a descendant reshaped; MY node survives
    SelfReplaced,    // my node was inside the reshaped subtree; I am gone
}
```

Two properties do the work:

- It is a FROM-BELOW signal. You learn about reshapes deeper than you, because those handlers ran before you on the way up, so their damage is visible when your turn comes. A reshape ABOVE you runs AFTER you and cannot touch effects you already returned (they are owned values), so you never need to hear about it — and couldn't, since it hasn't happened at your turn.
- It is per-node. `SubtreeChanged` means your own node is intact and you may still mutate it; only a child moved. `SelfReplaced` means your node was replaced — the `&mut Node` you were handed is stale, so you must NOT touch it. You may still emit from your carried value, or nothing.

## `pre` is carriage, not detection

The ONLY reason for a `pre` is to carry a value that was readable on the valid descent into a post effect, for the case where the ascent might reshape the source out from under the post. Detection — "did X change" — is `invalidated`'s job; a pre-snapshot-and-compare is a leaky hand-rolled version that can't see a reshape (the root can drop the whole branch and the compared value never told you).

No mercury post today needs carriage, so `pre` stays unbuilt. Add it the day a post says "I need what X WAS on the way in," never for "did X change."

## The rearm is a lone post

The return-home timer arms on activity and cancels when you leave the set of layers it guards. It was a `#[pre_post]` that threaded a minted timer from `arm` to `stay`; that was the side effect (minting) sitting in the wrong place. Mint in the post, gated by validity, and it collapses to one handler:

```rust
#[post(AnyKey => rearm)]
fn rearm(_ev: &KeyEvent, node: &mut Node<AndReturnHome>, inv: Invalidated) -> Vec<MercuryEffect> {
    if inv == Invalidated::SelfReplaced {
        return vec![];   // navigated out of the set: AndReturnHome is gone; the dropped node's
                         // guard cancels the old timer, and there is nothing to arm.
    }
    let (guard, schedule) = arm_return_home();   // mint HERE, on the valid ascent, only when it should happen
    node.get_mut().guard = guard;                // install: replaces the old guard -> cancels the old timer
    vec![schedule]
}
```

- `SelfReplaced` (nav to home, out of the set): do nothing; the drop cancels. No wasted arm — the arm never happens, because the knowledge that we left arrives before the mint.
- `Valid` (activity, no nav) or `SubtreeChanged` (nav within the set): AndReturnHome survives; rearm.
- The mint is a side effect on the valid ascent, run only when correct. The only mutation is on the node's OWN `guard` — self, not ancestor. No `arm`, no threaded value, no pending timer.

## The overlay is the proof of the minimal model

An `OverlayLayer` wraps the whole layer-enum and hides the overlay when the layer below it changed. One post on the outside, not a hide-the-overlay line copied into every navigation handler:

```rust
#[post(AnyKey => hide_on_change)]
fn hide_on_change(_ev: &KeyEvent, _node: &mut Node<OverlayLayer>, inv: Invalidated) -> Vec<MercuryEffect> {
    match inv {
        Invalidated::SubtreeChanged => vec![hide_overlay()],   // the layer below swapped
        Invalidated::Valid | Invalidated::SelfReplaced => vec![],
    }
}
```

Two things make this the case that earns the model, and only the minimal model:

- It is DRY and correct: "hide on any change" is stated once and can't be forgotten on the fifth layer someone adds. The overlay logic lives with the overlay.
- It EMITS an effect; it does not reach up and mutate an ancestor. So it needs `post` + `Invalidated` and nothing else — no owned path, no re-derivation, no scheduler. One bit in, a `Vec<Effect>` out.

`OverlayLayer` wraps everything, so it survives every nav and only ever sees `Valid`/`SubtreeChanged`. `AndReturnHome` wraps a SUBSET, so leaving the subset replaces it — that is the case that produces `SelfReplaced`. The two together exercise all three enum states; each alone would justify only a bool. Full level-granularity ("reshaped up to depth N") is a deferred optimization neither needs.

## What this does NOT justify: the scheduler

Every handler above either RETURNS effects (overlay) or mutates its OWN node (rearm's `guard`). None reaches up and MUTATES an ancestor.

The re-derivable-path scheduler is needed only when MULTIPLE handlers each mutate an ancestor in one dispatch — the `A { overlay, layer }` shape where an additive handler writes `A.overlay` while an exclusive writes `A.layer`. Under single-owner, that's impossible: reaching the root mutably consumes the path, and the path consumes once, so owned ⟺ exclusive. The scheduler is the only thing that breaks that equivalence, and it costs the borrow checker's veto on stale ancestor access — re-derivation makes a stale reach memory-safe but silently wrong, trading a compile error for a scheduling discipline. No mercury case needs owned-additive yet, so the scheduler stays unbuilt. Prove the need first: a non-winner that MUST mutate root state that genuinely can't be relocated to its own node.

## Effects survive invalidation

A post that runs before a shallower reshape keeps its effects. If `root(a) -> layer -> NavLayer(b)` and `b` returns `fx` and then `a` swaps the layer, `fx` stays: it is an owned `Vec<Effect>`, and `a`'s later swap can't reach into a copy `b` already handed off. This is the from-below property from the other side — you keep what you produced before a reshape you couldn't have seen, because it can't corrupt owned values.

## Refinements to the other docs

- `handler-kinds.md`: `Nested` (Handled/Missed) becomes `Invalidated` (Valid/SubtreeChanged/SelfReplaced) on the post's signature; `pre` becomes read-only carriage, or absent.
- `exclusive-as-post.md`: "a post that checks `Nested`" becomes "the winner is the post that RESHAPES, and every other post reads `Invalidated`." The winner raises the flag by reshaping, rather than by setting a separate `Nested`.

## Open

- The enum is `Valid | SubtreeChanged | SelfReplaced`. Full level-granular ("reshaped up to depth N") is deferred until a case needs precision the three states can't give.
- Whether the framework CALLS a post whose node is `SelfReplaced` (always-called-with-`Invalidated`, so the rearm can decide) or SKIPS it. Always-called is assumed here — it's what lets `SelfReplaced` be a decision rather than a silent no-run.
- `pre`'s exact signature if a carriage case appears: `fn(&Event, &Node) -> T`, shared ref, read-only.
- Multiple children: an enum (one active child) works today; a product node (several live children) extends the semantics cleanly but needs a JOIN in `into_parent` (the parent's post can't run until every child has ascended), which is orthogonal to this model.
