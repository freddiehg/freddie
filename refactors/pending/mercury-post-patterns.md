# Mercury under pre / post / invalidation

Inventory of every behavior mercury has today, and which scheduled-handler shape owns it once `invalidation.md` is finished (change 5: linear `Completed` body + handler migration; change 6: `#[post]` / `#[pre_post]`). Not a redesign of mercury's product surface. The point is that nothing in mercury sits outside the schedule: every piece of work that currently lives in `Mercury::handle`, in a `Drop`, or as a side-effect of `set_layer` either becomes a scheduled item or is shown to be something else.

Patterns (from `invalidation.md`):

```text
#[bind(T => h)]      =>  #[pre_post(T => (|_, _| (), exclusive(h)))]   claims; deepest wins
#[post(T => h)]      =>  #[pre_post(T => (|_, _| (), h))]              no claim; always runs if T matched
#[pre_post(T => (pre, post))]                                          snap before descent; post on ascent
```

Every handler is the one signature:

```rust
FnOnce(&Ev, Snap, AscendState<'a, P>) -> (Vec<E>, Completed<P>)  // Snap = () without a pre
```

`MaybeInvalidated` is the post's input: stay is `NotInvalidated(path)`, leave is `Invalidated(c)`. Posts run whether or not anything claimed. A leave is data; nothing early-returns.

## Downstream of invalidation

Framework still to land (in `invalidation.md`, ordered):

```text
change 5   Dispatch returns Completed; linear scheduled body; route folds; derived levels; every handler migrates
change 6   #[post] / #[pre_post] parse and schedule
```

Change 5 is blocked on the "Derived levels" section of that doc (AppLayer / SiteLayer use `#[derived_child]` / `#[derived_node]`).

Consumers that unstick once change 6 exists:

```text
timed-layer-wrapper.md     return-home rearm is the A/B demo; wrapper owns one timer + one pre_post
also-binds.md              superseded; posts are the non-claiming schedule slot also-bind was for
handler-kinds.md           superseded by invalidation's schedule (kept only as history)
exclusive-as-post.md       already the desugaring invalidation uses (exclusive is a post gate)
multiple-children.md       two live #[resolve_into] fields; needs posts-run-regardless
drop-emits-effects.md      orthogonal (RAII for visible counterparts); not a post
timer-ids-on-root.md       orthogonal (ambient timer id source); arms still happen in posts
```

Mercury work that is pure migration (no product change), once change 5 ships:

```text
every handler signature   Node -> AscendState, return (Vec, Completed)
Mercury::handle rearm     delete; becomes the return-home pre_post
Layer::rearm_timeout      delete with it
```

## Tree and schedule today

```text
Mercury (root)
  foreground, windows, typing_state, overlay
  #[resolve_into] layer: Layer
    Home | Nav | Resize | Typing | InApp | Site
      InApp  --derived--> AppData::Chrome | Ghostty
      Site   --derived--> SiteData::ClaudeAi
```

Root binds (all exclusive today):

```rust
#[bind(
    Foregrounded => record_front_app,
    Tabbed => record_tab_url,
    Windowed => record_windows,
    Quit => quit,
    |p| p.typing_state.jk.window_timer().map(TimerGuard::trigger) => jk_timeout,
    |p| p.overlay_timer().map(TimerGuard::trigger) => hide_overlay,
    |p| p.windows.pending_timer().map(TimerGuard::trigger) => placement_settled,
    AnyKey => maybe_pass_through,
)]
```

Imperative, outside dispatch:

```rust
// Mercury::handle, after bind::dispatch
if handled && Key(_) && layer discriminant unchanged {
    effects.push(layer.rearm_timeout());  // Cancel+Schedule by replacing TimerGuard
}
```

That block is the first post target. Everything else already is a bind.

---

## 1. Return-home deadline

What. Nav, Resize, InApp, Site idle out after `RETURN_TO_HOME_TIMEOUT`. Activity that keeps you in the layer pushes the deadline out. Leaving cancels the OS timer (the guard drops).

Today.

```rust
// each timed layer
pub struct NavLayer { home_timeout: TimerGuard }
// arm in new(); bind firing to to_home
// Layer::rearm_timeout + Mercury::handle push a fresh schedule after a staying key
```

Target. One `#[pre_post]` on the node that owns the guard (the A/B demo in `invalidation.md`, or the wrapper in `timed-layer-wrapper.md`). Pre snaps the old id before descent; post matches the state:

```rust
#[pre_post(AnyKey => (snap_return_home, return_home_deadline))]
// NotInvalidated => Cancel(old) + Schedule(fresh), rewrite guard
// Invalidated    => Cancel(old) only
```

