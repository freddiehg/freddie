# mercury's model is a crate that cannot call macOS

The model moves into its own workspace member, `mercury_model`, whose dependency graph contains no platform crate. "Handlers never read the outside world" stops being doctrine and becomes a link error: nothing in the crate's graph exports an OS symbol, the workspace's `unsafe_code = "forbid"` blocks declaring FFI, and a clippy disallowed-list closes std's own OS surface. The daemon, sources, performers, and CLI stay in the `mercury` crate with the platform dependencies.

figaro's half is `figaro/refactors/pending/model-crate.md`; the two are independent.

## The workspace

`Cargo.toml` at the root gains the member, beside mercury:

```toml
    "crates/mercury",
    "crates/mercury_model",
```

```toml
# crates/mercury_model/Cargo.toml
[package]
name = "mercury_model"
description = "mercury's model: a pure function of state and event, unable to reach the OS."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
freddie_sync = { path = "../freddie_sync", version = "0.0.1" }
laserbeam = { path = "../laserbeam", version = "0.0.1" }
bind = { path = "../bind", version = "0.0.1", default-features = false }
freddie = { path = "../freddie", version = "0.0.1" }
freddie_keys = { path = "../freddie_keys", version = "0.0.1" }
freddie_windows_types = { path = "../freddie_windows_types", version = "0.0.1" }
derive_more = { version = "2", features = ["from", "try_into"] }

[features]
# Derives the tests need and the normal build does not: effect equality, for asserting what a
# dispatch produced. Turned on for this crate's own tests through the self dev-dependency below,
# and kept out of `cargo build` by resolver 3.
testing = ["freddie/testing"]

[dev-dependencies]
# Turns the check on for TESTS only. With resolver 3 a dev-dependency's features do not leak
# into the normal build.
bind = { path = "../bind", version = "0.0.1", features = ["check"] }
# Turns `testing` on for this crate's own tests, the same way.
mercury_model = { path = ".", features = ["testing"] }
# The transitions tests rebuild a timer effect to assert what entering nav produced.
freddie = { path = "../freddie", version = "0.0.1" }

[lints]
workspace = true
```

The workspace lint table already carries `unsafe_code = "forbid"`, so unlike figaro's crate this one states nothing extra: inheriting the table is the forbid.

mercury's own `Cargo.toml` gains the dependency and reroutes its `testing` feature; everything else stays:

```toml
# in [dependencies]:
mercury_model = { path = "../mercury_model", version = "0.0.1" }

# [features], before:
testing = ["freddie/testing"]
# after — the derives it gates now live in the model crate:
testing = ["mercury_model/testing"]
```

The `tokio-tungstenite`, `futures-util`, and `tokio` dev-dependencies stay with mercury: they serve `tests/external.rs`, which does not move.

## What moves, what stays

Moves to `crates/mercury_model/src/`, paths otherwise unchanged:

- `state/` entirely — the tree, layers, `Mercury` itself, and `state/overlays/*.txt` (reached by `include_str!`, which resolves relative to the file, so the cards move with the module that includes them).
- `handlers/` entirely.
- `effect.rs` — effects are inert data; that is the point of them.
- `sources.rs` — the event vocabulary (`App::from_bundle_id` is string matching; the triggers are `bind` machinery).
- `model.rs` — the unified trigger/event/marker.
- `tests/transitions.rs` — it tests the model; it becomes `crates/mercury_model/tests/transitions.rs` and keeps `SimpleRunner`.

Stays in `mercury`: `main.rs` (with its `daemon`, `agent`, and CLI modules), `external.rs`, `tests/external.rs`, and `lib.rs` shrinks to the binary's own plumbing plus the wholesale re-export, so the daemon, the CLI, and `tests/external.rs` respell nothing:

```rust
// crates/mercury/src/lib.rs, after (module doc unchanged):
mod external;

pub use external::{DEFAULT_PORT, on_message};
pub use mercury_model::*;
```

`external.rs` keeps its `use crate::{...}` imports as they are: the names it uses now reach it through the re-export.

`mercury_model/src/lib.rs` is the moved module tree plus the re-exports that leave mercury's `lib.rs`:

