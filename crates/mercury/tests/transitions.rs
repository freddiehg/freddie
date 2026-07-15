//! Two kinds of test. The per-event ones send one event and assert the effect
//! (and resulting state) straight from `handle`. The loop one drives a
//! `bind::SimpleRunner`, recording effects and, for a `Foreground` effect,
//! reporting the app back the way the OS watcher would.

use bind::SimpleRunner;
use mercury::{
    App, AppLayer, Key, KeyEvent, Layer, Mercury, MercuryEffect, MercuryStruct, Placement,
    PressType, foreground, key,
};

const fn emit(key: Key, press: PressType) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press })
}

// A key passed straight through: the one event it arrived as.
fn passed(key: Key) -> Vec<MercuryEffect> {
    vec![emit(key, PressType::Down)]
}

fn tap(modifiers: &[Key], key: Key) -> MercuryEffect {
    MercuryEffect::Tap {
        modifiers: modifiers.to_vec(),
        key,
    }
}

// cmd-r, one chord.
fn cmd_r() -> Vec<MercuryEffect> {
    vec![tap(&[Key::MetaLeft], Key::KeyR)]
}

// ---- per-event: send an event, assert the effect ----

#[test]
fn home_n_enters_nav() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::KeyN)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Nav(_)));
}

#[test]
fn home_t_enters_typing() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::KeyT)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Typing(_)));
}

#[test]
fn home_q_quits() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::KeyQ)), Some(vec![MercuryEffect::Kill]));
}

#[test]
fn home_escape_does_nothing() {
    // In home, escape re-enters home (the layer-level go-home binding): no effect,
    // no visible change.
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::Escape)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));
}

#[test]
fn escape_goes_home_from_a_sublayer() {
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyN));
    assert!(matches!(m.layer, Layer::Nav(_)));
    assert_eq!(m.handle(&key(Key::Escape)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));
}

#[test]
fn typing_passes_any_key_through() {
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyT));
    assert_eq!(m.handle(&key(Key::KeyA)), Some(passed(Key::KeyA)));
    assert_eq!(m.handle(&key(Key::KeyZ)), Some(passed(Key::KeyZ)));
    assert!(matches!(m.layer, Layer::Typing(_)));
}

#[test]
fn typing_plain_escape_passes_through() {
    // In typing, escape is a normal key: it passes through and stays in typing.
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyT));
    assert_eq!(m.handle(&key(Key::Escape)), Some(passed(Key::Escape)));
    assert!(matches!(m.layer, Layer::Typing(_)));
}

#[test]
fn typing_cmd_escape_exits_to_home() {
    // cmd is held (tracked by the catch-all, which also emits it), then escape exits to home
    // and is swallowed. The plain escape above still passes through.
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyT));

    // cmd down: tracked, and passed through.
    assert_eq!(
        m.handle(&key(Key::MetaLeft)),
        Some(vec![emit(Key::MetaLeft, PressType::Down)])
    );
    assert!(matches!(m.layer, Layer::Typing(_)));

    // escape while cmd held: go home, swallow the escape.
    assert_eq!(m.handle(&key(Key::Escape)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));
}

// Nav is a one-shot chooser: picking an app emits the effect and lands in the in-app
// layer, with the navigation marked pending until the watcher reports the app.
#[test]
fn nav_c_foregrounds_chrome_and_enters_inapp() {
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyN));
    assert_eq!(
        m.handle(&key(Key::KeyC)),
        Some(vec![MercuryEffect::Foreground(App::Chrome)])
    );
    assert!(matches!(m.layer, Layer::InApp(_)));
    // The effect is inert: nothing is foregrounded until the watcher reports it, and
    // the navigation is pending until then.
    assert_eq!(m.foregrounded, App::Other);
    assert!(m.has_navigated);
}

