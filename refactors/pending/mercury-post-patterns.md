# Mercury: one gesture per bind, one concern per post

`invalidation.md` and `completed-ancestors.md` are landed (both in `refactors/past`), so every mercury behavior is a scheduled item. The state-level `into_ancestor` / `try_into_ancestor` are assumed throughout: they are what let a unit that touches the root be branch-free.

## Rule

A handler owns exactly one of:

```text
gesture     exclusive bind: deepest wins; the WHOLE of one user action, composed
            from units with and(..) at the bind site when it has parts
concern     post / pre_post: no claim; one cross-cutting job keyed on the
            trigger and on stay/leave (the deadline, held modifiers, logging)
recorder    exclusive bind at the root: one field assigned, stay
timer       exclusive bind: one timer id matched, one consequence
```

A gesture split across schedule slots is the same mistake as two gestures in one handler, mirrored: foregrounding Chrome without entering in-app is not a behavior, exactly as Kill without opening the modifiers is a bug. The schedule composes concerns with gestures; `and` composes a gesture from its units; nothing composes in `Mercury::handle` after the fact.

```text
#[bind(T => h)]      =>  exclusive(h)     claims
#[post(T => h)]      =>  h                no claim; runs if T matched
#[pre_post(T => (pre, post))]             snap before descent; post on ascent
```

```rust
FnOnce(&Ev, Snap, AscendState<'a, P>) -> (Vec<E>, Completed<P>)
```

Leave is data (`Invalidated`); every later item still runs and sees it. That is why a gesture and the deadline can be independent: the gesture peels, the deadline post matches `Invalidated`.

What a unit touches decides its reach, and its reach decides its shape:

```text
pure effect              (vec![eff], st.complete())                  branch-free, any state
its own node             match: NotInvalidated => get_mut(); Invalidated => forward `c`
the root, ending there   st.state.into_ancestor::<MercuryPath>(),    branch-free
                         mutate, root.complete()
a mid-level ancestor     st.state.try_into_ancestor::<..>():         Ok ends at the target,
                                                                     Err forwards the leave
```

A unit that writes the root ends there; there is deliberately no way to mutate an ancestor from a standing path. For `mark_navigating` and the place units the ending is truthful: their gestures end at the root anyway (`enter_inapp`, `go_home` complete the `and`), so `Invalidated` after them means what it says, and two root-enders compose, since the state-level `into_ancestor` is total on both branches. The overlay toggle stops being a counterexample by binding where its state lives (below): the root ending it hands back is its own completion, invalidating nothing beneath it. After that, every root-ender's gesture genuinely ends at the root, and `Invalidated` never lies.

Mutation methods on the root (`set_layer`, `placing`, `hide_overlay`) stay methods: each is one state write and returns the effects that write implies. A gesture calls `set_layer` at most once; calling it twice is two gestures.

## Prefactor: `and`

The schedule's fold, at expression level: run `a`, fold its leave back into the state, run `b` with what `a` left behind, effects concatenated in call order. One claim serves the whole composition (`#[bind]` wraps the outermost expression in `exclusive`, so the gesture claims once and units never claim). It lives in `bind` beside `exclusive`, because it destructures `AscendState`'s private claim; it nests (`and(a, and(b, c))`); and it needs only landed items (`AscendState`, `Claim::reborrow`, `Completed::to_maybe_invalidated`), so it ships now, ahead of everything below.

