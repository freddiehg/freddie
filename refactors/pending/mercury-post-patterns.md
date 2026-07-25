# Mercury: one gesture per bind, one concern per post

Once `completed-ancestors.md` (which ships first) and `invalidation.md` finish (change 5: linear body + signature migration + `#[post]` / `#[pre_post]` parsing + derived levels per `derived-levels.md`; change 6: demo + walks), every mercury behavior is a scheduled item. The state-level `into_ancestor` / `try_into_ancestor` from completed-ancestors are assumed throughout: they are what let a unit that touches the root be branch-free.

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
an ancestor, staying     match: NotInvalidated => p.ancestor_mut::<MercuryPath>(),
                         mutate, p.complete()                        (ancestor_mut prefactor)
the root, ending there   st.state.into_ancestor::<MercuryPath>(),    branch-free
                         one set_layer, root.complete()              (completed-ancestors)
a mid-level ancestor     st.state.try_into_ancestor::<..>():         Ok ends at the target,
                                                                     Err forwards the leave
```

A unit that mutates root state but does not end the layer — the overlay family, `mark_navigating`, the place units — reaches up with `ancestor_mut` and hands back `Here`, so the state stays truthful and the deadline post reads a real stay. The root-ending shape belongs to exactly the `set_layer` units: leaving the layer and ending at the root are the same event, so only they flip the state to `Invalidated`, and a later unit seeing `Invalidated` is seeing a genuine layer change. (The change-5 migration shipped `toggle_overlay` in the root-ending shape for want of `ancestor_mut`; step 2 re-shapes it.)

Mutation methods on the root (`set_layer`, `placing`, `hide_overlay`) stay methods: each is one state write and returns the effects that write implies. A gesture calls `set_layer` at most once; calling it twice is two gestures.

## Prefactor: `and`

The schedule's fold, at expression level: run `a`, fold its leave back into the state, run `b` with what `a` left behind, effects concatenated in call order. One claim serves the whole composition (`#[bind]` wraps the outermost expression in `exclusive`, so the gesture claims once and units never claim). It lives in `bind` beside `exclusive`, because it destructures `AscendState`'s private claim; it nests (`and(a, and(b, c))`); and it needs only landed items (`AscendState`, `Claim::reborrow`, `Completed::to_maybe_invalidated`), so it ships now, ahead of everything below.

```rust
/// Runs `a` then `b` as one handler: one claim, effects in order, `b`
/// receiving the state `a` left behind. A gesture composes from units at its
/// bind site: `#[bind(K => and(tap_cmd_l, enter_typing))]`.
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

Both units receive the same event and the same snap (hence the `Copy` bounds; in bind position the snap is `()`). Tests, in `crates/bind/tests` on the existing demo tree, landing with the prefactor:

- `and_concatenates_effects_in_order`: two effect-only units, the pair's effects in call order, one claim taken.
- `the_second_unit_sees_the_firsts_leave`: `a` leaves, `b` receives `Invalidated` and forwards it; the dispatch's fold re-establishes the parent.
- `and_nests`: `and(a, and(b, c))` runs all three in order.

## Prefactor: `ancestor_mut`

The mutable mirror of `HasAncestor`, in laserbeam. A path re-projects on every access and holds no live borrow of its node, so a mutable reach to an ancestor is a chain of field reborrows: the path is frozen while the borrow lives and whole again after. This is what lets a unit mutate the root and still hand back `Here` — without it, `toggle_overlay` ends at the root and reports a leave it did not make, and the deadline post cancels a timer for a layer the user is still in.

```rust
/// Walk up a path to an ancestor by mutable reference, keeping the path.
///
/// The mutable mirror of [`HasAncestor`]: the path re-projects on every
/// access, so this is a field reborrow, and the path is usable again when the
/// borrow ends. It is how a handler mutates an ancestor and stays.
pub trait HasAncestorMut<Target>: HasAncestor<Target> {
    fn ancestor_mut(&mut self) -> &mut Target;
}

/// Every path is its own ancestor, at depth zero.
impl<T> HasAncestorMut<T> for T {
    fn ancestor_mut(&mut self) -> &mut T {
        self
    }
}
```

`PathMut` gains the one-level building block beside `parent`:

```rust
    /// The parent, mutably. No borrow of the node is held, so the path is
    /// whole again when this borrow ends.
    pub const fn parent_mut(&mut self) -> &mut Parent {
        &mut self.parent
    }
```

The chain macro and the third arm of `ancestor_impls!`, mirroring the two that exist:

```rust
/// One `parent_mut()` per type parameter, the mutable mirror of `parent_chain!`.
macro_rules! parent_mut_chain {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        parent_mut_chain!($e.parent_mut() $(, $rest)*)
    };
}

// inside ancestor_impls!, beside the HasAncestor and IntoAncestor arms:
        impl<T, $($acc,)* $head> HasAncestorMut<T> for path_nest!(T $(, $acc)*, $head) {
            fn ancestor_mut(&mut self) -> &mut T {
                parent_mut_chain!(self $(, $acc)*, $head)
            }
        }
```

