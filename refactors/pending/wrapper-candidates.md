# What else is a wrapper

Discussion, not scheduled. `AndReturnHome` (`timed-layer-wrapper.md`) models the return-home timer as a wrapper: a node that owns state and binds, wraps a subtree, and acts on every event through it via an also-bind. Three questions: what else fits, do wrappers compose, and is the also-bind the only hook a wrapper needs. The short version: they compose; the also-bind is a PRE-descent hook and there is a symmetric TEARDOWN hook it does not cover; the overlay needs both, and the teardown one is the already-open `drop-emits-effects.md` question.

## Wrappers stack, and their also-binds commute

Nesting wrappers is fine. `root ▸ Overlay ▸ Layer ▸ AndReturnHome ▸ ...` is a legal tree; each owns its own binds, and no-clobber holds as long as their triggers are disjoint. On one event, every wrapper's also-bind on the active path fires — pre-descent, root-to-leaf, none able to prevent another (`also-binds.md`, "Also-binds compose"). So `AndReturnHome`'s `stay` and an overlay's `clear_overlay` both fire on one key, commutatively, because each mutates only its own node and emits independent effects.

The worry that an also-bind might stop `toggle_overlay` is unfounded: an also-bind never `Break`s, so it cannot short-circuit an exclusive bind. The only exclusive-vs-exclusive constraint is that two exclusive binds cannot claim one trigger on the path (clobber).

## Two hooks: while-active, and on-teardown

An also-bind fires while a node is active, on every event, pre-descent. That is one lifecycle hook. The other is teardown: do something when the node goes away — removed from the tree, its layer replaced. `AndReturnHome` needs only the first: `stay` resets the timer on every key. A wrapper that also needs "do X when I am torn down" needs the second, and Rust's name for it is `Drop`.

The return-home timer already leans on teardown, quietly: dropping the guard cancels the timer. But that cancellation is order-insensitive and emits no effect — it severs a channel. A teardown that must EMIT an effect (`HideOverlay`) is the hard case, because `Drop::drop` returns nothing. That is `drop-emits-effects.md`, and it is unsettled.

## The overlay: both hooks, and the hard one is teardown

The overlay is wrapper-shaped — it owns state (the showing and its dwell guard) and wants effects on events. Its two behaviors split one per hook, and only one is clean:

- Clear on any key — a pre-descent also-bind, `AnyKey => clear_overlay`. Easy, and crucially ORDERED: it pushes `HideOverlay` into the effect batch at a known point, like any handler, no channel involved. If "any activity dismisses the cheatsheet" is the behavior wanted, this is it. The one wrinkle is `o` itself: `clear_overlay` fires pre-descent and `o => show` fires post-descent, so on `o` the clear hides and the show re-shows — `o` becomes show-only, losing toggle-off, unless `clear_overlay` skips the show key at the handler level (`if ev.key != KeyO`).

- Close on layer change — a teardown. Scope the showing to the current layer; navigating away drops it and should emit `HideOverlay`. `Drop` is the instinct, and it is the wrong tool here, for the reason `drop-emits-effects.md` already worked out: `Drop::drop` cannot return, so it would push the effect out of band through a channel, and a channel-pushed effect is UNORDERED against the batch the handler is building. For the timer that is fine (cancellation commutes); for a shared panel it is not — a `HideOverlay` landing after a `ShowOverlay` from another path leaves a window on screen the model believes is gone. Order-sensitivity is the whole difference between the guard (fine) and the overlay (not).

So the overlay's teardown is not a `Drop` wrapper. `drop-emits-effects.md`'s two candidates stand: a `#[must_use]` `Shown<T>` setter (private field; `show`/`hide` return their effects; `set_layer` must call `hide`, so forgetting is a compile error and the effect stays ordered in the batch), or reconciliation (the model declares what SHOULD be on screen, the effect loop diffs it against what is, so nothing remembers to hide — which dissolves both hooks for the overlay, at the cost of a different effect architecture).

Net: the overlay is wrapper-shaped for its per-key behavior (an also-bind, if activity should dismiss), but its close-on-change is the open teardown question, and its answers live in `drop-emits-effects.md`, not in a `Drop` wrapper.

## Modifier tracking is a wrapper-free also-bind

`maybe_pass_through` (root, exclusive `AnyKey`) does two jobs: it records held modifiers on every key, and it passes the key through in a passthrough layer. The first is a textbook pre-descent also-bind — `AnyKey => track_held` at the root, mutating only root state, independent of everything. Splitting it out lets `maybe_pass_through` do one thing. No wrapper: the concern is global (every key, every layer), so it is a root also-bind, not a nested node. The other end of the spectrum from `AndReturnHome` — same mechanism, no subtree.

## Not wrappers

- The `jk` run (`typing_state.jk`) is typing-specific state a layer reads, not a cross-cutting effect on every event. Stays put.
- The front app and window frames are external truth mirrored into root state under the idempotence rule (CLAUDE.md). Seeded and event-fed; not subtree membership. Root state.

## The spectrum

Two axes, not one — where the state lives, and which hooks it needs:

- `AndReturnHome`: subtree state, pre-descent hook only (`stay`). A clean wrapper.
- Modifier tracking: global state, pre-descent hook only (`track_held`), no wrapper — a root also-bind.
- The overlay: global-ish state, BOTH hooks — pre-descent (`clear_overlay`, an also-bind) and teardown (close-on-change, which is `drop-emits-effects.md`, answered by a setter or a reconcile loop, not a wrapper).

The also-bind is a wrapper's while-active hook, and it is the whole story only when a concern needs nothing on teardown. `AndReturnHome` is that clean case because its teardown (cancel the timer) is a bare guard drop with no effect. The moment a wrapper needs to EMIT an effect when it goes away, it is no longer just an also-bind wrapper — it lands in `drop-emits-effects.md`, and the overlay is the first thing that does.