```rust
/// Runs `a` then `b` as one handler: one claim, effects in order, `b`
/// receiving the state `a` left behind. A gesture composes from units at its
/// bind site: `#[bind(K => and!(tap_cmd_l, enter_typing))]`.
pub fn and<Ev, Snap, P, E, A, B>(
    a: A,
    b: B,
) -> impl for<'x> FnOnce(Ev, Snap, AscendState<'x, P>) -> (Vec<E>, ::laserbeam::Completed<P>)
where
    Ev: Copy,
    Snap: Copy,
    P: ::laserbeam::HasStop,
    A: for<'x> FnOnce(Ev, Snap, AscendState<'x, P>) -> (Vec<E>, ::laserbeam::Completed<P>),
    B: for<'x> FnOnce(Ev, Snap, AscendState<'x, P>) -> (Vec<E>, ::laserbeam::Completed<P>),
{
    move |ev, snap, st| {
        let AscendState { mut claim, state } = st;
        let (mut effs, completed) = a(ev, snap, AscendState::new(state, claim.reborrow()));
        let state = completed.to_maybe_invalidated();
        let (e, completed) = b(ev, snap, AscendState::new(state, claim));
        effs.extend(e);
        (effs, completed)
    }
}
```

Both units receive the same event and the same snap (hence the `Copy` bounds; in bind position the snap is `()`).

The flat form is a macro over the same fn — it expands to the nested calls, so closures and generic units survive and nothing goes dynamic (a slice would force one element type: fn-pointer coercion breaks a parameterized unit like `tmux_window(1)`, and `&dyn` buys vtables and per-element `&`):

```rust
/// `and!(a, b, c)` is `and(a, and(b, c))`.
#[macro_export]
macro_rules! and {
    ($h:expr) => { $h };
    ($h:expr, $($rest:expr),+ $(,)?) => {
        $crate::and($h, $crate::and!($($rest),+))
    };
}
```

Tests, in `crates/bind/tests` on the existing demo tree, landing with the prefactor:

- `and_concatenates_effects_in_order`: two effect-only units, the pair's effects in call order, one claim taken.
- `the_second_unit_sees_the_firsts_leave`: `a` leaves, `b` receives `Invalidated` and forwards it; the dispatch's fold re-establishes the parent.
- `and_nests`: `and!(a, b, c)` runs all three in order, identically to the hand-nested form.

## Downstream

```text
and (prefactor above)        ships now: bind addition + tests
timed-layer-wrapper.md       step 2's tree restructure (in past; its pre/post mechanics superseded)
invalidation-granularity.md  the general hole (field-granular writes); no longer gates anything here
multiple-children.md         needs posts-run-regardless
also-binds / handler-kinds / exclusive-as-post   history; schedule + and replace them
```

---

## Units, and the gestures they compose

### Leave / enter layer

```text
go_home            set_layer(Home); leave
enter_nav          set_layer(Nav); leave     // Nav::new arms its timer inside construction;
enter_resize       set_layer(Resize); leave  // arm effect is returned by new and is part of
enter_typing       set_layer(Typing); leave  // that one mutation, not a second handler job
enter_inapp        set_layer(InApp); leave
enter_site         set_layer(Site); leave
```

`set_layer` is the one mutation: hide overlay, reset jk, open/close modifiers, `ShowLayer`. Each takes the root-consuming shape: `st.state.into_ancestor::<MercuryPath>()`, one `set_layer`, `root.complete()`.

### Units that emit

```text
tap_cmd_space      Tap(space, COMMAND)
tap_cmd_l          Tap(l, COMMAND)
tap_cmd_r          Tap(r, COMMAND)          // Chrome refresh
tap_cmd_shift_o    Tap(o, COMMAND|SHIFT)    // claude.ai new chat
tmux_prev          Tap(ctrl-a), Tap(p)
tmux_next          Tap(ctrl-a), Tap(n)
tmux_window(N)     Tap(ctrl-a), Tap(shifted digit)
```

A tap unit never calls `set_layer`. Chrome refresh is `tap_cmd_r` alone; walking tmux with j/k is `tmux_prev` / `tmux_next` alone (repeatable, stays).

### The root-writing units

`mark_navigating` mutates `foreground`, which lives on the root, so it consumes to the root and completes there; its gesture leaves anyway, so the ending is truthful. `foreground_chrome` is effect-only and runs on any state.

```rust
fn mark_navigating<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    // one job: the watcher has not confirmed the new front app yet
    let root: MercuryPath<'a> = st.state.into_ancestor();
    root.foreground.start_navigating();
    (vec![], root.complete())
}

