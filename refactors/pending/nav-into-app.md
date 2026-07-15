# nav lands you in the app's in-app layer

Not built.

Today nav is a one-shot: `n c` foregrounds Chrome and returns to home, and you press `i`
separately to enter the in-app layer for whatever is foregrounded. So `n c i` reaches
Chrome's in-app bindings, but only after the watcher has reported Chrome as frontmost.

Wanted: navigating to an app takes you straight into its in-app layer.

Two ways to model it.

- An "about-to-be-Chrome" layer. `n c` enters a layer that is waiting for Chrome to come
  up; when the foreground event for Chrome lands, that layer transitions to the in-app
  layer. This keeps the layer honest about the gap between asking for an app and it
  actually being frontmost.
- Or just set state. `n c` sets `foregrounded = Chrome` and switches to the in-app layer
  immediately, without waiting for the watcher. Simpler, but the recorded foregrounded app
  is then a prediction that the real event later confirms (or, if the app failed to come
  up, contradicts).

The first is truer to the asynchrony; the second is less machinery. Decide which when this
is built.