And the sugar, in `PathMut`'s existing inherent block beside `ancestor` / `into_ancestor`:

```rust
    /// Walk up to `Target` by mutable reference, keeping the path, naming the
    /// target rather than leaving it to inference.
    #[must_use]
    pub fn ancestor_mut<Target>(&mut self) -> &mut Target
    where
        Self: HasAncestorMut<Target>,
    {
        HasAncestorMut::ancestor_mut(self)
    }
```

Tests, beside `ancestor_tests` on its fixture: `ancestor_mut_writes_at_each_depth` (write the root's `hits` through paths at depths zero, one, and two, and use the path again after each borrow ends), and a generic fn bounded on `HasAncestorMut<AppPath>` instantiated at every depth. Ships now, independent of everything else here.

## Downstream

```text
and (prefactor above)           ships now: bind addition + tests
ancestor_mut (prefactor above)  ships now: laserbeam addition + tests
completed-ancestors.md  ships before invalidation change 5
invalidation change 5   Completed body; handler signature; #[post]/#[pre_post]; derived levels (derived-levels.md)
invalidation change 6   demo tree + full walks
timed-layer-wrapper.md  the deadline's design (in past; revived at step 2)
multiple-children.md    needs posts-run-regardless
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

### The staying root-mutators

`mark_navigating` mutates `foreground`, which lives on the root, but the mutation is not a leave, so it reaches up with `ancestor_mut` and hands back `Here`; `foreground_chrome` is effect-only and runs on any state.

```rust
fn mark_navigating<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop + Complete<P> + HasAncestorMut<MercuryPath<'a>>,
{
    match st.state {
        MaybeInvalidated::NotInvalidated(mut p) => {
            // one job: the watcher has not confirmed the new front app yet
            p.ancestor_mut().foreground.start_navigating();
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
    (vec![MercuryEffect::Foreground(App::Chrome)], st.complete())
}
```

`left_half` / `right_half` / `maximize` / `restore_window` each only touch `windows`, on the root, and none of them is a leave: the same staying shape, with `go_home` doing the leaving beside them in the `and(..)`.

### The gestures

Each multi-part gesture is one bind whose rhs is an `and` of its units. Effects land in call order, so a tap precedes the transition's flush inside the one bind (Spotlight wants the modifier downs from typing's open to land after the spotlight chord):

```text
// Nav app keys — the whole gesture, one claim
#[bind(Key::KeyC.down() => and(mark_navigating, and(foreground_chrome, enter_inapp)))]

// emit then type
#[bind(Key::KeyL.down().bare() => and(tap_cmd_l, enter_typing))]      // Chrome l
#[bind(Key::KeyN.down() => and(tap_cmd_shift_o, enter_typing))]      // claude.ai n
#[bind(Key::Space.down() => and(tap_cmd_space, enter_typing))]       // Nav space

// place, then the choice is made, so home
#[bind(Key::LeftArrow.down() => and(left_half, go_home))]            // Resize
#[bind(Key::Num1.down() => and(tmux_window_1, go_home))]             // Ghostty digits
```

### Return-home deadline: one owner, above the layers

The design is `refactors/past/timed-layer-wrapper.md`, revived by this step: the four timed layers regroup under one wrapper node that owns the one guard, and the deadline `pre_post` sits on it.

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

The wrapper carries the one `pre_post` and the one firing bind; the four leaves lose their `home_timeout` fields, their arming, and their firing closures; an untimed layer is unrepresentable in the deadline's domain, so there is no `Option` snap and no accessor. Placement above the leaves is also what makes the deadline correct: a leaf's own `go_home` claim happens inside the wrapper's descent, so the post sees the leave as `Invalidated` and cancels; scheduled on a leaf it would run before that leaf's binds and rearm a layer about to die, with the cancel lost to the guard's `Drop`.

```text
// on AndReturnHome, the one site
#[pre_post(AnyKey => (snap_home_timeout, home_deadline))]
#[bind(|p| p.get().guard.trigger() => go_home)]
// pre:  |_, p| p.get().guard.id
// post: NotInvalidated => cancel old, arm fresh, rewrite p.get_mut().guard
//       Invalidated    => cancel old
```

`Mercury::handle`'s rearm and `Layer::rearm_timeout` are both deleted. The shared exit keys (escape, o, t) staying on the leaves versus lifting onto the wrapper is the follow-up that doc names, as is grouping `Home`/`Typing`.

### Overlay

```text
show_overlay       ShowOverlay + arm dwell; stay
hide_overlay       HideOverlay; clear guard; stay
toggle_overlay     one gesture; one handler that branches on overlay.is_some()
                   is still one job: toggle
```

All three mutate overlay state on the root and none is a leave, so all three take the staying `ancestor_mut` shape. Written out for the one the migration got wrong:

```rust
fn toggle_overlay<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop + Complete<P> + HasAncestorMut<MercuryPath<'a>>,
{
    match st.state {
        MaybeInvalidated::NotInvalidated(mut p) => {
            let effs = p.ancestor_mut().toggle_overlay();
            (effs, p.complete())
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}
```

It hands back `Here`, so the deadline post on `AndReturnHome` reads a stay and pushes the deadline out, which is what pressing `o` in a timed layer should do. Dwell fire is only `hide_overlay`. Layer-change hide stays inside `set_layer` (that mutation's implied effect), not a second handler.

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

One gesture: `held.open()` then Kill, one handler. An open without Kill is not a behavior, and Kill without open is a bug; whether the body is a single fn or `and(open_held, kill)` is immaterial, since the claim and the key are one either way.

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
open_chrome (etc.)              and(mark_navigating, and(foreground_chrome, enter_inapp))
open_spotlight                  and(tap_cmd_space, enter_typing)
focus_address_bar               and(tap_cmd_l, enter_typing)
new_chat                        and(tap_cmd_shift_o, enter_typing)
refresh                         tap_cmd_r
previous_window / next_window   tmux_prev / tmux_next
window_N                        and(tmux_window(N), go_home)
maximize / left_half / ...      and(place unit, go_home)
restore_window                  and(restore unit, go_home)
and_go_home / and_go_home_from  deleted; and(unit, go_home) at the bind site
maybe_pass_through              track_held_modifiers (post) + pass_or_swallow (bind)
Mercury::handle rearm           the AndReturnHome home_deadline pre_post
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
#[pre_post(AnyKey => (snap_home_timeout, home_deadline))]
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
    Key::KeyO.down() => toggle_overlay,
    Key::KeyT.down() => enter_typing,
    Key::KeyC.down() => and(mark_navigating, and(foreground_chrome, enter_inapp)),
    Key::KeyF.down() => and(mark_navigating, and(foreground_finder, enter_inapp)),
    Key::KeyG.down() => and(mark_navigating, and(foreground_ghostty, enter_inapp)),
    Key::KeyZ.down() => and(mark_navigating, and(foreground_zed, enter_inapp)),
    Key::Space.down() => and(tap_cmd_space, enter_typing),
)]
struct NavLayer;
```

### Chrome derived leaf

```rust
#[bind(Key::KeyR.down() => tap_cmd_r)]
#[bind(Key::KeyL.down().bare() => and(tap_cmd_l, enter_typing))]
#[bind(Key::KeyL.down().with(SHIFT) => copy_url)]
#[bind(Key::KeyL.down().with(COMMAND) => copy_host)]
```

### Resize

```rust
#[bind(Key::LeftArrow.down() => and(left_half, go_home))]
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
a gesture's steps
    units composed by and(..) at the bind site: one claim, one schedule slot

set_layer's overlay hide / jk reset / open-close / ShowLayer
    one mutation's implied effects; not scheduled items

TimerGuard Drop cancel
    OS cancel; the home_deadline pre_post is the explicit cancel-on-leave form
    for the idle timer (Drop alone cannot push CancelTimer into the batch)

app_data / site_data
    resolve inputs, not handlers

windows.placing / restoring
    state methods; the place unit calls one of them
```

---

## Order of work

```text
0. Prefactors, ship now: and in bind; ancestor_mut in laserbeam; each with
   its tests, each its own commit
1. gesture binds via and: delete and_go_home / and_go_home_from; open_*,
   focus_address_bar / new_chat / spotlight, window_N, and the place keys
   become and(..) binds of their units
2. the AndReturnHome restructure per timed-layer-wrapper.md: the wrapper node
   with the one guard and the home_deadline pre_post; the four timer fields,
   arming sites, and firing closures deleted; the Mercury::handle rearm and
   Layer::rearm_timeout deleted; the staying root-mutators (toggle_overlay,
   show/hide_overlay, mark_navigating, the place units) re-shaped onto
   ancestor_mut so the deadline reads true stay/leave
3. track_held_modifiers as root post; slim pass_or_swallow
4. multiple-children when designed
```

The change-5 migration already landed the signatures; fat handlers keep their bodies until step 1.

The acceptance test for the whole migration: every multi-part gesture in `crates/mercury/src/handlers/` is one `and(..)` bind and no gesture is split across schedule slots; exactly one `TimerGuard` for return-home exists, on `AndReturnHome`, and no rearm exists outside its `pre_post`; `and_go_home` does not exist; `Mercury::handle` is only dispatch.