fn foreground_chrome<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (vec![MercuryEffect::Foreground(App::Chrome)], st.complete())
}
```

`left_half` / `right_half` / `maximize` / `restore_window` each only touch `windows`, on the root: the same root-ending shape, with `go_home` composing after them in the `and(..)` — a second root-ender, which works because the state-level `into_ancestor` is total on `Invalidated` too.

### The gestures

Each multi-part gesture is one bind whose rhs is an `and` of its units. Effects land in call order, so a tap precedes the transition's flush inside the one bind (Spotlight wants the modifier downs from typing's open to land after the spotlight chord):

```text
// Nav app keys — the whole gesture, one claim
#[bind(Key::KeyC.down() => and!(mark_navigating, foreground_chrome, enter_inapp))]

// emit then type
#[bind(Key::KeyL.down().bare() => and!(tap_cmd_l, enter_typing))]      // Chrome l
#[bind(Key::KeyN.down() => and!(tap_cmd_shift_o, enter_typing))]      // claude.ai n
#[bind(Key::Space.down() => and!(tap_cmd_space, enter_typing))]       // Nav space

// place, then the choice is made, so home
#[bind(Key::LeftArrow.down() => and!(left_half, go_home))]            // Resize
#[bind(Key::Num1.down() => and!(tmux_window_1, go_home))]             // Ghostty digits
```

### Return-home deadline: one owner, above the layers

The tree restructure is `refactors/past/timed-layer-wrapper.md`'s: the four timed layers regroup under one wrapper node that owns the one guard, and the deadline sits on it. That doc's pre/post mechanics predate the landed model and are superseded by what follows.

```rust
pub enum Layer {
    Home(HomeLayer),
    Typing(TypingLayer),
    ReturnHome(AndReturnHome),
}

pub struct AndReturnHome {
    #[resolve_into]
    layers: ReturnHomeLayers,
    guard: TimerGuard,
}

pub enum ReturnHomeLayers {
    Nav(NavLayer),
    Resize(ResizeLayer),
    InApp(AppLayer),
    Site(SiteLayer),
}
```

The wrapper carries the one deadline post and the one firing bind; the four leaves lose their `home_timeout` fields, their arming, and their firing closures; an untimed layer is unrepresentable in the deadline's domain. Placement above the leaves is what makes the deadline correct: a leaf's own `go_home` claim happens inside the wrapper's descent, so the post sees the leave and does nothing; scheduled on a leaf it would run before that leaf's binds and rearm a layer about to die.

The deadline is a plain `#[post]`, with no pre, because nothing consumes the old timer id: mercury cancels a timer by dropping its guard into freddie's cancel channel, so on a stay the overwrite is the cancel, and on a leave `set_layer`'s swap already dropped the wrapper and its guard. Every mercury timer cancels this way; the deadline is not special. (The A/B demo in `refactors/past/invalidation.md` keeps its `pre_post`: it demonstrates the general mechanism in a world whose cancel is an effect; mercury's is not, and `MercuryEffect` gains no `CancelTimer`.)

```text
// on AndReturnHome, the one site (AndReturnHomePath<'a> = PathMut<AndReturnHome, LayerPath<'a>>)
#[post(AnyKey => home_deadline)]
#[bind(|p| p.get().guard.trigger() => go_home)]
```

```rust
fn home_deadline<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, AndReturnHomePath<'x>>,
) -> (Vec<MercuryEffect>, Completed<AndReturnHomePath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(mut p) => {
            let (guard, arm) = arm_return_home();
            p.get_mut().guard = guard;
            (vec![arm], p.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}
```

`Mercury::handle`'s rearm and `Layer::rearm_timeout` are both deleted. The shared exit keys (escape, t) staying on the leaves versus lifting onto the wrapper is the follow-up timed-layer-wrapper.md names, as is grouping `Home`/`Typing`.

### Overlay

`o` binds once, at the root, whose own field `overlay` is; the five per-layer o binds are deleted. The typing gate rides the trigger, preserving today's behavior (o in typing types an o):

