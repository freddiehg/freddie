# Recovering the invalidation implementation

You are the agent that stopped at change 4 of `refactors/pending/invalidation.md` with two blockers (the derived interim and route-enum parents). Stopping was correct: both were defects in the doc, both were raised, and the doc has been revised since. This file tells you what changed and what to do; the doc itself remains the sole source of design truth.

## Where things stand

- Changes 0–3 are landed and green: fa2f155 (change 0), 74e8077 (change 1), 7b19f86 (change 2), 2f6c083 (change 3).
- If you stashed partial change-4 work, discard it. The change you stopped on has been renumbered and revised; old in-flight code predates decisions made since.

## The doc was renumbered

Reread `refactors/pending/invalidation.md` in full before touching code. The mapping from the numbering you knew:

- New change 4: route enums. This change did not exist when you stopped.
- Old change 4 (the linear `Completed` body) is now change 5. It additionally carries: the `Dispatch`/`DispatchIntoParent` trait signature change (the old "Landed baseline" presented those as landed; they are not), the route fold in `dispatch_body`, the `up =` / `route` attribute parsing, and the `dispatch_into_parent_impl` skip for `route` nodes. Its derived interim is gone; derived levels migrate per the doc's "Derived levels" section instead.
- Old change 5 (`#[post]` / `#[pre_post]`) is now change 6.
- Old change 6 (derived levels) is no longer a separate change. Its content will be a "Derived levels" design section, implemented within change 5.

## Blocker resolutions, as decided

Route enums (your blocker 2): support stays; figaro will need it. The consumer hand-writes both directions per route enum: the route enum (exists), an Up enum whose variants carry each parent's `Completed`, and a one-line `Above` impl. No derive, no laserbeam changes. The two `unreachable!()`s in the generated fold are approved; they assert the same descent-built-this-variant invariant `Edge::recover_parent` asserts today. Route-parented nodes get no `DispatchIntoParent` impl (the `route` marker skips emission, as `is_root` already does). Full design: the doc's "Route enums" section.

Derived levels (your blocker 1): not yet resolved. The doc's "Derived levels" section currently records only the constraints the design must satisfy. It is being iterated with Robert and will be written out before change 5 proceeds.

Smaller gaps you worked around, now ratified in the doc's "Landed baseline": the `dispatch::<Demo, App, _>` turbofish, `Mercury::handle -> (Vec<MercuryEffect>, bool)` with the rearm gated on the claim, `SimpleRunner::next -> Option<(Vec<E>, bool)>` and `process_event` returning the pair.

## What to do

1. Reread `refactors/pending/invalidation.md` end to end.
2. Implement change 4 exactly as its section specifies: `TitleParentUp` and the `Above` impl beside `TitleParent` in `crates/bind/tests/common/mod.rs`; the `title_shapes` pin in `crates/bind/tests/complete.rs`; delete `crates/bind/tests/compile_fail/route_parent_completed.rs` and its `.stderr`. Nothing else: the `up =` / `route` attribute changes, the `home` bind, and `title_home` belong to change 5 (they need change 5's handler shape and parsing). Workspace green, one commit.
3. Stop. Change 5 is blocked until the doc's "Derived levels" section holds the full design. Do not improvise it, do not write an interim, do not narrow change 5 to fit. Report that change 4 is landed and you are stopped on "Derived levels".
