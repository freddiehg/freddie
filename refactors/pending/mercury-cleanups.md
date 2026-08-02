# mercury cleanups

The figaro cleanups mercury still needs, in the same order they landed there. Mercury's `Mercury::handle` is already one `bind::dispatch` call and stays that way; every change here is binds and node state.

## Change 1: esc → home binds once on the root

The root already carries the shared-key pattern for `o` (typing's catch-all claims the key first, so the row never fires there). Escape joins it, and the five per-layer rows are deleted.

`src/state/mod.rs`, before:

```rust
// `o` binds once, here, because the overlay is the root's own field. In typing an `o` is an `o`:
// typing's own catch-all claims the key before the root's items run, so this bind never fires
// there.
#[bind(Key::KeyO.down() => toggle_overlay)]
```

after:

```rust
// `o` and escape bind once, here: in typing, its catch-all claims both keys before the root's
// rows run, so an `o` is an `o` and an escape is the app's.
#[bind(
    Key::KeyO.down() => toggle_overlay,
    Key::Escape.down() => go_home,
)]
```

The rows deleted, one each in `home.rs`, `nav.rs`, `resize.rs`, `site.rs`, `app.rs`:

```rust
    Key::Escape.down() => go_home,
```

The `Layer` enum's comment goes with them (it says escape is bound per layer, which stops being true):

```rust
#[derive(Bind, Debug, derive_more::From)]
#[node(parent = MercuryPath)]
#[binds(MercuryStruct)]
// This node binds nothing. `escape` leaves for home from every layer that binds keys as commands,
// but NOT from typing, where it is a key the app is waiting for, so it is bound per layer and
// typing simply does not have it. The return-home firing is bound the same way, by whichever layer
// set that timer, so it matches only its own.
pub enum Layer {
```

becomes:

```rust
#[derive(Bind, Debug, derive_more::From)]
#[node(parent = MercuryPath)]
#[binds(MercuryStruct)]
pub enum Layer {
```

Two behavior notes, both matching figaro:

- Escape in home now re-enters home (the root row fires; `set_layer(Home)` is idempotent) instead of home's own row doing the same thing. The `shows("Home")` assertion in any home-escape test keeps passing.
- In a return-home layer, the escape is no longer claimed by the leaf, so `AndReturnHome`'s `home_deadline` post sees it during descent and rearms the deadline; the root's `go_home` then replaces the layer, dropping `AndReturnHome` and its guard, which cancels the rearmed timer. No timer survives. `escape_goes_home_from_a_sublayer` and `escape_leaves_resize` assert exact effect vectors, so each gains the `return_home_timer()` ahead of its `shows("Home")`.

## Change 2: t → typing only from home and in-app

The figaro ruling applied here: typing entry is a chooser row home and the in-app layer carry, and nobody else. The `Key::KeyT.down() => enter_typing` rows in `nav.rs`, `resize.rs`, and `site.rs` are deleted; `home.rs` and `app.rs` keep theirs. Nav's `Space` (Spotlight → typing) is a different action and stays.

The overlay cards change in the same commit: the `t    typing` line leaves `nav.txt`, `resize.txt`, `site.txt`, and `claude-ai.txt`; `home.txt` and `inapp.txt` (with `chrome.txt`, `ghostty.txt`) keep theirs.
