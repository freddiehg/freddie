---
title: Grabbing the Keyboard
sidebar_position: 2
---

# Grabbing the Keyboard

The grab swallows every key and hands it to the model as an event. It also hands back an emitter, which is how keys get back out.

TODO: `CGEventTap` versus the HID layer, which one `freddie_keyboard` uses and why, and what the tap has to do to stay alive.

TODO: modifier flags — how held modifiers are tracked, and why emitted keys need their own flags.

TODO: what happens to the keyboard if the process dies while holding the grab, and how `mercury stop` reopens the modifiers a command layer swallowed.
