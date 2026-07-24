# Handler kinds

Discussion, not scheduled. Dispatch has one handler kind today (exclusive, `bind`) and one planned (the pre-descent also-bind, `also_bind`, in `also-binds.md`). This maps the full space — pre, exclusive, post, and the paired pre+post — what each can and cannot do given how dispatch consumes the path, and which one the rearm actually wants.

## The four positions

- exclusive (`bind`): post-descent, one winner per event, CONSUMES the event (`Break` short-circuits). Runs at the winning node's turn.
- pre (`also_bind`): pre-descent, on every event that reaches the node, node-local (`Node<&mut P>`), does NOT consume. Runs at every node on the way down.
- post: post-descent, on the way up, on every event. Not yet built.
- pre+post: a paired handler — `pre` runs on the way down and returns an intermediate value, `post` runs on the way up and receives it. `pre`-only and `post`-only are this with the other half a no-op.

We control the proc macro, so emitting a post phase is mechanically easy. The constraint is not the macro; it is the path.

## Post can only run at the root

An exclusive winner consumes the path: every mercury handler `ascend_mut`s to the root to mutate root state, which walks up and takes the whole `PathMut` chain with it, and then `Break`s. On `Break` there is no unwind, so no ancestor holds a path, so no ancestor's post can run.

A mid-tree post could still run in one case — when the subtree MISSES (returns `Continue`), the node recovers its path and could run a post on the way up. But "runs only when nothing deeper won" is exactly the timing of an exclusive bind; the only difference is that it would not consume. So a mid-tree post is either equivalent to a non-consuming exclusive bind (fires on a miss) or, if it must fire regardless of the outcome, impossible.

An UNCONDITIONAL post — one that fires whether or not something deeper won — therefore exists only where the path survives the entire descent. That is the root: `handle` holds `&mut Mercury`, lends it to `dispatch`, and has it back when `dispatch` returns. There is no mid-tree unconditional post. Post is a root post-dispatch hook.

## Tracing the unwind mechanism

The natural way to picture a post is on the unwind: descend into the child, and right before ascending back to the parent — when the child's dispatch returns, or hooked into the `into_parent` step — run the node's post. That is the right instinct, and tracing it shows why it collapses to the root.

Descent projects a path chain, each link borrowing the one above:

```
&mut Mercury ▸ layer_path ▸ AndReturnHome_path ▸ NavLayer_path
```

An exclusive handler that changes the layer — `open_chrome` — must reach `&mut Mercury` to call `set_layer`, and the only way there is `ascend_mut`, which CONSUMES the chain: it walks `NavLayer_path` up through `AndReturnHome_path` and `layer_path` to `&mut Mercury`, taking all of them. This is not incidental. `set_layer` does `self.layer = new`, and the borrow checker will not let you hold a path INTO `self.layer` while reassigning `self.layer`; the path has to be released first, and consuming it is the release.

So after the handler runs, two things are true at once: the path chain is gone (consumed), and the nodes it addressed are gone (the old `Layer`, `AndReturnHome`, `NavLayer` were dropped when `self.layer` was overwritten). The suspended frames — `AndReturnHome::dispatch`, `Layer::dispatch` — have no path to run a post with and no node to run it on. There is nothing to unwind through.

The root is the sole survivor because it never held a path INTO `self.layer`. It held `&mut Mercury`, lent a path built from it, and gets `&mut Mercury` back when dispatch returns; `self.layer` is a FIELD of the thing it owns, so overwriting the field does not invalidate the owner, and `handle` reads the new `self.layer` safely. So the root post is not merely the easy place, it is the only place — and the reason is a borrow-checker necessity, not a design preference.

## What would make mid-tree post real