// Every nav choice lands in the in-app layer, not just Chrome's.
#[test]
fn every_nav_choice_enters_inapp() {
    for (k, app) in [
        (Key::KeyC, App::Chrome),
        (Key::KeyG, App::Ghostty),
        (Key::KeyZ, App::Zed),
    ] {
        let mut m = Mercury::default();
        let _ = m.handle(&key(Key::KeyN));
        assert!(matches!(m.layer, Layer::Nav(_)));
        assert_eq!(
            m.handle(&key(k)),
            Some(vec![MercuryEffect::Foreground(app)])
        );
        assert!(matches!(m.layer, Layer::InApp(_)), "{app:?} left nav");
        assert!(m.has_navigated, "{app:?} did not mark the nav pending");
    }
}

// The whole point: `n c` foregrounds Chrome and, once the watcher reports it, `r`
// refreshes it. No separate `i`.
#[test]
fn n_c_then_foreground_then_r_refreshes_chrome() {
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyN));
    assert_eq!(
        m.handle(&key(Key::KeyC)),
        Some(vec![MercuryEffect::Foreground(App::Chrome)])
    );
    assert!(matches!(m.layer, Layer::InApp(_)));

    let _ = m.handle(&foreground(App::Chrome)); // the watcher reports it
    assert_eq!(m.foregrounded, App::Chrome);
    assert!(!m.has_navigated);
    assert_eq!(m.handle(&key(Key::KeyR)), Some(cmd_r()));
}

// While a nav is pending, the in-app level is empty: `foregrounded` is still the old
// app, so its bindings must not apply in the gap. A key pressed before the foreground
// event lands is unbound; once the event lands, the chosen app's bindings apply.
#[test]
fn a_pending_nav_binds_nothing_until_the_foreground_event() {
    let mut m = Mercury::default();
    // Ghostty is frontmost, an app that has in-app bindings.
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyN)); // home -> nav
    let _ = m.handle(&key(Key::KeyC)); // navigate to Chrome; foregrounded still Ghostty
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert!(m.has_navigated);
    assert_eq!(m.foregrounded, App::Ghostty);
    // Ghostty's `j` does not apply, even though Ghostty is still `foregrounded`.
    assert_eq!(m.handle(&key(Key::KeyJ)), None);
    // Chrome's `r` does not apply yet either: nothing binds while the nav is pending.
    assert_eq!(m.handle(&key(Key::KeyR)), None);

    let _ = m.handle(&foreground(App::Chrome)); // the watcher catches up
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Chrome);
    assert!(!m.has_navigated);
    assert_eq!(m.handle(&key(Key::KeyR)), Some(cmd_r()));
}

#[test]
fn foreground_records_the_app_without_changing_layer() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&foreground(App::Zed)), Some(vec![]));
    assert_eq!(m.foregrounded, App::Zed);
    assert!(matches!(m.layer, Layer::Home(_)));
}

#[test]
fn i_enters_inapp_for_the_foregrounded_app() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Chrome));
    assert_eq!(m.handle(&key(Key::KeyI)), Some(vec![]));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Chrome);
}

#[test]
fn chrome_r_refreshes() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::KeyR)), Some(cmd_r()));
}

#[test]
fn inapp_other_app_ignores_keys() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Zed));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert!(matches!(m.foregrounded, App::Zed | App::Other));
    assert_eq!(m.handle(&key(Key::KeyR)), None);
}

#[test]
fn unbound_key_is_none() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::KeyX)), None);
}

// ---- ghostty: j/k walk tmux's windows, digits jump to one ----

// tmux's prefix is a chord, and the command is a bare tap. If the prefix were held
// through the command, tmux would see `ctrl-p` rather than `p`.
fn tmux(modifiers: &[Key], command: Key) -> Vec<MercuryEffect> {
    vec![tap(&[Key::ControlLeft], Key::KeyA), tap(modifiers, command)]
}

#[test]
fn i_enters_ghostty_in_app_when_ghostty_is_frontmost() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Ghostty);
}

