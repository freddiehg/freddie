# Recovering the invalidation implementation

You are the agent implementing `refactors/pending/invalidation.md`. This file tells you where things stand and what to do next; the design docs remain the sole source of design truth.

## Where things stand

- Changes 0–4 are landed and green: fa2f155 (change 0), 74e8077 (change 1), 7b19f86 (change 2), 2f6c083 (change 3), 62345c7 (change 4, route-enum Up half).
- The derived-levels design now lives in its own doc, `refactors/pending/derived-levels.md`: ascent flattens to the place path beneath the `Node` chain (`HasPlace`), `DispatchIntoParent` is deleted in favor of `DispatchIntoPlace` for `Node`s only, and derived data reaches handlers through pres.
- Consequences folded back into invalidation.md since change 4 landed: the place `dispatch_into_parent_impl` is deleted outright, so the `#[node(parent = .., route)]` marker no longer exists (only the field-side `up = ..` argument remains), and `#[post]` / `#[pre_post]` parsing moved from change 6 into change 5, because the migrated derived tests read data through pres. Change 6 is now only the demo tree and the full walks.

## What to do

1. Wait until told that `derived-levels.md` is settled. Until then change 5 stays blocked: do not improvise, do not write an interim, do not narrow the change to fit.
2. When told to continue, reread `refactors/pending/invalidation.md` and `refactors/pending/derived-levels.md` end to end; both changed after change 4 landed.
3. Implement `completed-ancestors.md` first (its changes 1 and 2, laserbeam only, each its own commit): the state-level root reach and `TryIntoAncestor`. Mercury's change-5 migration lands directly on those shapes.
4. Implement `derived-levels.md` change 1 (`HasPlace` + `DispatchIntoPlace`, pure additions with unit tests), as its own commit.
5. Then invalidation change 5 as one workspace-global change: trait signatures, the linear `Completed` body, scheduled blocks, the place `dispatch_into_parent_impl` deletion, route folds, `up =` parsing, `#[post]` / `#[pre_post]` parsing, the derived codegen (before/afters in invalidation.md's change-5 section), and the handler migration in mercury and the bind tests. Workspace green, commit.
6. Then invalidation change 6 (demo tree + walks). Move the finished docs to `refactors/past` when the work is implemented and tested.

If any step stops matching the code, stop and report, as before; the fix goes into the doc first.
