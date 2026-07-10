# Overall plan

Status: retired to past. The design it describes is built; the parts that were not built have their own docs.

Built: the single-root state tree, variants-as-states, the `Path`/`Cursor` design (reborrow from the root, `into_parent` consumes), the `Laserbeam` and `bind` derives, effects-as-data, and one `Trigger` and one `Effect` enum per consumer. mercury is the working consumer. The crate layout settled roughly as sketched, plus `freddie_keys`, `freddie_keyboard`, `freddie_app_nav`, and `freddie_main_loop`.

Not built, and still wanted: the `freddie` crate is a one-line stub, not the framework tying laserbeam and bind together. The event loop lives in mercury and is bespoke, which bind's own comment argues is correct. The daemon CLI with `start`/`stop` and a pid file does not exist; see freddie-cli-plan.md and launch-at-login.md.

Its open questions found homes rather than answers. Enum-versus-substate binding precedence is freddie-dispatch-precedence.md, which ships static and non-clobberable. Validity encoded in types is the type-level enumeration note in laserbeam-missing-features.md. Per-keyboard identity in the `Keyboard` trigger is laserbeam-missing-features.md and modifier-keys.md. `Path` projection settled on `unreachable!` over `Option`. Constructing a target variant is explicit construction, with carry-over available since the cursor owns the data.

The `main.rs` this doc refers to is long gone; mercury's is in `crates/mercury/src/main.rs`.

## Goal

Precise modeling of a domain's state transitions. The point is that I can specify transitions exactly, in types. This is not about ergonomics, and not about being easy for other people to use. It is a low-level, precise tool. The driving use case is a keyboard state machine (mercury, the successor to an earlier Karabiner + Hammerspoon setup), but the core is a general "input -> action over a typed state tree" machine.

Non-goals, stated plainly so they stop creeping back in:

- Not optimizing for ease of use or a gentle learning curve.
- Not async. The core is synchronous.
- Not hiding behavior behind heuristics or magic primitives. Everything is explicit.

## What this is

- A Cargo workspace: `laserbeam`/`laserbeam_macro` (the typed path and its derive), `bind`/`bind_macro` (the binding layer and its derive), the `freddie` core (the event loop and effects), and the mercury daemon binary.
- Reference domain: an earlier Karabiner + Hammerspoon setup. mercury re-models the same keyboard behavior as a precise typed state machine instead of flat Karabiner variables.

## Reusable building blocks

The genericity goal is library reuse, not cross-compiling one binary. The macOS key-remapping daemon is one consumer of these libraries; a browser app is another, built from the same building blocks to do its own input -> action work. It is not the same code compiled to `wasm32`. It is a different app that reuses laserbeam, the `bind` accumulation and dispatch, effects-as-data, and the derives, while bringing its own state tree, its own inputs, and its own effects.

What this requires of the libraries is that they stay domain- and platform-agnostic, so a second consumer can pick them up:

- The reusable parts: laserbeam (cursors, `resolve`, `into_parent`/`get_root`), the `bind` accumulation and dispatch (the `Trigger` set, the diff, the outer registration handler), effects-as-data, and the derives. None of these name a keyboard, macOS, or Hammerspoon.
- What each consumer brings: the state tree, its own `Trigger` enum and the outer handler that registers its variants (CGEventTap/Hammerspoon for the daemon; DOM events for the browser), the sinks that perform effects, and the `Effect` set itself.

This is why neither `Trigger` nor `Effect` is a single global enum owned by a library. Within one consumer each is a single enum (the daemon has one `Trigger` and one `Effect`); across consumers the sets differ. The daemon's effects (emit a key, foreground an app, Hammerspoon arbitrary) mean nothing in a browser, and vice versa. The libraries provide the accumulate, diff, and dispatch machinery over whatever those enums are, and each consumer fixes its own. See `freddie-keys-plan.md`.

Other consumers in the same shape, beyond the daemon (mercury) and a browser app:

- A router. The active route is the state tree, resolving picks the active page, navigating switches a variant, and route params are fields rather than variants.
- The state of a reactive UI. The UI is in some state, e.g. looking at `/blog/:id`, on the blog-detail page, with a dropdown open. The pieces of data the current view reads (the blog, whatever the dropdown shows) are the active triggers: we accumulate exactly the data the current state looks at, the way mercury accumulates the active bindings. When a datum changes we propagate to the UI and re-render, so a blog change re-renders the detail page. Deleting the blog moves us to a 404 page, where there is no dropdown, so the dropdown's subscription drops out of the accumulated set and is deregistered, exactly like a key binding the new state no longer wants. Realtime updates fall out of treating the viewed data as subscribed inputs tied to the current state.

## Core model