#[test]
fn ghostty_j_is_previous_window_and_k_is_next() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));

    assert_eq!(m.handle(&key(Key::KeyJ)), Some(tmux(&[], Key::KeyP)));
    assert_eq!(m.handle(&key(Key::KeyK)), Some(tmux(&[], Key::KeyN)));
    // Still in Ghostty's layer, so windows can be walked without re-entering.
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Ghostty);
}

// The command carries no modifiers. Emitting it inside the prefix chord is the bug
// that would make tmux see `ctrl-p` rather than `p`, and the shape now says so:
// the prefix is one tap and the command is another.
#[test]
fn the_tmux_command_is_a_bare_tap() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));
    let effects = m.handle(&key(Key::KeyJ)).expect("j is bound");

    assert_eq!(effects.len(), 2, "a prefix and a command");
    assert_eq!(effects[0], tap(&[Key::ControlLeft], Key::KeyA));
    assert_eq!(effects[1], tap(&[], Key::KeyP));
}

// j and k belong to Ghostty, not to every app.
#[test]
fn j_and_k_are_unbound_in_chrome_in_app() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::KeyJ)), None);
    assert_eq!(m.handle(&key(Key::KeyK)), None);
}

// Foregrounding Ghostty while in-app retargets to its layer, so its bindings
// follow the front app the way Chrome's do.
#[test]
fn foregrounding_ghostty_retargets_the_inapp_layer() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Chrome);

    let _ = m.handle(&foreground(App::Ghostty));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Ghostty);
    assert_eq!(m.handle(&key(Key::KeyJ)), Some(tmux(&[], Key::KeyP)));
}

// The digits jump to a tmux window with the *shifted* symbol, because that is what
// the tmux config binds: `!`..`)` select windows 1..10, while the bare digits
// select window indices and cannot reach the tenth.
#[test]
fn the_digits_select_a_tmux_window_and_return_home() {
    for (k, expected) in [
        (Key::Num1, Key::Num1),
        (Key::Num5, Key::Num5),
        (Key::Num9, Key::Num9),
        (Key::Num0, Key::Num0),
    ] {
        let mut m = Mercury::default();
        let _ = m.handle(&foreground(App::Ghostty));
        let _ = m.handle(&key(Key::KeyI));

        assert_eq!(
            m.handle(&key(k)),
            Some(tmux(&[Key::ShiftLeft], expected)),
            "{k:?}"
        );
        // Choosing a window is a choice, not something you repeat.
        assert!(matches!(m.layer, Layer::Home(_)), "{k:?} stayed in ghostty");
    }
}

// Every digit is bound, and each sends its own.
#[test]
fn all_ten_digits_are_bound_in_ghostty() {
    let digits = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
        Key::Num0,
    ];
    for digit in digits {
        let mut m = Mercury::default();
        let _ = m.handle(&foreground(App::Ghostty));
        let _ = m.handle(&key(Key::KeyI));
        assert_eq!(
            m.handle(&key(digit)),
            Some(tmux(&[Key::ShiftLeft], digit)),
            "{digit:?} is unbound"
        );
    }
}

// Walking windows repeats, so j and k stay; jumping to one does not, so it leaves.
#[test]
fn walking_stays_but_jumping_leaves() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));

    let _ = m.handle(&key(Key::KeyJ));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Ghostty);
    let _ = m.handle(&key(Key::Num3));
    assert!(matches!(m.layer, Layer::Home(_)));
}

// The digits belong to Ghostty, not to home or to Chrome.
#[test]
fn the_digits_are_unbound_outside_ghostty() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::Num1)), None);

    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::Num1)), None);
}

// ---- resize: `r` from home, then the arrows place the focused window ----

#[test]
fn home_r_enters_resize() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::KeyR)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Resize(_)));
}

