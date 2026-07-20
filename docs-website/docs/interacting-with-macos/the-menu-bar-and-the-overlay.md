---
title: The Menu Bar and the Overlay
sidebar_position: 6
---

# The Menu Bar and the Overlay

`mercury` creates a menu bar item, which shows the current layer name and exposes a quit option. If you end up with a non-responsive keyboard while iterating, that is how you save yourself.

From any layer except typing, `o` shows an overlay of what is bound.

TODO: the main-thread requirement — which of these calls must run on the main thread, and how `freddie_main_loop` gets them there.

TODO: how the menu bar text is kept in sync with the layer, as an effect rather than a read of the state.

TODO: how the overlay builds its contents from the bindings on the active level.
