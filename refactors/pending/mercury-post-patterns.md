# Mercury: one thing per handler

Once `invalidation.md` finishes (change 5: linear body + signature migration; change 6: `#[post]` / `#[pre_post]`), every mercury behavior is a scheduled item. The schedule is how composition works: several small handlers on one trigger, not one fat handler that does several jobs.

## Rule

A scheduled handler does exactly one thing. One of:

```text
claim and act          exclusive bind: deepest wins; one user-facing action or one leave
observe stay/leave     post / pre_post: no claim; one concern keyed on MaybeInvalidated
record external truth  exclusive bind: one field assigned, stay
fire a timer           exclusive bind: one timer id matched, one consequence
```

Composition is source order on the node (and posts on ancestors). Not a helper that chains two mutations. Not a handler that emits a chord and also changes layer. Not `Mercury::handle` after the fact.

```text
#[bind(T => h)]      =>  exclusive(h)     claims
#[post(T => h)]      =>  h                no claim; runs if T matched
#[pre_post(T => (pre, post))]             snap before descent; post on ascent
```

```rust
FnOnce(&Ev, Snap, AscendState<'a, P>) -> (Vec<E>, Completed<P>)
```

Leave is data (`Invalidated`); every later item still runs and sees it. That is why a leave and a cancel can be two handlers: the leave peels, the cancel post matches `Invalidated`.

Mutation methods on the root (`set_layer`, `placing`, `hide_overlay`) stay methods: each is one state write and returns the effects that write implies. Handlers call at most one of them. A handler that calls `set_layer` and also builds a `Foreground` effect, or places and also goes home, is two things.

## Downstream

```text
invalidation change 5   Completed body; handler signature; derived levels
invalidation change 6   #[post] / #[pre_post]
timed-layer-wrapper.md  one timer owner; one pre_post; leaves the four copies
multiple-children.md    needs posts-run-regardless
also-binds / handler-kinds / exclusive-as-post   history; schedule replaces them
```

---

## Unit handlers mercury needs

Each unit is one function, one schedule slot, one job. Today's fat handlers are listed under "compose from".

### Leave / enter layer

```text
go_home            set_layer(Home); leave
enter_nav          set_layer(Nav); leave     // Nav::new arms its timer inside construction;
enter_resize       set_layer(Resize); leave  // arm effect is returned by new and is part of
enter_typing       set_layer(Typing); leave  // that one mutation, not a second handler job
enter_inapp        set_layer(InApp); leave
enter_site         set_layer(Site); leave
```

`set_layer` is the one mutation: hide overlay, reset jk, open/close modifiers, `ShowLayer`. The handler's one job is "enter this layer" (or home). It does not also foreground an app or emit a key.

### Foreground

Nav's app keys today do navigating + enter inapp + Foreground in one body. Three units, one key. Posts that need the path stay put and are scheduled before the leave bind.

```rust
fn mark_navigating<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasAncestor<MercuryPath<'a>> + HasStop + Complete<P>,
{
    match st.state {
        MaybeInvalidated::NotInvalidated(p) => {
            // one job: watcher has not confirmed the new front app yet
            p.ancestor().foreground.start_navigating();
            (vec![], p.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}

fn foreground_chrome<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    // one job: effect only
    (vec![MercuryEffect::Foreground(App::Chrome)], st.complete())
}
```

```text
// Nav KeyC — posts before the leave bind
#[post(Key::KeyC.down() => mark_navigating, Key::KeyC.down() => foreground_chrome)]
#[bind(Key::KeyC.down() => enter_inapp)]
```

Same triple for F/G/Z with different `foreground_*`.

### Emit a chord / key

```text
tap_cmd_space      Tap(space, COMMAND)
tap_cmd_l          Tap(l, COMMAND)
tap_cmd_r          Tap(r, COMMAND)          // Chrome refresh
tap_cmd_shift_o    Tap(o, COMMAND|SHIFT)    // claude.ai new chat
tmux_prev          Tap(ctrl-a), Tap(p)
tmux_next          Tap(ctrl-a), Tap(n)
tmux_window(N)     Tap(ctrl-a), Tap(shifted digit)
```