```rust
// on Mercury (root), scheduled before the AnyKey pair
#[bind(
    |m| (!matches!(m.layer, Layer::Typing(_))).then(|| Key::KeyO.down()) => toggle_overlay,
)]
```

```rust
fn toggle_overlay<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    let effs = root.toggle_overlay();
    (effs, root.complete())
}
```

This is an own-node write. The root owns `overlay`, reads the layer beneath it for the content (`Mercury::toggle_overlay` already does exactly that), and hands back its own completion, so nothing below is invalidated. The deadline post ran earlier, during the ascent below, and read a true stay; an o-press counts as activity and pushes the deadline out. Dwell fire is only `hide_overlay`, the root timer bind it already is. Layer-change hide stays inside `set_layer` (that mutation's implied effect), not a second handler.

An overlay that itself binds keys while open is not foreclosed: such an overlay is a child of the root beside the layer, which is `multiple-children.md`'s territory whichever node the toggle binds on. The root then resolves into two children, the overlay's own keys bind on the overlay node, and `o` stays the root's toggle — today's flat root is that shape with the second child not yet a node.

### Root recorders

```text
record_front_app   foreground.set_front_app; stay
record_tab_url     foreground.set_tab_url; stay
record_windows     windows.record; stay
```

Each is already one thing. Stay exclusive binds.

### Root timers

```text
jk_timeout         replay(jk.interrupt()); stay
placement_settled  windows.forget_pending(); stay
hide_overlay       (dwell) as above
```

### Quit

One gesture: `held.open()` then Kill, one handler. An open without Kill is not a behavior, and Kill without open is a bug; whether the body is a single fn or `and!(open_held, kill)` is immaterial, since the claim and the key are one either way.

### Root AnyKey (passthrough)

Today `maybe_pass_through` does four jobs, and modifier tracking is a genuine cross-cutting concern: it must run for keys a deeper layer claimed (a modifier pressed in nav). That is exactly a post; the claim stays on the passthrough policy.

```text
// root — AnyKey; source order; only the claiming bind is exclusive
#[post(AnyKey => track_held_modifiers)]     // held.apply if modifier; always; no claim
#[bind(AnyKey => pass_or_swallow)]          // claim; passthrough: jk + emit; command: empty
```

```rust
fn track_held_modifiers<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    if ev.key.is_modifier() {
        root.typing_state.held.apply(ev);
    }
    (vec![], root.complete())
}
```

(Named lifetime, because the elided return with two input lifetimes does not resolve; branch-free, because a leave from the root's own descent still holds the root and `into_ancestor` on the state is total there.)

`pass_or_swallow` still owns jk advance, emit, and enter home: three outcomes of one policy ("what does an unbound key do in this layer"), one handler.

---

## Fat handlers today → after

```text
to_home                         go_home
to_nav / to_resize / ...        enter_*
to_typing                       enter_typing
open_chrome (etc.)              and!(mark_navigating, foreground_chrome, enter_inapp)
open_spotlight                  and!(tap_cmd_space, enter_typing)
focus_address_bar               and!(tap_cmd_l, enter_typing)
new_chat                        and!(tap_cmd_shift_o, enter_typing)
refresh                         tap_cmd_r
previous_window / next_window   tmux_prev / tmux_next
window_N                        and!(tmux_window(N), go_home)
maximize / left_half / ...      and!(place unit, go_home)
restore_window                  and!(restore unit, go_home)
and_go_home / and_go_home_from  deleted; and!(unit, go_home) at the bind site
maybe_pass_through              track_held_modifiers (post) + pass_or_swallow (bind)
Mercury::handle rearm           the AndReturnHome home_deadline post
Layer::rearm_timeout            deleted (timed-layer-wrapper.md)
NavLayer.home_timeout (x4)      deleted; AndReturnHome owns the one guard
set_layer                       stays (one mutation); a gesture calls it once
```

---

## Schedule sketches

### AndReturnHome (the one deadline site)

```rust
#[derive(Bind)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[post(AnyKey => home_deadline)]
#[bind(|p| p.get().guard.trigger() => go_home)]
pub struct AndReturnHome {
    #[resolve_into]
    layers: ReturnHomeLayers,
    guard: TimerGuard,
}
```

### Nav (a leaf under it; no timer field, no firing closure)

```rust
#[derive(Bind)]
#[node(parent = ReturnHomeLayersPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => go_home,
    Key::KeyT.down() => enter_typing,
    Key::KeyC.down() => and!(mark_navigating, foreground_chrome, enter_inapp),
    Key::KeyF.down() => and!(mark_navigating, foreground_finder, enter_inapp),
    Key::KeyG.down() => and!(mark_navigating, foreground_ghostty, enter_inapp),
    Key::KeyZ.down() => and!(mark_navigating, foreground_zed, enter_inapp),
    Key::Space.down() => and!(tap_cmd_space, enter_typing),
)]
struct NavLayer;
```

### Chrome derived leaf

```rust
#[bind(Key::KeyR.down() => tap_cmd_r)]
#[bind(Key::KeyL.down().bare() => and!(tap_cmd_l, enter_typing))]
#[bind(Key::KeyL.down().with(SHIFT) => copy_url)]
#[bind(Key::KeyL.down().with(COMMAND) => copy_host)]
```

### Resize

```rust
#[bind(Key::LeftArrow.down() => and!(left_half, go_home))]
// same for right, up, r(restore), ...
```

### Root

```rust
#[bind(
    Foregrounded => record_front_app,
    Tabbed => record_tab_url,
    Windowed => record_windows,
    Quit => quit,
    |p| p.typing_state.jk.window_timer().map(TimerGuard::trigger) => jk_timeout,
    |p| p.overlay_timer().map(TimerGuard::trigger) => hide_overlay,
    |p| p.windows.pending_timer().map(TimerGuard::trigger) => placement_settled,
)]
#[bind(|m| (!matches!(m.layer, Layer::Typing(_))).then(|| Key::KeyO.down()) => toggle_overlay)]
#[post(AnyKey => track_held_modifiers)]
#[bind(AnyKey => pass_or_swallow)]
```

`handle` is only dispatch:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> (Vec<MercuryEffect>, bool) {
    bind::dispatch::<MercuryStruct, Self, _>(self, event)
}
```

---

## What is not a separate handler

```text
a gesture's steps
    units composed by and(..) at the bind site: one claim, one schedule slot

set_layer's overlay hide / jk reset / open-close / ShowLayer
    one mutation's implied effects; not scheduled items

TimerGuard Drop cancel
    the cancel: a dropped guard cancels through freddie's cancel channel, so
    the rearm's overwrite and set_layer's swap both cancel by dropping, and
    no CancelTimer effect exists

app_data / site_data
    resolve inputs, not handlers

windows.placing / restoring
    state methods; the place unit calls one of them
```

---

## Order of work

```text
0. Prefactor, ships now: and in bind, with its three tests
1. gesture binds via and: delete and_go_home / and_go_home_from; open_*,
   focus_address_bar / new_chat / spotlight, window_N, and the place keys
   become and(..) binds of their units
2. the AndReturnHome restructure per timed-layer-wrapper.md: the wrapper node
   with the one guard and the home_deadline post; the four timer fields,
   arming sites, and firing closures deleted; the Mercury::handle rearm and
   Layer::rearm_timeout deleted
3. track_held_modifiers as root post; slim pass_or_swallow
4. multiple-children when designed
```

The change-5 migration already landed the signatures; fat handlers keep their bodies until step 1.

The acceptance test for the whole migration: every multi-part gesture in `crates/mercury/src/handlers/` is one `and(..)` bind and no gesture is split across schedule slots; exactly one `TimerGuard` for return-home exists, on `AndReturnHome`, and no rearm exists outside its `home_deadline` post; `and_go_home` does not exist; `Mercury::handle` is only dispatch.