Scheduled before any exclusive bind on that node (source order), so the post sees the descent's answer, not a later bind's.

Where the guard lives is a separate decision: four copies (today) or one wrapper (`timed-layer-wrapper.md`). The schedule shape is the same either way.

What this deletes: `Layer::rearm_timeout`, the `before`/`after` discriminant check in `handle`, and (if the wrapper lands) the four timer fields and four firing binds.

---

## 2. Layer transitions that leave

What. Exclusive binds that replace `layer` and return home or enter another layer.

```text
Escape / idle fire                              to_home              -> Home
Home n / InApp n                                to_nav               -> Nav (+ arm timer)
Home r                                          to_resize            -> Resize (+ arm)
Home t / InApp t / Site t / Resize t / Nav t    to_typing            -> Typing
Home i                                          to_inapp             -> InApp (+ arm)
Home u / InApp s                                to_site              -> Site (+ arm)
Nav app keys                                    open_*               -> InApp (+ arm + Foreground + navigating)
Nav space                                       open_spotlight       -> Typing (+ cmd-space)
Resize arrows / r                               place / restore      -> Home (+ SetFrame)
Ghostty digits                                  window_N             -> Home (+ tmux taps)
Chrome bare l                                   focus_address_bar    -> Typing (+ cmd-l)
claude.ai n                                     new_chat             -> Typing (+ cmd-shift-o)
jk completed                                    maybe_pass_through   -> Home
```

Target. Still `#[bind]` (exclusive). Body leaves via `into_parent` / `set_layer` and returns `Completed`. The leave is what makes later posts on ancestors see `Invalidated`.

```rust
fn to_home<'x>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(path) => {
            let root = path.into_ancestor(); // or into_parent chain + set_layer
            let effects = go_home(root);
            // leave: the Completed that records the peel
            ...
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}
```

Exact leave spelling follows the change-5 handler migration; the point is exclusive + leave, not a post.

`and_go_home` / `and_go_home_from` stay as helpers that produce effects; the schedule slot is still the exclusive bind.

---

## 3. Layer transitions that stay

What. Actions that keep you in the active layer so they can be repeated.

```text
Chrome r          refresh          Tap(cmd-r)
Ghostty j/k       previous/next    tmux prefix + key
o (many layers)   toggle_overlay   Show/Hide overlay + dwell timer
```

Target. `#[bind]`, `st.complete()` on both arms (or only NotInvalidated if Invalidated is unreachable for a leaf that never leaves from these). Stay is what makes the return-home post rearm.

---

## 4. External truth, recorded at the root

What. Watchers and sockets push state mercury already holds a field for. Idempotent assign.

```text
Foregrounded   record_front_app     foreground.set_front_app
Tabbed         record_tab_url       foreground.set_tab_url
Windowed       record_windows       windows.record
```

Target. `#[bind]` at the root (exclusive: the event is "handled" so nothing else claims it). Bodies stay put: `st.complete()`, empty effects. No post needed; there is no leave/cancel story.

These never rearm return-home: they are not `Key` events, and the rearm post is keyed on `AnyKey` / keys only.

---

## 5. Root catch-all: modifiers, passthrough, jk

What. `AnyKey => maybe_pass_through` at the root, last resort after every layer miss.

```text
every key          held.apply if modifier
command layer      swallow (empty effects)
passthrough layer  feed jk run:
                     Advanced (opening)  arm jk window timer
                     Advanced            nothing
                     Passed              replay swallowed + emit this key
                     Completed           set_layer(Home)
```

Target. Still `#[bind(AnyKey => ...)]` exclusive. It is the claim that means "this key was handled by mercury" for pass-through policy (free `dispatch` returns the claim bool). A post must not replace it: posts do not claim, and unclaimed keys pass to the OS.

Held-modifier tracking lives here for unbound keys. Bound keys in command layers never reach it; that is why open/close on layer change exists (section 9).

---

## 6. Overlay show / hide / dwell

What. One overlay across all layers. Root holds `overlay: Option<TimerGuard>`.

```text
o (toggle)           toggle_overlay   show content + arm dwell, or hide
dwell fire           hide_overlay     HideOverlay, drop guard
set_layer            hide_overlay     always, before flush
```

Target.

```text
toggle_overlay   #[bind] on each layer that has o; stays or leaves unchanged
hide_overlay     #[bind] on root matching the live guard's trigger; stays
set_layer hide   stays inside set_layer (not a post): layer change is the mutation that implies hide
```

