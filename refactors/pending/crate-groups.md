# group crates under freddie/ and mercury/

Split the flat `crates/` directory into two groups by reuse. `crates/freddie/` holds every crate a second app on the framework (figaro) would also depend on: the state-tree engine, the binding layer, and the reusable macOS building blocks. `crates/mercury/` holds only what is specific to the mercury app.

The rule for placing any future crate: reusable by another app on freddie goes in `crates/freddie/`; specific to one app goes in that app's group.

## target layout

```
crates/
  freddie/
    freddie/
    laserbeam/
    derive_support/
    bind/
    bind_macro/
    freddie_keys/
    freddie_keyboard/
    freddie_app_nav/
    freddie_main_loop/
    freddie_menu_bar/
    freddie_windows/
  mercury/
    Cargo.toml
    src/
```

`mercury` stays where it is. The other eleven crates move under `crates/freddie/`, the `freddie` crate among them at `crates/freddie/freddie/`. No crate is renamed; only its directory moves.

## change 1: move the eleven crates

The `freddie` crate occupies `crates/freddie`, the name the group directory takes, so it moves aside first, then back in.

```sh
git mv crates/freddie crates/freddie-pkg
mkdir crates/freddie
git mv crates/freddie-pkg          crates/freddie/freddie
git mv crates/laserbeam            crates/freddie/laserbeam
git mv crates/derive_support       crates/freddie/derive_support
git mv crates/bind                 crates/freddie/bind
git mv crates/bind_macro           crates/freddie/bind_macro
git mv crates/freddie_keys         crates/freddie/freddie_keys
git mv crates/freddie_keyboard     crates/freddie/freddie_keyboard
git mv crates/freddie_app_nav      crates/freddie/freddie_app_nav
git mv crates/freddie_main_loop    crates/freddie/freddie_main_loop
git mv crates/freddie_menu_bar     crates/freddie/freddie_menu_bar
git mv crates/freddie_windows      crates/freddie/freddie_windows
```

The eleven move as a unit, so their path dependencies on each other are unchanged: `bind`'s `../laserbeam` and `../bind_macro`, `bind_macro`'s `../derive_support`, `freddie_keys`'s `../bind`, and `freddie_keyboard`'s `../freddie_keys` all still resolve, because both ends moved together. The only path dependencies that change are `mercury`'s, since it did not move (change 3).

## change 2: workspace members

`crates/Cargo.toml` root, before:

```toml
members = [
    "crates/freddie",
    "crates/laserbeam",
    "crates/derive_support",
    "crates/bind",
    "crates/bind_macro",
    "crates/freddie_keys",
    "crates/freddie_keyboard",
    "crates/freddie_app_nav",
    "crates/freddie_main_loop",
    "crates/freddie_menu_bar",
    "crates/freddie_windows",
    "crates/mercury",
]
```

after:

```toml
members = [
    "crates/freddie/freddie",
    "crates/freddie/laserbeam",
    "crates/freddie/derive_support",
    "crates/freddie/bind",
    "crates/freddie/bind_macro",
    "crates/freddie/freddie_keys",
    "crates/freddie/freddie_keyboard",
    "crates/freddie/freddie_app_nav",
    "crates/freddie/freddie_main_loop",
    "crates/freddie/freddie_menu_bar",
    "crates/freddie/freddie_windows",
    "crates/mercury",
]
```

## change 3: mercury's path dependencies

`mercury` stayed at `crates/mercury/`, and its dependencies moved from `crates/<c>` to `crates/freddie/<c>`, so each `../<c>` becomes `../freddie/<c>`.

`crates/mercury/Cargo.toml`, before:

```toml
laserbeam = { path = "../laserbeam", version = "0.0.1" }
bind = { path = "../bind", version = "0.0.1", default-features = false }
freddie_keys = { path = "../freddie_keys", version = "0.0.1" }
freddie_keyboard = { path = "../freddie_keyboard", version = "0.0.1" }
freddie_app_nav = { path = "../freddie_app_nav", version = "0.0.1" }
freddie_main_loop = { path = "../freddie_main_loop", version = "0.0.1" }
freddie_menu_bar = { path = "../freddie_menu_bar", version = "0.0.1" }
freddie_windows = { path = "../freddie_windows", version = "0.0.1" }
```

after:

```toml
laserbeam = { path = "../freddie/laserbeam", version = "0.0.1" }
bind = { path = "../freddie/bind", version = "0.0.1", default-features = false }
freddie_keys = { path = "../freddie/freddie_keys", version = "0.0.1" }
freddie_keyboard = { path = "../freddie/freddie_keyboard", version = "0.0.1" }
freddie_app_nav = { path = "../freddie/freddie_app_nav", version = "0.0.1" }
freddie_main_loop = { path = "../freddie/freddie_main_loop", version = "0.0.1" }
freddie_menu_bar = { path = "../freddie/freddie_menu_bar", version = "0.0.1" }
freddie_windows = { path = "../freddie/freddie_windows", version = "0.0.1" }
```

The `[dev-dependencies]` entry for the check feature moves the same way, before:

```toml
bind = { path = "../bind", version = "0.0.1", features = ["check"] }
```

after:

```toml
bind = { path = "../freddie/bind", version = "0.0.1", features = ["check"] }
```

## change 4: the release job's manifest globs

`.github/workflows/ci.yml` rewrites versions across the manifests with `crates/*/Cargo.toml`, which matches one level and would miss the crates now nested at `crates/freddie/*/Cargo.toml`. Both release jobs (`main-release`, `versioned-release`) carry the same two loops.

The version-rewrite loop, before:

```sh
for f in Cargo.toml crates/*/Cargo.toml; do
```

after:

```sh
for f in Cargo.toml $(find crates -name Cargo.toml); do
```

The dependency-pin loop, before:

```sh
for f in crates/*/Cargo.toml; do
```

after:

```sh
for f in $(find crates -name Cargo.toml); do
```

`find crates -name Cargo.toml` reaches every crate manifest at either depth and excludes the root `Cargo.toml`, which the first loop still names explicitly. The `cargo publish -p laserbeam` and `-p freddie` steps are unchanged: they select by package name, not path.

## unaffected

- The publish commands (`cargo publish -p <name>`) and the crate package names.
- `rust-toolchain.toml`, `.pre-commit-config.yaml`, and the CI runner OS: none reference crate paths; the cargo hooks run at the workspace root.
- `Cargo.lock` regenerates on the next build with the new member paths.

Other pending refactor docs reference the pre-move paths (`crates/bind/src/lib.rs`, `crates/mercury/...`, and so on). Those go stale on this move and are corrected when each is next worked on, per the staleness policy.
