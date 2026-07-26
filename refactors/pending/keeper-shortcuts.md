# keeper's shortcuts

What KeeperFill, the Keeper Chrome extension, can be driven by from the keyboard, and therefore what mercury can bind. Per ideas.md, a browser extension's integration point is a hotkey, not an API, so this is an inventory of the hotkeys.

## What the extension declares, measured

The installed extension (`bfogiafebfohielmmehodmfbbebbbpei`, v18.0.0) declares exactly one command in its manifest:

```json
"commands": {
  "_execute_action": {
    "suggested_key": { "mac": "Command+Shift+K" }
  }
}
```

`_execute_action` opens the extension's popup; there is no separate declared command for "fill this page" or anything finer. So `cmd-shift-K` (rebindable at `chrome://extensions/shortcuts`) is the entire command surface, and everything past it happens inside the popup, which Keeper's docs say takes keyboard navigation of its own: arrows to pick a record, enter to open it, shift-enter to launch and fill.

Re-check by rereading the manifest when the extension updates: `~/Library/Application Support/Google/Chrome/Default/Extensions/bfogiafebfohielmmehodmfbbebbbpei/<version>/manifest.json`, the `commands` key. A new declared command would be a new bindable entry point.

## The binding

A Chrome-layer binding taps `cmd-shift-K` and ends in typing, because what follows is Keeper's popup eating arrows and enter, and a command layer would swallow them. That is the whole integration:

```rust
Key::KeyK.down() => and!(tap_cmd_shift_k, enter_typing),
```

with `tap_cmd_shift_k` emitting `Tap(Chord { key: Key::KeyK, flags: ModifierFlags::COMMAND | ModifierFlags::SHIFT })`, the same unit shape as `tap_cmd_l`. Whether `k` is the right key in the Chrome layer, and whether the binding belongs on the site layers where logins actually happen instead, is the open question; the mechanics have no others.

Inline autofill (the Keeper lock icon inside a login field) needs no binding at all: focusing the field is what triggers it, and focusing a field is typing-layer territory already.

## What is deliberately not attempted

Driving Keeper's popup itself from mercury (synthesizing the arrows and enter) would mean mercury guessing at a list it cannot see; the popup is Keeper's UI and the user's eyes are on it. AX-clicking the toolbar button or the desktop app's menus (both sketched in ideas.md) is strictly worse than the hotkey and stays unbuilt.
