# nav lands you in the app's in-app layer

Built.

Before, nav was a one-shot into home: `n c` foregrounded Chrome and returned to home, and you pressed `i` separately to enter the in-app layer for whatever was foregrounded. So `n c i` reached Chrome's in-app bindings, but only after the watcher reported Chrome as frontmost.

Now navigating to an app takes you straight into its in-app layer: `n c` foregrounds Chrome and lands in the in-app layer, and once Chrome is frontmost, `r` refreshes it, with no `i`.

## How it works

The root grows one bool, `has_navigated`, next to `foregrounded`. A nav choice does three things: it sets `has_navigated = true`, switches the layer to `InApp`, and emits the `Foreground` effect. It does NOT record the app; the app is still whatever was frontmost before. When the watcher reports the app that actually came up, `on_foregrounded` records it in `foregrounded` and clears `has_navigated`.

`has_navigated` closes the gap between asking for an app and it becoming frontmost. During that gap `foregrounded` is still the old app, so the in-app level must not bind the old app's keys. The derived child `app_data` returns `None` whenever `has_navigated` is set, so the in-app layer is empty until the foreground event lands and clears the flag, at which point it resolves the newly-recorded app. A key pressed in the gap is unbound rather than misdirected to the previous app.

This is the second of the two options the original plan sketched (set state rather than model an explicit "about-to-be-Chrome" layer), with the flag standing in for the asynchrony the first option would have encoded in a layer: the recorded app is a fact set by the event, and `has_navigated` marks the interval where that fact is not yet known.

No new variants: neither `App` nor the in-app `AppData` gained a case. `foregrounded` already holds the app, and the empty-during-nav state is `None` from `app_data`, not a variant.

The `i`-from-home path is unchanged: it enters the in-app layer for the already-foregrounded app, with `has_navigated` false, so `app_data` resolves normally.