```rust
//! mercury's model: a pure function of state and event.
//!
//! This crate cannot call macOS. No platform crate is in its dependency graph, the workspace
//! forbids `unsafe`, and clippy denies std's own OS surface (processes, threads, clocks,
//! files, sockets). `cargo check -p mercury_model` is the proof: if it compiles, dispatch
//! reads nothing but `(state, event)`. The one sanctioned impurity is timer-guard minting
//! through `freddie::timer_effect_and_guard`, which is channel construction, not an OS call.

pub use freddie_keys::{Key, KeyEvent, KeyPress, ModifierFlags, PressType};

mod effect;
mod handlers;
mod model;
mod sources;
mod state;

pub use effect::{Chord, Copied, MercuryEffect, UrlPart};
pub use freddie_windows_types::{Pid, Placement};
pub use model::{MercuryEvent, MercuryStruct, MercuryTrigger};
pub use sources::{
    AnyKey, App, FocusLanded, FocusRead, ForegroundEvent, Foregrounded, FrameLanded, FrameRead,
    Quit, Site, TabEvent, Tabbed, WindowEvent, Windowed, host,
};
pub use state::{
    AndReturnHome, AppData, AppLayer, ChromeApp, ClaudeAiSite, ForegroundedApp, ForegroundedChrome,
    FrontApp, GhosttyApp, HomeLayer, JK_TIMEOUT, Layer, Mercury, NavLayer, OVERLAY_DWELL,
    PLACEMENT_SETTLE, RETURN_TO_HOME_TIMEOUT, ResizeLayer, ReturnHomeLayers, SiteData, SiteLayer,
    TypingLayer, Windows, focus_read, foreground, frame_read, key, quit_event, tab,
};
```

(mercury's crate-level module doc stays on mercury's `lib.rs`; the model crate's doc is the contract above.)

## The import respells

The moved set names one platform crate today, `freddie_windows`, and only for its re-exported pure types, so every respell is `freddie_windows` → `freddie_windows_types`:

```rust
// effect.rs
use freddie_windows_types::{Pid, Placement, WindowId};
// sources.rs
use freddie_windows_types::WindowChange;
use freddie_windows_types::{Frame, Pid, WindowId};
// state/mod.rs
use freddie_windows_types::{Frame, Monitor, Pid, Placement, WindowId};
// handlers/window.rs
use freddie_windows_types::WindowChange;
// handlers/resize.rs
use freddie_windows_types::{Frame, Pid, Placement};
// tests/transitions.rs
use freddie_windows_types::{Frame, Monitor, WindowChange, WindowId};
```

Nothing else in the moved set names a platform crate (no `tracing`, no `serde`, no `ts_rs`; the `typescript` feature's derives live in `external.rs`, which stays), which is the audit this split freezes into place.

## Closing the std holes

`crates/mercury_model/clippy.toml`:

```toml
disallowed-methods = [
    { path = "std::process::Command::new", reason = "the model cannot run a subprocess; effects carry data and the binary performs them" },
    { path = "std::thread::spawn", reason = "the model runs on the dispatch thread; concurrency belongs to the sources and performers" },
    { path = "std::time::SystemTime::now", reason = "the clock is the outside world; time arrives as timer events" },
    { path = "std::time::Instant::now", reason = "the clock is the outside world; time arrives as timer events" },
]

disallowed-types = [
    { path = "std::fs::File", reason = "the model does not touch the filesystem" },
    { path = "std::fs::OpenOptions", reason = "the model does not touch the filesystem" },
    { path = "std::net::TcpStream", reason = "the model does not touch the network" },
    { path = "std::net::TcpListener", reason = "the model does not touch the network" },
    { path = "std::net::UdpSocket", reason = "the model does not touch the network" },
]
```

These run under the same `cargo clippy --all-targets` the pre-commit hook already keeps clean, at the same deny level.

## The proof command

`cargo check -p mercury_model` — the package alone, so no feature unification with the binary can smuggle symbols in — is the enforcement check, and it is what a change to the model is verified with before the full build.

## Order of changes

One change: the workspace member, the file moves, the import respells, the two `lib.rs` files, the clippy.toml, and mercury's manifest edits land together. Behavior-preserving; `cargo test -p mercury_model -p mercury` and a `mercury` binary build pin it.
