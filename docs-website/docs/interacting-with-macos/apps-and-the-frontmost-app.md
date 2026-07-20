---
title: Apps and the Frontmost App
sidebar_position: 4
---

# Apps and the Frontmost App

`freddie_app_nav` does two things: it foregrounds an app, and it watches which one is frontmost.

TODO: the notification the watcher subscribes to, and how a raw frontmost-app change becomes a `ForegroundEvent`.

TODO: how the root `Mercury` struct keeps the one copy of the foregrounded app, and how the inapp layer reads it through a [virtual field](../architecture/virtual-fields.md).

TODO: foregrounding — what an app is identified by, and what happens when it is not running.