// Resize is a one-shot chooser, like nav: each arrow emits its placement and lands
// back in home, so `r up` maximizes and leaves you where you started.
#[test]
fn the_arrows_place_the_window_and_return_home() {
    for (k, placement) in [
        (Key::UpArrow, Placement::Maximize),
        (Key::LeftArrow, Placement::LeftHalf),
        (Key::RightArrow, Placement::RightHalf),
    ] {
        let mut m = Mercury::default();
        let _ = m.handle(&key(Key::KeyR));
        assert!(matches!(m.layer, Layer::Resize(_)));

        assert_eq!(
            m.handle(&key(k)),
            Some(vec![MercuryEffect::Place(placement)]),
            "{k:?}"
        );
        assert!(matches!(m.layer, Layer::Home(_)), "{k:?} stayed in resize");
    }
}

// Escape leaves resize without placing anything.
#[test]
fn escape_leaves_resize() {
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyR));
    assert!(matches!(m.layer, Layer::Resize(_)));

    assert_eq!(m.handle(&key(Key::Escape)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));
}

// Placing twice means entering resize twice: `r up r left`.
#[test]
fn placing_twice_re_enters_resize() {
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::UpArrow)),
        Some(vec![MercuryEffect::Place(Placement::Maximize)])
    );
    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::LeftArrow)),
        Some(vec![MercuryEffect::Place(Placement::LeftHalf)])
    );
    assert!(matches!(m.layer, Layer::Home(_)));
}

// The arrows mean nothing outside resize, so they do not place a window by
// accident from home.
#[test]
fn the_arrows_are_unbound_in_home() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::UpArrow)), None);
    assert_eq!(m.handle(&key(Key::LeftArrow)), None);
    assert_eq!(m.handle(&key(Key::RightArrow)), None);
}

// `r` is Chrome's refresh in the in-app layer, and resize's entry from home. The
// layers keep them apart.
#[test]
fn r_still_refreshes_chrome_in_app() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::KeyR)), Some(cmd_r()));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Chrome);
}

// ---- loop: driving a bind::SimpleRunner ----

/// Drain the runner, recording each effect and reporting a foregrounded app back
/// the way the OS watcher would (a `Foreground` effect becomes a foreground
/// event).
fn settle(
    runner: &mut SimpleRunner<'_, MercuryStruct, Mercury>,
    performed: &mut Vec<MercuryEffect>,
) {
    while let Some(dispatched) = runner.next() {
        if let Some(output) = dispatched {
            for effect in output {
                if let MercuryEffect::Foreground(app) = &effect {
                    runner.queue_event(foreground(*app));
                }
                performed.push(effect);
            }
        }
    }
}

// Foregrounding an app from nav emits the effect, and the reported-back event
// records it: after n, c the effect is Foreground(Chrome) and Chrome is recorded.
#[test]
fn foregrounding_chrome_is_reported_back() {
    let mut m = Mercury::default();
    let mut performed = Vec::new();
    {
        let mut runner = SimpleRunner::<MercuryStruct, _>::new(&mut m);
        for k in [Key::KeyN, Key::KeyC] {
            runner.queue_event(key(k));
            settle(&mut runner, &mut performed);
        }
    }
    assert_eq!(performed, vec![MercuryEffect::Foreground(App::Chrome)]);
    assert_eq!(m.foregrounded, App::Chrome);
    // Nav landed in Chrome's in-app layer, and the reported-back event cleared the
    // pending flag so Chrome's bindings are live.
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert!(!m.has_navigated);
}

// ---- app navigation: name mapping and the in-app layer following the front app ----

// Every real app's bundle id maps back to that app, and `Other` (no specific app)
// has no bundle id and is where unknown ids land.
#[test]
fn bundle_id_round_trips() {
    for app in [App::Chrome, App::Ghostty, App::Zed] {
        let id = app.bundle_id().expect("a real app has a bundle id");
        assert_eq!(App::from_bundle_id(id), app);
    }
    assert_eq!(App::Other.bundle_id(), None);
    assert_eq!(App::from_bundle_id("com.example.Unknown"), App::Other);
}

