---
title: The Chrome Extension
sidebar_position: 7
---

# The Chrome Extension

The extension at `./chrome-extension` reports the URL of the foregrounded tab. Load it into Chrome and the site layer, reachable with `s` from home or from inapp, gets per-site bindings. On `claude.ai`, `n` creates a new chat, normally bound to `cmd-shift-o`, and leaves you in the typing layer, since a new chat lands in its prompt box.

TODO: loading the unpacked extension.

TODO: the event socket on `127.0.0.1:3883`, the frame the extension sends, and how it becomes a `TabEvent`.

TODO: how the site layer matches a host, and what happens on a site with no bindings.