`drop-emits-effects.md` is the related open: forgetting `HideOverlay` on a path that clears `overlay` is a bug RAII cannot fix because `Drop` returns nothing. That is not a pre/post problem.

---

## 7. Window placement settle

What. A `SetFrame` is followed by several AX reports. While `pending` is live, those reports do not clear `restore`. A timer ends the wait.

```text
placing / restoring    set pending + SetFrame + Schedule(PLACEMENT_SETTLE)
placement_settled      forget_pending
user move (not ours)   windows.record clears restore
```

Target. `#[bind]` at root for the timer fire. Arming stays in `Windows::asking_for` (called from resize handlers). No post: cancel is Drop of the guard when a new placement replaces pending or the window closes out from under it; there is no "rearm on activity" story.

---

## 8. jk window timer

What. In typing, a started `jk` run waits `JK_TIMEOUT` for the next key; on fire, swallowed keys replay.

```text
arm in maybe_pass_through on Advanced+opening
fire: jk_timeout => replay(interrupt())
```

Target. Keep as root `#[bind]` on the guard trigger. Arming stays inside the exclusive AnyKey body (it is the run that opens, not a post keyed on descent). No separate post.

---

## 9. Layer change side effects (`set_layer`)

What. The one writer of `layer`. Always produces:

```text
hide overlay if up
jk sequence reset
if leaving passthrough  held.close()   // UPs for held modifiers
if entering passthrough held.open()    // DOWNs for held modifiers
ShowLayer(name)
```

Target. Stays a method called from exclusive handlers that change layer. Not a post: it is the mutation those handlers perform, and the effects are part of their return. Posts on ancestors that need to know a leave happened already see `Invalidated` from the leave itself; they do not re-implement flush.

---

## 10. Quit

What. Home `q` and menu-bar Quit. Open held modifiers, then `Kill`.

Target. `#[bind]` at home for `q`, at root for `Quit`. Stays put until `Kill` tears the process down. Exclusive so nothing else also acts.

---

## 11. Derived app / site levels

What. Levels that are not stored; rebuilt each dispatch from root truth.

```text
app_data(path)   confirmed() front app  -> Option<AppData>
site_data(path)  confirmed Chrome URL   -> Option<SiteData>
```

Bindings on the derived leaves (Chrome, Ghostty, ClaudeAi) are ordinary exclusive binds once change 5 defines the derived handler's `AscendState<P>`.

No post lives on a derived node today. A post that needed the derived path would be part of the derived-levels design, not this catalog.

---

## 12. Copy URL / host

What. Chrome `shift-l` / `cmd-l`: put URL text on the clipboard from model state, or fall back to AppleScript.

Target. `#[bind]`, stay. Reads root; no leave.

---

## Pattern checklist (what uses which slot)

Exclusive bind (`#[bind]` / `exclusive`):

```text
every command key (layer transitions, app actions, overlay toggle, quit)
root external events (foreground, tab, window)
root timer fires (jk, overlay dwell, placement settle)
root AnyKey catch-all (modifiers + passthrough + jk advance)
```

Post or pre_post (no claim; keys on stay vs leave):

```text
return-home deadline rearm/cancel     pre_post(AnyKey) on the timed node / wrapper
```

Not a scheduled handler:

```text
set_layer's flush, overlay hide, jk reset     method of the root
TimerGuard Drop cancel                        OS cancel; demo's CancelTimer is the explicit form
windows.record / placing                      state methods called from binds
derived app_data / site_data                  resolve_into inputs, not handlers
```

Still outside the model (product debt, not schedule debt):

```text
held-keys.md              non-modifier stream balance across swallow/pass
drop-emits-effects.md     HideOverlay if overlay field cleared without the method
timer-ids-on-root.md      TimerIds field instead of static AtomicU64
```

---

## First consumer

The return-home deadline (section 1) is the only behavior that *needs* `#[pre_post]` to leave `Mercury::handle`. Everything else is already a bind and migrates under change 5's signature rewrite.

Order once change 6 is green:

```text
1. Migrate return-home rearm into a pre_post on each timed layer (or the wrapper)
2. Delete Mercury::handle's rearm block and Layer::rearm_timeout
3. Optionally fold the four layers into AndReturnHome (timed-layer-wrapper.md)
4. Optionally multiple-children (two keyboards) once posts-at-branch-points is designed
```

No other mercury behavior is blocked on posts. New features that key on stay-vs-leave (a which-key overlay that arms on enter and cancels on leave; a sticky-modifier timeout; anything with an OS resource that must cancel when the active path dies) use the same pre_post shape as return-home.