A true mid-tree post needs the descent to be free of structural mutation, so the unwind path stays valid. That means handlers cannot `set_layer` inline; they would return a COMMAND — change the layer to X — that the framework applies at the root AFTER the unwind. Dispatch then becomes a pure walk: pre on the way down, the handler returns effects and commands without touching the tree, posts on the way up with the path and nodes intact, and the root applies the commands last. It is a command-pattern rewrite of the handler model — handlers stop reaching the root and start describing what should change — and far larger than adding a post phase. Under the model we have, where handlers mutate the root inline, post is root-only, full stop.

Worth noting the tension it would resolve, since it recurs: a post that runs BEFORE the command is applied sees the pre-transition state, and one that runs after sees the post-transition state, so a before/after compare still wants the root (where "after" is unambiguous). The command rewrite buys mid-tree posts that observe the pre-transition tree, which is a different thing from what the rearm's before/after needs.

## The pair is a before/after with a threaded snapshot

`pre` returns an intermediate value and `post` receives it, which is the shape of a before/after comparison: snapshot on the way down, compare on the way up. Since an unconditional post is root-only, the useful pair is at the root: `pre` snapshots before dispatch, `post` compares after. That is exactly what `Mercury::handle` does by hand today for the rearm — snapshot `discriminant(&self.layer)`, dispatch, compare — written as a declarative handler pair instead of inline driver code.

The pair also states the missing symmetry cleanly. The full handler is pre+post; `also_bind` is the pair with a no-op post (act on the way down, ignore the outcome); a bare post is the pair with a no-op pre (ignore the snapshot, act on the outcome). One mechanism, three shapes.

## What the rearm wants

The rearm is a before/after: reset the timer only when a key left you in the same layer. Two ways to spell it, and they trade off:

- Pre-only also-bind (`stay`) on `AndReturnHome`, from `timed-layer-wrapper.md`. The rearm lives ON the wrapper, where the timer lives — clean ownership. But it fires on EVERY key, including the ones that leave, arming a fresh timer the leaving handler then drops, so the schedule self-cancels. That wasted arm is the extra work.

- Pre+post pair at the root, over the home-vs-rest enum. `pre` snapshots the layer discriminant; `post` compares it after dispatch and rearms only when it is unchanged and the event was a key. No wasted arm — it never arms on a leave, because it sees the leave before deciding. But the decision is root-level and reaches into the current timed layer to rearm, which is the "the outside does the rearming" shape the also-bind was chosen to avoid, and it is a bigger proc-macro feature (paired handlers, a threaded value, a root post phase).

The mechanics of the pair are sound: `set_layer` runs deep during dispatch (the exclusive handler ascends and swaps `self.layer`), so by the time the root post runs, the layer has already changed. The discriminant compare sees the post-transition layer and correctly declines to rearm on a leave. Post is not "too early"; it is strictly after every state change the event caused.

So it is ownership-plus-waste (also-bind) versus no-waste-plus-root-decision (pre+post). The pre+post is the general, "missing" handler and it removes the only blemish on the also-bind rearm (the self-cancelling timer on leaves); the also-bind is the smaller feature and keeps the rearm on the wrapper.

## The open decision

Whether to build post/pre+post at all, or ship the rearm as the pre-only also-bind and accept the wasted arm. Points:

- The also-bind (pre-only) is already fully specified (`also-binds.md`) and needed regardless — a genuine pre-descent per-event effect (modifier tracking, an overlay clear) has no post half.
- Post/pre+post is additional proc-macro surface: a root-only post phase, and the threading of `pre`'s value into `post`. Given post is root-only, an alternative to a full handler kind is to keep the rearm's before/after as the named `rearm_after` method it already is, and NOT generalize — the pair earns its keep only if a second before/after concern appears.
- If it is built, `also_bind` becomes the pre-only special case of it, and the taxonomy is one mechanism (pre+post) with three degenerate spellings, which is tidier than two unrelated attributes.

Recommendation deferred to review: the wasted arm is cosmetic (a self-cancelling effect visible in tests), so the also-bind is shippable now; the pre+post is the cleaner end state but should wait for a second before/after user, so the general machine is justified by more than one case.