- There is a single data structure: one root value (the base enum, call it `Layer`) that holds the entire state.
- Variants are states. Each variant wraps exactly one struct. These are not "layers" in any meaningful sense; a variant just means "we are in the situation modeled by this struct." Keyboard modes are one application of that, not the model.
- A struct is the end of the line for the derive's enum requirements, but it can hold arbitrary, non-trivial data, and it can itself nest a further `Laserbeam`-derived enum. A struct whose data evolves (a buffer going `[a]`, `[a, b]`, ...) represents distinct logical states without needing new variants. Those are separate states, just not separate variants.
- Everything knows its parent. The whole tree is reachable from the single root. This is similar to the `ResolvedItem` stuff in the isograph LSP.
- Two ways to express "knows its parent": generate paths that point to parents (more composable, probably not useful here), or have a single known base layer. Decision: single base layer.

## Synchronous, low-level, no async

- The trait has no `after(500)` and nothing async on it. The core does not own a clock or an executor.
- Timers are modeled as explicit states. The pattern: we enter a state, we schedule a timer for 500ms, and after 500ms we handle that as just another event. Before-event and after-event are two distinct states. There is no hidden tap-vs-hold primitive.
- Scheduling and external event sources live in userland. The library maps inputs to actions and exposes a way to listen; wiring real sources (the keyboard, a scheduled timer, a socket) is the user's job.
- "Special" keys are not special. `Cmd` down is a state transition `A -> B`; `Cmd` up is `B -> A`. Momentary behavior is two states and two transitions, not a primitive. The library should provide helper fns that make declaring this hold-pattern less tedious, but it is still just states and transitions.

## The derives (`Laserbeam` and `bind`)

What the derive enforces and generates:

On an enum:

- Each variant must wrap a type that implements the trait. The enum's impl delegates to the active variant's bindings and adds its own. Whether the enum's bindings override a child's, or collisions must be resolved explicitly, is open.
- Enum-level bindings (e.g. `F3 -> show_overlay`) are active across all of that enum's sub-states.
- Ideally the derive validates, with validity encoded in types: you have to validate in order to proceed (valid by construction), rather than validating at runtime.

On a struct:

- Build the struct's own trigger -> handler map. No delegation; it is the end of the line.

Both:

- One trait method that returns the bindings (a map of trigger -> handler).
- A separate, non-trait fn that takes those bindings and connects them: it hands the accumulated triggers to the outer handler, which registers them and listens. Connecting is deliberately outside the trait.

## Bindings and input sources

- A node declares bindings with `#[bind(trigger, handler)]`; the derive emits a map of trigger to handler. The bindings model is detailed in `freddie-keys-plan.md`.
- Generic over input. Keys are one kind of input; arbitrary other events work the same way. Real sources beyond keys are punted to userland.
- One `Trigger` enum, not per-input distinct types. A trigger is a value like `Keyboard::new('g')` or `Foreground::new("Chrome")`, unified into a single `Trigger` enum (one variant per source, `derive_more::From` for the `.into()` lift). This reverses an earlier plan that wanted a distinct type per input (`F3Press`, `SpacePress`) so double-binding was a type error: with one enum the discriminant is a runtime variant, so a double-binding is caught at runtime during accumulation rather than at the type level. The tradeoff is accepted, and adding a source edits the central enum, which is cheap.
- One outer handler owns registration. It receives the accumulated `Trigger` diff and routes each variant to its OS mechanism (a keyboard tap, a workspace observer). Trigger values stay pure data and hold no reference to that state; per-trigger OS handles live on the handler keyed by trigger. Connecting is deliberately outside the trait.

## Actions mutate the single structure through a `Path`

Actions receive a cursor into the state (a `Path`) and mutate the single data structure in place. This supersedes the earlier debate about returning `Layer` vs a boxed trait object: there is no boxing and nothing is returned, the action just mutates through the cursor.

Shape:

```rust
#[derive(Laserbeam, Default)]
#[laserbeam_root(resolved = LayerResolved)]
#[bind(Keyboard::new("f3"), show_overlay)]
enum Layer {
    Nav(Nav),
    Typing(Typing),
}

#[derive(Laserbeam, Default)]
#[laserbeam(path = NavPath, resolved = LayerResolved)]
#[bind(Keyboard::new("space"), to_typing)]
struct Nav {}

#[derive(Laserbeam, Default)]
#[laserbeam(path = TypingPath, resolved = LayerResolved)]
struct Typing {}

// switch the parent enum to a new variant, through the cursor
fn to_typing(nav: &mut NavPath) {
    *nav.parent().node_mut() = Layer::Typing(Typing::default());
}
```

An optional context struct lets actions close over external handles; pass it as a second argument when present.

## The `Path` (current focus)

This is the hard part. Goal: expose a mutable reference to the leaf node, and let you ascend to the parent (consuming the cursor, since the usual reason to ascend is a transition that reassigns the parent and thereby invalidates the child), up to the root, using only `&mut` and ownership: never two aliasing mutable references at once, and no `Rc`/`RefCell`.

Why the naive struct does not work:

```rust
struct Path<Item, Parent> { inner: Item, parent: Parent }
// NavPath = Path<&mut Nav, Path<&mut Layer, ()>>
```

