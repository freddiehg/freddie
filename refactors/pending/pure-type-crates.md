# a platform crate's vocabulary is importable without its machinery

Every `freddie_*` platform crate splits its reported vocabulary — the pure data types its events and snapshots carry — into a sibling `_types` crate with no platform dependencies and `#[forbid(unsafe_code)]`. The platform crate depends on its types crate and re-exports everything, so existing consumers do not respell an import; a consumer that only speaks the vocabulary (a model crate that must not be able to call the OS) depends on the types crate alone, and the compiler enforces what today is doctrine: a crate whose dependency graph contains no platform crate cannot call macOS.

This is the prefactor for `figaro/refactors/pending/model-crate.md`, which builds figaro's model on exactly that guarantee.

## Change 1: `freddie_windows_types`

`crates/freddie_windows_types`, `#[forbid(unsafe_code)]`, depending on nothing. What moves in, verbatim, with their derives and docs:

- `Pid` (and it stays the shared process vocabulary: `freddie_ax_observer` and `freddie_selection`, both pending, re-export it from here instead of defining it).
- `WindowId`, `Frame`, `Monitor`, `WindowFrame`, `Snapshot`.
- `WindowChange` (and `FocusChange` when `windows-watcher-fixes.md` lands; whichever doc lands second carries the type in its shape).
- `WindowError`, if its variants carry no platform types; a variant wrapping an `AXError` keeps the error in `freddie_windows`.

`freddie_windows`'s lib gains `pub use freddie_windows_types::*;` and loses the moved definitions. No consumer changes an import; mercury and figaro compile untouched.

```toml
# crates/freddie_windows_types/Cargo.toml
[package]
name = "freddie_windows_types"
description = "The windows watcher's reported vocabulary, importable without the watcher."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[lints]
workspace = true
```

## Change 2: `freddie_displays_types`

The same split for `freddie_displays`: `Display`, `DisplayId`, and whatever else its report carries move to `crates/freddie_displays_types`, re-exported from `freddie_displays`. Same Cargo.toml shape.

## Change 3: the convention, written down

`docs/platform-apis.md` gains a section:

> ## The vocabulary is a separate crate
>
> A platform crate's reported vocabulary — the types its events, snapshots, and effects carry — lives in a sibling `_types` crate that depends on nothing and forbids `unsafe`. The platform crate re-exports it wholesale, so its own consumers import one name. The split exists for consumers that must not be able to reach the OS: a model crate depending only on `_types` crates has no platform symbol in its dependency graph, which turns "handlers do not call macOS APIs" from a review rule into a link error. A new platform crate starts with its types crate; an existing one grows it the first time a pure consumer wants its vocabulary.

The two pending crate docs follow it: `selection-watcher.md`'s vocabulary (`Selection`, `SelectionEntry`, `SelectionChange`, `SelectionGen`) lands in `freddie_selection_types` with `freddie_selection` re-exporting, and `freddie_ax_observer` re-exports `Pid` from `freddie_windows_types`. Each doc gets that one-line amendment when this lands.

## Order of changes

Three, independently shippable, in any order; each is behavior-preserving (moves and re-exports, no semantic change), pinned by the existing test suites compiling unchanged.
