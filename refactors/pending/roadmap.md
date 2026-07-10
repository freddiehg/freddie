# roadmap

The four things that matter next, in the owner's priority order. Everything else in `refactors/pending` is a note; this is the plan.

## 1. Flesh out events and effects, and build more of them

The model is proven; the surface is thin. `effects-and-events.md` and `ideas.md` are the backlog. This is the priority that produces visible value on every commit, and it is where the "message an app at an address" shape gets exercised: tmux commands, Chrome tab targeting, the LLM rewrite, window and audio and display work.

It is also the cheapest way to keep finding the sharp edges early, the way the flags bug and the key-up bug turned up only once real bindings existed.

## 2. HID

The one the whole thing is secretly for. mercury on a `CGEventTap` is a decent remapper for a shared keyboard; the idea that makes sense in the owner's head is an external keyboard that is *entirely* mercury's, which is HID. See `virtual-hid.md` and `cgevent-vs-hid.md`.

HID is the thing three other docs are quietly waiting on. It is the only real fix for the cross-process loop (`synchronous-dispatch.md`). It is the only way to know which physical keyboard a key came from, which is the real motivation for multiple `#[resolve_into]` (`laserbeam-missing-features.md`) and for per-keyboard layers. And it makes secure-input fields and cmd+Tab reachable, both of which the tap cannot touch.

It is also the largest single item on the page: a DriverKit extension, a managed entitlement Apple grants, a root daemon, and IPC, or leaning on Karabiner's already-installed driver. The API does not change above `Grab`, so nothing built now is wasted.

## 3. A good default schema for mercury

The `App`/`Layer`/`AppLayer` tree grew binding by binding and nobody has sat down and asked what the *default* mercury should be. Home swallowing everything but `n`/`t`/`i`/`q` is why `launch-at-login.md` is blocked: autostart it and the machine looks broken.

So this is not cosmetics. The boot state, whether unbound keys pass through or are eaten, and whether the top level should be driven by the focused-element source (`ideas.md`) rather than chosen, are all the same question: what is the sensible thing for a keyboard that is always on to do when you are not asking it for anything.

Small in code, load-bearing for shipping.

## 4. Start figaro

The thesis test. `refactors/past/overall-plan.md` claims laserbeam and bind are not keyboard-specific, that a router and a reactive UI are the same machine. Nothing has tested that, and mercury cannot: it is the only consumer, so every abstraction looks right because it is the only thing there. The README's rule about what belongs in a `freddie_*` crate versus mercury has been applied on faith.

figaro is the second consumer that makes the rule real. If sharing laserbeam, bind, `freddie_main_loop`, and the derives across two apps is awkward, the abstraction is a keyboard remapper in a costume, and better to learn it against a second real app than a toy.

## How they relate

1 and 2 are independent and can run in parallel; 1 is incremental, 2 is a project.

3 should come before autostart is attempted, and probably before 4, because figaro will copy mercury's shape and it should copy a considered one, not an accreted one.

4 is the one that could invalidate the others: if figaro reveals the core is wrong, that changes what 1 and 2 are even building on. So there is an argument for a thin figaro early, purely to test the shape, before investing more in mercury's surface.