`Nav` lives inside `Layer`, so storing `&mut Nav` and `&mut Layer` simultaneously aliases the same memory. That is the bug behind `*state.parent = Layer::Typing(...)` while also holding `&mut Nav`. Rejected.

Working design: store only the single root borrow. Reborrow from the root on every access, so exactly one mutable borrow is ever live. Ascend by consuming the cursor.

```rust
/// A cursor into the single state tree. Implementors reborrow from the root
/// down to their own node, so only one mutable borrow is ever live.
pub trait Cursor {
    type Node;
    fn node_mut(&mut self) -> &mut Self::Node;
}

/// The root cursor owns the only `&mut`.
pub struct Root<'a, R> {
    root: &'a mut R,
}

impl<'a, R> Cursor for Root<'a, R> {
    type Node = R;
    fn node_mut(&mut self) -> &mut R {
        self.root
    }
}

/// One level down: a parent cursor plus a projection from the parent's node to
/// this node. The projection matches the currently-active variant.
pub struct Step<P: Cursor, N> {
    parent: P,
    project: fn(&mut P::Node) -> &mut N,
}

impl<P: Cursor, N> Cursor for Step<P, N> {
    type Node = N;
    fn node_mut(&mut self) -> &mut N {
        // reborrow the whole chain from the root, then project this level
        (self.project)(self.parent.node_mut())
    }
}

impl<P: Cursor, N> Step<P, N> {
    /// Consume this cursor to recover the parent cursor.
    pub fn into_parent(self) -> P {
        self.parent
    }
}
```

The user-facing `Path` aliases are then built from these:

```rust
type LayerPath<'a> = Root<'a, Layer>;
type NavPath<'a> = Step<LayerPath<'a>, Nav>;
```

What the macro generates: the per-variant projections (the `match` that extracts the active child struct), the type aliases (`LayerPath`, `NavPath`, ...), and the dispatch that walks from the root to the active leaf, building the cursor, and calls the bound action. Sketch of generated dispatch:

```rust
fn dispatch(root: &mut Layer, event: Event) {
    match root {
        Layer::Nav(_) => {
            let path: NavPath = Step {
                parent: Root { root },
                project: |l| match l {
                    Layer::Nav(n) => n,
                    _ => unreachable!("cursor built for the active variant"),
                },
            };
            // look up `event` in Nav's bindings, call the action with `path`
        }
        Layer::Typing(_) => { /* ... */ }
    }
}
```

Properties:

- One live borrow. `node_mut` walks root -> node on each call, so there is never a second outstanding `&mut`.
- Ascending consumes. `into_parent` gives up the child cursor before the parent's node is touched, so a stale child reference cannot be used. This is exactly the aliasing bug, prevented by the type system.
- Validity by construction. The projection assumes the active variant matches; it does, because the cursor is built by dispatch for the variant that is currently active. Once an action switches the parent's variant, it has consumed the cursor, so there is no stale access. Open question whether the projection should be fallible (`Option`) for defense in depth or panic via `unreachable!`.

## CLI and daemon (clap)

The whole library's surface is clap-inspired: a derive on a data structure that generates behavior, the way clap derives a parser from a struct. The CLI itself uses clap.

The binary runs as a daemon. Commands:

- `start` — run the daemon. If one is already running, panic (refuse a second instance). `--force` kills the old pid first, then starts.
- `stop` — stop the running daemon.

Single-instance enforcement: `start` writes a pid file so there is only ever one daemon. `start` checks it; a live pid means refuse (or kill-then-start under `--force`). `stop` reads the pid file and signals that process.

v1 scope: not required to get something working. We can run the binary in the background and `pkill` it ourselves. But `start`/`stop` with a pid file is simple enough to do early.

## Open questions

- Enum bindings: when both the enum and an active sub-state bind the same key, does the enum override, or must it be resolved explicitly?
- How is validity encoded in types (valid by construction) rather than checked at runtime?
- `Path` projection: fallible (`Option`) vs `unreachable!` panic on a stale cursor.
- `Trigger`: how is per-keyboard identity represented within the `Keyboard` variant?
- Constructing a target variant on transition: `Default` vs carrying fields over from the old state. We own the data through the cursor, so carry-over is possible.
- Crate layout and final names.

## Crate sketch (provisional)

- `laserbeam` / `laserbeam_macro` — the typed path (`Path`/`Cursor`) and its derive.
- `bind` / `bind_macro` — the `#[bind]` derive and the binding machinery (accumulation, diff, dispatch over the `Trigger` set). See `bind.md`.
- `freddie` — the framework tying laserbeam and bind together: the event loop, effects-as-data, helpers for the hold-pattern.
- mercury — the daemon binary (see `main.rs` in this folder).

## Note

`main.rs` in this folder still reflects the older async/effect-loop model and is now stale relative to this rewrite. It should be re-aligned to the cursor/`Path` model before it is used as the reference example.
