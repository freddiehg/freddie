# roadmap

The goal is not figaro yet. The goal is to get mercury to a state worth announcing.

That reorders everything, because "announceable" is a different bar than "more features". It is the bar where someone who is not the author can install the thing, run it, and understand what it is, without it dying after sixty seconds or swallowing their keyboard with no way out.

## The one question the announcement has to answer first

Who is it for, because that decides what mercury even is.

If the audience is programmers, the pitch is the thesis: a keyboard remapper as a typed state machine, bindings in Rust, valid by construction, checked by the compiler. Karabiner is JSON and Hammerspoon is Lua; mercury is a program. Then bindings living in Rust is not a gap, it is the entire selling point, and "announceable" means the developer experience of writing a remapper is good.

If the audience is end users, bindings in Rust is a wall, and announceable means configuration without recompiling, which is a real project mercury has not started and which cuts against the typed-by-construction pitch.

These are two different products and two different roadmaps. Nothing below can be sequenced until this is answered. The rest of this doc assumes the first, the programmer pitch, because it is the one the design is already built for.

## What announceable requires, that mercury does not have

The dev killswitch has to go, or become opt-in. A thing that force-quits after sixty seconds is a demo, not a tool. Its backstop role is real while iterating, so it becomes a flag, off by default. This is small.

It has to survive a login. `launch-at-login.md` is the doc, and its blockers are the blockers here: a signed binary at a stable path so the Accessibility grant is not invalidated by every rebuild, a LaunchAgent, and a way to disable it without the keyboard. Announcing a keyboard swallower with no recovery story is announcing a footgun.

It needs a default that makes sense cold. This is priority 3 from before, and it is now load-bearing rather than nice: the first thing anyone runs is the default, and the default currently boots into a layer that swallows every key but four. The focused-element source (`kAXFocusedUIElementChangedNotification`, in `ideas.md`) is the likely answer, a model that passes keys through when the cursor is in a text field and is modal otherwise.

The developer experience of writing bindings has to be the good part. If the pitch is "write your remapper in Rust", then adding a binding, a layer, an effect has to be clean, and the errors have to be legible. This is where the sharp edges found by building more effects (priority 1) pay back: every wart in the binding surface is a wart in the pitch.

## What is not required for the announcement

HID. It is the thing the project is secretly for, and it is a DriverKit project gated on an Apple entitlement. A `CGEventTap` remapper is announceable now; the loop hole and secure-input gaps are real but they are not what a first announcement lives or dies on. HID is the second chapter, not the first. `virtual-hid.md` stands; it just is not on the critical path to announcing.

figaro. Explicitly deferred. It is the thesis test and it matters, but it is after the announcement, not before it.

## Order

1. Answer the audience question. Everything hangs on it.
2. Kill the killswitch-by-default, and settle the default schema. Small, and they are what make a cold run coherent.
3. Build more effects and events, because the surface is the demo and building it is what surfaces the DX warts.
4. Make it survive a login: signing, the agent, the recovery story.

HID and figaro are chapter two.
