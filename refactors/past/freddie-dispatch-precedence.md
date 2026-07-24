# precedence and dispatch

How multiple bindings on the active path resolve to one behavior, or to several. Undecided. A priority model was built and reverted (see "what was tried"); the current leaning is fan-out. This records the discussion so the decision can be made before implementing again.

Terminology: leafward means closer to the active leaf (the more specific node), rootward means closer to the root.

## What ships today

The simplest thing. Dispatch tries the active child first and takes the first binding that matches, so a leafward binding wins by position, and exactly one handler runs. `accumulate` errors (`DuplicateTrigger`) if the same trigger is bound at two levels of the active path.

This has one wart, worked around by hand: typing's catch-all (`AnyKey`) sits at the leaf and matches every key, so it shadows the layer-level `escape`. The workaround was to make typing rebind `escape` itself. Broadly, a wildcard nearer the leaf shadows a specific binding nearer the root, and nothing detects it because `accumulate` compares triggers by equality and a wildcard's overlap with a specific key is not equality.

## The realization: two kinds of event

Keys are commands: one key should do one thing, the most specific binding. That is exclusive dispatch, one winner.

Foreground, and every notification like it (display change, modifier change, focused-element), are not commands. Several layers legitimately react to the same fact: the root records which app is frontmost, the in-app layer retargets its variant, an overlay redraws. That is fan-out, many handlers.

mercury only has dispatch for the first kind, so it modeled foreground as a command: one handler at the root that does everything, including reaching down to retarget the in-app layer. That central handler, and the sync bugs it caused, exist only because there is no fan-out. With fan-out, the root's handler is just `foregrounded = app` and knows nothing about the in-app layer, which subscribes to foreground for itself.

## The unification: do everything at the winning tier

Exclusion and fan-out are not two mechanisms. They are one: run every handler at the winning tier, and let ties fan out while gaps exclude.

- Escape in typing, if typing kept a catch-all: the layer's `escape` is a specific binding, the catch-all is a fallback below it. The specific tier wins, the fallback is below it and excluded. One handler.
- Foreground: the root and the in-app layer both bind it at the same tier. Both run. Fan-out.

So whether one handler runs or several depends only on whether their tiers tie. Commands are built so they do not tie (one binding per key on the active path), so they stay exclusive. Notifications tie on purpose, so they broadcast.

## But the catch-all is not a binding

The only competing pair in all of mercury is typing's catch-all versus the layer's `escape`. And a catch-all is not really a binding: it means "the default for keys I did not name." That belongs as a per-state policy, unhandled key passes through in typing and is swallowed in home, not as a wildcard that competes with real bindings. See the opt-out-of-capture idea in synchronous-dispatch.md.

Reframe the catch-all as a per-state default and mercury has zero competing bindings. Then pure fan-out is correct with no tiers at all: every explicit binding that matches runs, and if none did, the state's default applies.

## Why the priority model was reverted

The reverted commit made a trigger's test return `Handle(Priority) | DontHandle` and dispatched by the highest priority across the path. It fixed the catch-all shadow. But it was still winner-take-all: it ran the one highest-priority handler, never several, so it did not give the foreground fan-out that turned out to be the actual want. It solved a smaller problem, thoroughly, on the wrong axis.

It also over-generalized. "Explicit beats the default" is a two-level priority; the catch-all only ever needed those two levels. An `i32` of priorities is more than the one real case asked for.

So it was reverted, to decide the shape before building it again.

## The one question that decides everything

Does a leaf ever need to override an ancestor's *explicit* binding for the same key, replacing it rather than adding to it?

- If never: fan-out plus a per-state default is complete and simplest. Every real case is one binding, or independent reactors, or explicit-versus-default. No priority tiers.
- If ever: you need suppression beyond explicit-versus-default, and something like the reverted priority comes back.

The precedence doc's old motivating example, "global escape goes home, but in typing escape should type a character," is achievable with the default policy (typing does not bind escape, so the layer's escape fires) right up until you decide typing's escape should type, at which point you have two handlers for escape and a genuine override. That is a product decision, not yet made. Name a binding where a deeper layer must replace a shallower layer's action for the same physical key; if none exists, fan-out wins.

## The problem that may sink fan-out: mutation invalidates the rest

Fan-out means running several handlers for one event, each mutating the state tree through its path. But the first handler to run can mutate the tree so that the handlers queued after it are invalid: the variant they were going to run on is no longer active, or their path was computed against a tree that has since changed. This is the exact aliasing and staleness that laserbeam's `Path` design exists to prevent, and single-winner dispatch sidesteps it for free by running exactly one handler.

It is not a rootward-versus-leafward choice. Any handler in the fan-out can mutate and invalidate the others, whichever order they run in. Order decides who reads whose write; it does not decide who invalidates whom. So ordering the fan-out does not solve this.

There are three escapes, each with a cost.

A notification handler may only touch its own node's local fields, never restructure the tree or change which variant is active. Then the handlers cannot invalidate each other. Restrictive, and it is unclear it holds: the in-app layer retargeting its own variant on a foreground is a structural change, so even the motivating case may break the rule.

Re-resolve the active path between handlers, so each runs on a fresh valid path. Safe, and it lets an earlier handler legitimately remove a later subscriber (if the root's foreground handler navigates away from the in-app layer, the in-app handler should not run). The cost is a resolve per handler and a moving target for "which subscribers are left".

Collect responses read-only, then apply. Every handler sees the same pre-mutation tree, returns effects or a state delta, and the deltas are applied after. No invalidation, because nothing mutates during the fan-out. But this is the barnum model, handlers returning a value the engine writes back, which freddie deliberately rejected in favour of mutating through the cursor. Fan-out may force that model back for notifications.

This is the strongest argument against fan-out, and it points at the other option.

## The alternative fan-out avoids: derive on resolve

The reason the in-app layer wants a foreground handler is to keep its variant in sync with `foregrounded`. But sync is only needed because the variant is a stored copy. If the in-app layer instead *resolved* to the Chrome child exactly when `foregrounded == Chrome`, computed fresh every dispatch, there would be nothing to keep in sync, no handler, no mutation, and therefore no invalidation. Derivation is idempotent; it cannot invalidate anything.

The cost is the opposite one: `resolve` would have to read state held on an ancestor (the root's `foregrounded`) to pick a child, which laserbeam's resolve does not do today, it descends and each node picks its child from its own fields. This is the ancestor-selector generalization of laserbeam-state-controlled-children.md.

So the two ways to decouple the root from the in-app layer are fan-out, which reintroduces the invalidation problem, and derive-on-resolve, which does not mutate at all. The invalidation problem is a real reason to prefer the second.

## What changes regardless

`accumulate` forbids the same trigger at two levels (`DuplicateTrigger`). Root and the in-app layer both binding `Foregrounded` is exactly that, and is exactly the intentional fan-out. So whichever way the question above goes, `accumulate` has to stop treating "same trigger, two nodes" as an error and start distinguishing intentional sharing from accidental clobber.

## The road not taken

Dynamic fall-through, where a matched handler returns whether it handled the event and dispatch falls through on decline, is strictly more powerful than any of the above: a leafward handler can take some cases and pass the rest rootward. It is also the most machinery, a handler-return protocol and a kept chain. Nothing wanted so far needs it, and it composes as an additive step later if it ever does.
