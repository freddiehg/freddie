# v1: announce mercury, then start figaro

Where mercury stands and what is between here and a public v1, then what starting figaro takes. `todos.md` is the raw list; this is the shape of it.

## What already works

The model is real and exhaustively tested: home, nav, resize, typing, and the in-app layer, with nav landing straight in the app's in-app layer, the in-app layer reaching `n`/`t` like home, and quit reachable from any layer. The runtime grabs the keyboard, re-emits, foregrounds apps, places windows on the correct monitor across displays, and shows a menu-bar status item with a Quit. The keyboard is released cleanly on exit. The dev killswitch is gone, so a run lasts until you quit it.

## Gaps to announce (v1 blockers)

From `todos.md`, grouped by what they need.

Chrome, the headline feature. Per-site remaps (claude.ai `n` -> new chat) are `chrome-tab-url.md`; the URL source is a small push-only extension, `chrome-extension.md`. This is the largest single item and the one that makes mercury more than a layered remapper. v0 is the extension streaming the active tab URL plus the site level in the model; the command bus is explicitly out of v1.

Living in the menu bar. Three related items:
- Run at launch, and ideally restart on crash: `launch-at-login.md`.
- The icon reflects state (which layer is active): the "later" half of `menu-bar.md`, a pure `label`/`menu` off the state tree.
- An overlay that shows what mercury is doing (e.g. voice-mode), openable from any state and from the menu bar, auto-closing after a while. No doc yet; this one needs designing.

The off switch. `enable-disable.md`: a global disable that passes keys through with a re-enable chord and menu-bar toggle. It doubles as the motivating case for the state-selected mutable child in laserbeam.

Polish and outward-facing:
- Audit: a pass over the bindings and the emitted-key correctness (the stuck-modifier class of bug the `cmd`-`escape` fix closed is the kind to hunt for), and the exhaustive-model standard extended to every reachable state.
- Documentation: the model is well-documented internally; what is missing is the user-facing "what mercury does and how to drive it."
- Website: the public front door.

Nothing here is blocked on a laserbeam change except enable-disable (which wants the state-selected mutable child) and the icon-reflects-state item (which is additive, not blocking). The Chrome extension is the critical path.

## Starting figaro

figaro is freddie's second consumer, and the README's rule is the whole point: anything mercury and figaro would write identically belongs in a `freddie_*` crate, not in mercury. So "start figaro" is really "prove the freddie boundary is where it should be," and the way to prove it is to write a second consumer and watch what mercury still holds that figaro would want too.

What figaro gets for free, already extracted: `freddie_keyboard` (grab and emit), `freddie_app_nav` (foreground and watch), `freddie_windows` (place, now multi-monitor), `freddie_main_loop` (own the main thread, pump NSApp), `freddie_menu_bar` (the status item), `freddie_keys`, and the `laserbeam` + `bind` core. That is most of the macOS surface.

What figaro writes itself, because it is mercury-specific and figaro's would differ: the `App`/`ForegroundedApp` enum and its bundle-id table, the state tree and its layers, the bindings, and the effects. figaro's version of each is bespoke by design.

The likely friction, and what starting figaro would surface:
- The bundle-id-to-app mapping and the app's bindings live in mercury; `foreground-events.md` and `app-foregrounding.md` already flag that figaro overrides this. First real test of whether the override seam is clean.
- The Chrome extension bus, if built for mercury, is a freddie-level capability (any consumer wants browser events), so it should land in a `freddie_*` crate, not in mercury. Deciding that before building it avoids a mercury-shaped extension.
- The overlay, if built, is likely `freddie_*` too (figaro would want an overlay identically), same as the audio and display source docs already call out.

So the honest sequencing: finish the mercury v1 blockers above, and as each cross-cutting one (extension, overlay) gets built, put it in a `freddie_*` crate from the start rather than in mercury. Then figaro is a new binary crate plus its own state tree, and the amount of copy-paste from mercury it needs is the measure of whether the boundary held. `ideas.md` argues the cheapest first test is smaller still: a second consumer with no keyboard at all (a build-pipeline state machine, a vending machine) to check that laserbeam and bind are not a keyboard remapper in disguise before figaro leans on them.

## Open questions

- The overlay: what it shows, how it renders (a borderless `NSWindow`? a separate lightweight surface?), and whether it is `freddie_*` or mercury.
- Whether v1 announces with the Chrome extension or ships it as a fast follow, given it is the critical path.
- Whether figaro starts before or after the extension, i.e. whether the second-consumer pressure should shape the extension's crate boundary from the start.