// The bundle ids the OS actually reports. Unlike display names, these do not vary
// with who is asked, so there is one spelling and it is this one.
#[test]
fn reported_bundle_ids_map() {
    assert_eq!(App::from_bundle_id("com.google.Chrome"), App::Chrome);
    assert_eq!(App::from_bundle_id("com.mitchellh.ghostty"), App::Ghostty);
    assert_eq!(App::from_bundle_id("dev.zed.Zed"), App::Zed);
    // A display name is not a bundle id.
    assert_eq!(App::from_bundle_id("Google Chrome"), App::Other);
}

// The in-app layer holds no app. Its bindings come from `root.foregrounded`, read on every
// dispatch, so changing the root changes what binds WITHOUT anything re-entering the layer
// and without any resync. There is no copy to go stale.
#[test]
fn the_inapp_layers_bindings_follow_the_root_with_no_resync() {
    let mut m = Mercury {
        foregrounded: App::Chrome,
        layer: Layer::InApp(AppLayer::default()),
        ..Default::default()
    };
    // Chrome binds `r`.
    assert_eq!(m.handle(&key(Key::KeyR)), Some(cmd_r()));

    // Write the ROOT directly. Nothing touches the layer.
    m.foregrounded = App::Ghostty;

    // Chrome's `r` is gone and Ghostty's `j` is live, with no re-entry and no resync.
    assert_eq!(m.handle(&key(Key::KeyR)), None);
    assert!(m.handle(&key(Key::KeyJ)).is_some());

    // An app with no bindings has no level at all.
    m.foregrounded = App::Zed;
    assert_eq!(m.handle(&key(Key::KeyJ)), None);
}

// In the in-app layer, foregrounding a different app retargets the layer to it, so
// the old app's bindings drop and the new app's apply.
#[test]
fn foreground_retargets_the_inapp_layer() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Chrome);

    assert_eq!(m.handle(&foreground(App::Zed)), Some(vec![]));
    assert_eq!(m.foregrounded, App::Zed);
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert!(matches!(m.foregrounded, App::Zed | App::Other));
    // Chrome's refresh is gone now that Chrome is not the front app.
    assert_eq!(m.handle(&key(Key::KeyR)), None);
}

// Foregrounding Chrome again while in-app restores its bindings.
#[test]
fn foreground_back_to_chrome_restores_its_bindings() {
    let mut m = Mercury::default();
    let _ = m.handle(&foreground(App::Zed));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert!(matches!(m.foregrounded, App::Zed | App::Other));

    let _ = m.handle(&foreground(App::Chrome));
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Chrome);
    assert_eq!(m.handle(&key(Key::KeyR)), Some(cmd_r()));
}

// Outside the in-app layer, foregrounding records the app but never moves you
// between layers.
#[test]
fn foreground_outside_inapp_does_not_change_layer() {
    let mut m = Mercury::default();
    let _ = m.handle(&key(Key::KeyN));
    assert!(matches!(m.layer, Layer::Nav(_)));

    assert_eq!(m.handle(&foreground(App::Chrome)), Some(vec![]));
    assert_eq!(m.foregrounded, App::Chrome);
    assert!(matches!(m.layer, Layer::Nav(_)));
}

// The full loop: foreground Chrome from nav (reported back), enter its in-app
// layer, then the OS switches the front app to Zed and the in-app layer follows.
#[test]
fn inapp_follows_the_front_app_across_a_switch() {
    let mut m = Mercury::default();
    let mut performed = Vec::new();
    {
        let mut runner = SimpleRunner::<MercuryStruct, _>::new(&mut m);
        // `n c` lands straight in Chrome's in-app layer; the reported-back foreground
        // event clears the pending flag. No `i` needed.
        for k in [Key::KeyN, Key::KeyC] {
            runner.queue_event(key(k));
            settle(&mut runner, &mut performed);
        }
    }
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert_eq!(m.foregrounded, App::Chrome);
    // The user switches to Zed outside mercury; the watcher reports it.
    let _ = m.handle(&foreground(App::Zed));
    assert_eq!(m.foregrounded, App::Zed);
    assert!(matches!(m.layer, Layer::InApp(_)));
    assert!(matches!(m.foregrounded, App::Zed | App::Other));
}