A tap-only handler never calls `set_layer`. Chrome refresh is only `tap_cmd_r` + stay (exclusive, so it claims).

### Emit then enter typing

Today: one handler does both. Split on the same key:

```text
// Chrome bare l
#[post(Key::KeyL.down().bare() => tap_cmd_l)]
#[bind(Key::KeyL.down().bare() => enter_typing)]

// claude.ai n
#[post(Key::KeyN.down() => tap_cmd_shift_o)]
#[bind(Key::KeyN.down() => enter_typing)]

// Nav space
#[post(Key::Space.down() => tap_cmd_space)]
#[bind(Key::Space.down() => enter_typing)]
```

Post runs whether or not the bind claims; both match the same trigger. Exclusive only enters typing. Order: post first so the tap is in the batch before the leave's flush, matching today's "tap then transition" ordering (Spotlight wants modifier downs from typing's open to land after the spotlight chord).

### Place window then go home

Today: `place` + `and_go_home`. Split:

```text
// Resize LeftArrow
#[post(Key::LeftArrow.down() => left_half)]   // only SetFrame + pending timer; stay
#[bind(Key::LeftArrow.down() => go_home)]     // claim + leave home
```

`left_half` / `right_half` / `maximize` / `restore_window` each only touch `windows`. They do not leave. `go_home` only leaves. Same for Ghostty digits:

```text
#[post(Key::Num1.down() => tmux_window_1)]
#[bind(Key::Num1.down() => go_home)]
```

Walking tmux with j/k stays: exclusive that only emits, no go_home post.

### Return-home deadline

One pre_post, two arms, one concern ("the idle timer"):

```text
#[pre_post(AnyKey => (snap_return_home, return_home_deadline))]
// pre:  snap old TimerId
// post: NotInvalidated => cancel old + schedule fresh + rewrite guard
//       Invalidated    => cancel old
```

This is the A/B demo. It is not folded into `go_home` or into each layer key. It is not `Mercury::handle`.

Scheduled before exclusive binds on that node so the post sees the descent's stay/leave, not a later bind's.

### Overlay

```text
show_overlay       ShowOverlay + arm dwell; stay
hide_overlay       HideOverlay; clear guard; stay
toggle_overlay     // NOT a unit — compose at bind site or keep as one
                   // "toggle" is one user gesture; one handler that branches
                   // on overlay.is_some() is still one job: toggle
```

Dwell fire is only `hide_overlay`. Layer change hide stays inside `set_layer` (that mutation's implied effect), not a second handler.

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

Today: open modifiers + Kill. Split if both are effects of one user gesture "quit":

```text
// one job "quit the program": Kill is the act; opening modifiers is required
// cleanup so the OS is not left with stranded downs — same as set_layer's flush
// belonging to the mutation. Keep as one handler that: held.open() then Kill.
quit
```

Do not split open and Kill into two schedule slots: an open without Kill is not a behavior, and Kill without open is a bug. One handler, one gesture.

### Root AnyKey (passthrough)

Today `maybe_pass_through` does four jobs. Split by concern; claim stays on the passthrough policy:

```text
// root — AnyKey; source order; only the claiming bind is exclusive
#[post(AnyKey => track_held_modifiers)]     // held.apply if modifier; always; no claim
#[bind(AnyKey => pass_or_swallow)]          // claim; passthrough: jk + emit; command: empty
```

`track_held_modifiers` must run for keys a deeper layer claimed too (a modifier pressed in nav). That is exactly a post: no claim, runs on match, deepest claim still works. Today's bug-shaped coupling — tracking only on miss — goes away.

```rust
fn track_held_modifiers(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'_>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'_>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(root) => {
            if ev.key.is_modifier() {
                root.typing_state.held.apply(ev);
            }
            (vec![], root.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}
```

`pass_or_swallow` still owns jk advance, emit, and enter home on Completed — that is still three outcomes of one policy ("what does an unbound key do in this layer"). Further splitting jk into its own post is optional: the run is one state machine, one handler is fine.

```text
// optional further split later
#[post(AnyKey => track_held_modifiers)]
#[bind(AnyKey => advance_jk_or_emit)]   // only the passthrough policy
```

---

## Fat handlers today → units

```text
to_home                         go_home
to_nav / to_resize / ...        enter_*
to_typing                       enter_typing
open_chrome (etc.)              mark_navigating + foreground_chrome + enter_inapp
open_spotlight                  tap_cmd_space + enter_typing
focus_address_bar               tap_cmd_l + enter_typing
new_chat                        tap_cmd_shift_o + enter_typing
refresh                         tap_cmd_r
previous_window / next_window   tmux_prev / tmux_next
window_N                        tmux_window(N) + go_home
maximize / left_half / ...      place unit + go_home
restore_window                  restore unit + go_home
and_go_home / and_go_home_from  deleted; composition is the schedule
maybe_pass_through              track_held_modifiers + pass_or_swallow
Mercury::handle rearm           return_home_deadline pre_post
Layer::rearm_timeout            deleted
set_layer                       stays (one mutation); handlers call it once
```

---

## Schedule sketches

### Timed layer (nav), after units

Source order is the whole attribute list. pre_post first so the deadline sees the descent. Pure-effect and mark posts for a key sit before that key's leave bind so they still see `NotInvalidated` when they need the path. Pure-effect posts may also sit after a leave (they use `st.complete()` and ignore the path).

```rust
#[derive(Bind)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[pre_post(AnyKey => (snap_return_home, return_home_deadline))]
#[post(
    Key::KeyC.down() => mark_navigating,
    Key::KeyC.down() => foreground_chrome,
    Key::KeyF.down() => mark_navigating,
    Key::KeyF.down() => foreground_finder,
    Key::KeyG.down() => mark_navigating,
    Key::KeyG.down() => foreground_ghostty,
    Key::KeyZ.down() => mark_navigating,
    Key::KeyZ.down() => foreground_zed,
    Key::Space.down() => tap_cmd_space,
)]
#[bind(
    |p| p.get().home_timeout.trigger() => go_home,
    Key::Escape.down() => go_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyT.down() => enter_typing,
    Key::KeyC.down() => enter_inapp,
    Key::KeyF.down() => enter_inapp,
    Key::KeyG.down() => enter_inapp,
    Key::KeyZ.down() => enter_inapp,
    Key::Space.down() => enter_typing,
)]
struct NavLayer { home_timeout: TimerGuard }
```

### Chrome derived leaf

```rust
#[bind(Key::KeyR.down() => tap_cmd_r)]
#[post(Key::KeyL.down().bare() => tap_cmd_l)]
#[bind(Key::KeyL.down().bare() => enter_typing)]
#[bind(Key::KeyL.down().with(SHIFT) => copy_url)]
#[bind(Key::KeyL.down().with(COMMAND) => copy_host)]
```

### Resize

```rust
#[pre_post(AnyKey => (snap_return_home, return_home_deadline))]
#[post(Key::LeftArrow.down() => left_half)]
#[bind(Key::LeftArrow.down() => go_home)]
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
set_layer's overlay hide / jk reset / open-close / ShowLayer
    one mutation's implied effects; not scheduled items

TimerGuard Drop cancel
    OS cancel; the return-home pre_post is the explicit cancel-on-leave form
    for the idle timer (Drop alone cannot push CancelTimer into the batch)

app_data / site_data
    resolve inputs, not handlers

windows.placing / restoring
    state methods; the place unit handler calls one of them
```

---

## Order of work (mercury, after change 6)

```text
1. Unit handlers + schedule composition for leave/enter/tap/place
   (delete and_go_home; split open_*; split focus_address_bar / new_chat / spotlight)
2. return-home pre_post; delete handle rearm + Layer::rearm_timeout
3. track_held_modifiers as root post; slim pass_or_swallow
4. AndReturnHome wrapper (timed-layer-wrapper.md) — one pre_post site
5. multiple-children when designed
```

Step 1 does not need pre_post if every split is post+bind on the same key; posts need change 6. Until then, fat handlers stay, signatures only migrate under change 5.

The acceptance test for the whole migration: no handler in `crates/mercury/src/handlers/` both changes layer and emits a chord; no handler both places a window and goes home; no rearm outside a pre_post; `and_go_home` does not exist.
