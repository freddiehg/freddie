# switchers

The everyday cycling mechanisms the keymap is missing: the cmd-tab family, at every level of the tree. Each level already has a "go to a named one" binding or a plan for it; what is missing is "go back to the last one" and "walk them", which are different bindings with different repeat behavior. This doc is a running list; new gaps land here.

## The previous app

The core of cmd-tab is a two-app bounce, and the model can already afford it: every front-app change arrives as a `Foregrounded` event, including ones mercury did not cause. The handler that assigns the front app also keeps the one it replaced:

```rust
pub struct Foreground {
    // ...the current fields...
    /// The app the current one replaced. Assigned, never accumulated: a change from A to B
    /// sets it to A, and a repeated report of B leaves it alone (idempotence).
    pub previous: Option<App>,
}
```

The binding foregrounds `previous` and goes home, one decision like the rest of nav; pressing it twice bounces, because the bounce falls out of the assignment. It binds only when `previous` is `Some`, which is the same no-value-no-binding trick as `app_data`.

A held-cmd-tab-style walker across the full MRU list (overlay showing the row, repeated presses stepping) is a real feature and explicitly deferred: it wants an MRU `Vec<App>`, the overlay, and a timed layer, and the two-app bounce is most of the daily value.

## Chrome tabs

Adjacent movement is Chrome's own: `ctrl-tab` and `ctrl-shift-tab` as `Tap`s in the Chrome layer, staying in the layer because walking repeats, exactly like tmux's window walk. Tab-by-index is `cmd-1` through `cmd-8` if wanted.

The switcher worth having is the addressed one: once the extension reports all tabs (chrome-control.md), the overlay can list them with keys, and choosing one sends the activate command at its `TabId`. That is a chooser layer over state the model already holds, the same shape as the agent picker in send-to-agent.md, and it goes home on choice.

The previous-tab bounce also becomes free at that point: the model sees front-tab changes as events already, so a `previous: Option<TabId>` beside the front tab is the same assignment as the previous app.

## Terminal

tmux's last-window: `tmux last-window` as a `Run`, or `ctrl-a` plus tmux's own binding as a `Tap`; the `Run` form needs no focus and works addressed at a session. Ghostty's own tabs cycle with `cmd-shift-[` / `cmd-shift-]` as `Tap`s in the Ghostty layer.

## Windows within an app

`cmd-backtick` cycles an app's windows and is a `Tap` in the in-app layer, staying, since walking repeats. The addressed version (a chooser over the window list `freddie_windows` already reports) is the same chooser-layer shape as the tab switcher and waits its turn behind it.
