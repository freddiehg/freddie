//! Two kinds of test. The per-event ones send one event and assert the effect
//! (and resulting state) straight from `handle`. The loop one drives a
//! `bind::SimpleRunner`, recording effects and, for a `Foreground` effect,
//! reporting the app back the way the OS watcher would.

use bind::SimpleRunner;
use freddie_windows::{Frame, Monitor, WindowChange, WindowFrame, WindowId};
use mercury::{
    App, Chord, Copied, HomeLayer, JK_TIMEOUT, Key, KeyEvent, Layer, Mercury, MercuryEffect,
    MercuryEvent, MercuryStruct, ModifierFlags, OVERLAY_DWELL, PLACEMENT_SETTLE, PressType,
    RETURN_TO_HOME_TIMEOUT, ReturnHomeLayers, UrlPart, WindowEvent, Windows, foreground, key,
    quit_event, tab,
};

// `BOOT_TITLE` is painted on the status item before the model exists, so it is a literal rather
// than read off the boot layer. This is the guard that keeps the literal honest.
#[test]
fn boot_title_matches_the_boot_layer() {
    let booted = Mercury::new(App::Other, Windows::default());
    assert_eq!(booted.layer().name(), Mercury::BOOT_TITLE);
}

// Entering nav, resize, or the in-app layer arms the return-to-home timer; this is the effect
// that schedules it. Equality under `testing` compares the delay and fire event, so a rebuilt one
// matches what a layer produced.
fn return_home_timer() -> MercuryEffect {
    let (_guard, effect) = freddie::timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, fired);
    MercuryEffect::Timer(effect)
}

// A placement arms the settle wait; this is the effect that schedules it. It bounds how long a
// move reported for that window counts as mercury's own rather than the user's.
fn settle_timer() -> MercuryEffect {
    let (_guard, effect) = freddie::timer_effect_and_guard(PLACEMENT_SETTLE, fired);
    MercuryEffect::Timer(effect)
}

// A firing of `id`. A test reads the id off the effect that set the timer: nothing else can know
// it, because the timer mints it.
const fn fired(id: freddie::TimerId) -> MercuryEvent {
    MercuryEvent::Timer(freddie::TimerFired(id))
}

// The id a timer effect was set with.
fn timer_id(effects: &[MercuryEffect]) -> freddie::TimerId {
    let timer = effects
        .iter()
        .find_map(|e| match e {
            MercuryEffect::Timer(timer) => Some(timer),
            _ => None,
        })
        .expect("these effects set a timer");
    match timer.event {
        MercuryEvent::Timer(freddie::TimerFired(id)) => id,
        ref other => panic!("not a timer firing: {other:?}"),
    }
}

// The active return-home layer, or `None` when the active layer is home or typing. The four
// timed layers sit under the wrapper that owns their shared timer.
const fn return_home(m: &Mercury) -> Option<&ReturnHomeLayers> {
    match m.layer() {
        Layer::ReturnHome(w) => Some(w.layers()),
        Layer::Home(_) | Layer::Typing(_) => None,
    }
}

// A mercury in Home, the command layer. The default is Typing (passthrough), but most per-event
// tests exercise Home's command bindings, so they start here.
fn home() -> Mercury {
    Mercury::with_layer(Layer::Home(HomeLayer))
}

const fn emit(key: Key, press: PressType) -> MercuryEffect {
    emit_with(key, press, ModifierFlags::empty())
}

const fn emit_with(key: Key, press: PressType, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press, flags })
}

// A key passed straight through: the one event it arrived as.
fn passed(key: Key) -> Vec<MercuryEffect> {
    vec![emit(key, PressType::Down)]
}

// A key's release, for the halves the jk sequence cares about.
const fn up(key: Key) -> MercuryEvent {
    MercuryEvent::Key(KeyEvent {
        key,
        press: PressType::Up,
        flags: ModifierFlags::empty(),
    })
}

// A key carrying a modifier, the way the source stamps it.
const fn key_with(key: Key, flags: ModifierFlags) -> MercuryEvent {
    MercuryEvent::Key(KeyEvent {
        key,
        press: PressType::Down,
        flags,
    })
}

const fn tap(key: Key, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Tap(Chord { key, flags })
}

// cmd-r, one chord.
fn cmd_r() -> Vec<MercuryEffect> {
    vec![tap(Key::KeyR, ModifierFlags::COMMAND)]
}

// A transition also tells the menu bar which layer it landed in, as the last effect it produces.
const fn shows(layer: &'static str) -> MercuryEffect {
    MercuryEffect::ShowLayer(layer)
}

// Effects from a transition that landed in home, which is most of them: the go-home name comes
// last, since the handler emits its own effects before it changes the layer.
fn leaves(mut effects: Vec<MercuryEffect>) -> Vec<MercuryEffect> {
    effects.push(shows("Home"));
    effects
}

// A key handled while staying in the in-app layer is activity: its return-home timer is reset, so
// the effects come back with the re-scheduling timer effect appended. Keys that leave the in-app
// layer (the digits' window jump, `n`, `t`, `escape`) do not, so they use the bare effects.
fn in_app(mut effects: Vec<MercuryEffect>) -> Vec<MercuryEffect> {
    effects.push(return_home_timer());
    effects
}

// ---- per-event: send an event, assert the effect ----

#[test]
fn default_boots_into_typing() {
    // A fresh mercury is in typing (passthrough), the login-safe state, not command-mode Home.
    assert!(matches!(
        Mercury::new(App::Other, Windows::default()).layer(),
        Layer::Typing(_)
    ));
}

#[test]
fn every_layer_has_a_name_for_the_menu_bar() {
    // A new layer has to name itself here rather than inheriting something generic.
    let mut m = home();
    assert_eq!(m.layer().name(), "Home");
    let _ = m.handle(&key(Key::KeyN));
    assert_eq!(m.layer().name(), "Nav");
    let _ = m.handle(&key(Key::Escape));
    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(m.layer().name(), "Resize");
    let _ = m.handle(&key(Key::Escape));
    let _ = m.handle(&key(Key::KeyT));
    assert_eq!(m.layer().name(), "Typing");

    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.layer().name(), "App");
}

#[test]
fn home_n_enters_nav() {
    let mut m = home();
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        (vec![shows("Nav"), return_home_timer()], true)
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))));
}

#[test]
fn home_t_enters_typing() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::KeyT)), (vec![shows("Typing")], true));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

#[test]
fn home_q_quits() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::KeyQ)), (vec![MercuryEffect::Kill], true));
}

#[test]
fn quit_event_kills_from_home() {
    let mut m = home();
    assert_eq!(m.handle(&quit_event()), (vec![MercuryEffect::Kill], true));
    // No layer change: quit is an effect, not a transition.
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn quit_emits_held_modifiers_so_the_app_learns_the_physical_state() {
    // cmd held in home is swallowed, so the app never saw its down. On quit the grab is
    // released and no further down is coming, so emit the down before Kill or the app is left
    // thinking a physically-held cmd is up.
    let mut m = home();
    let _ = m.handle(&key(Key::MetaLeft)); // tracked, swallowed in home
    assert_eq!(
        m.handle(&quit_event()),
        (
            vec![
                emit_with(Key::MetaLeft, PressType::Down, ModifierFlags::COMMAND),
                MercuryEffect::Kill,
            ],
            true
        )
    );
}

#[test]
fn quit_event_kills_from_every_layer() {
    // The menu-bar Quit is a recovery path: it must kill from any layer, not just
    // home. One case per layer. Typing matters most: its `AnyKey` catch-all must not
    // swallow the quit event (a different event type), so quit still reaches the root.
    for enter in [Key::KeyN, Key::KeyT, Key::KeyR, Key::KeyI] {
        let mut m = home();
        let _ = m.handle(&key(enter));
        assert_eq!(
            m.handle(&quit_event()),
            (vec![MercuryEffect::Kill], true),
            "quit from the layer entered by {enter:?}"
        );
    }
}

#[test]
fn home_escape_does_nothing() {
    // In home, escape re-enters home (the layer-level go-home binding): it renames the layer to
    // the one it is already in, and nothing else changes.
    let mut m = home();
    assert_eq!(m.handle(&key(Key::Escape)), (vec![shows("Home")], true));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn escape_goes_home_from_a_sublayer() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyN));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))));
    assert_eq!(m.handle(&key(Key::Escape)), (vec![shows("Home")], true));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn nav_times_out_home() {
    let mut m = home();
    let entered = m.handle(&key(Key::KeyN)).0;
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))));
    // The timer nav set fires: its id came back on the effect that set it.
    assert_eq!(
        m.handle(&fired(timer_id(&entered))),
        (vec![shows("Home")], true)
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn a_firing_from_a_layer_already_left_matches_nothing() {
    // Enter nav, leave, and enter again: the first timer's firing arrives late, after a second
    // nav replaced it. It must not send the live one home.
    let mut m = home();
    let first = timer_id(&m.handle(&key(Key::KeyN)).0);
    let _ = m.handle(&key(Key::Escape));
    let second = timer_id(&m.handle(&key(Key::KeyN)).0);
    assert_ne!(first, second, "each entry sets its own timer");

    assert_eq!(
        m.handle(&fired(first)),
        (vec![], false),
        "no binding matches a stale firing"
    );
    assert!(
        matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))),
        "still in nav"
    );

    assert_eq!(m.handle(&fired(second)), (vec![shows("Home")], true));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn a_firing_in_a_layer_that_set_no_timer_matches_nothing() {
    // Home sets none, so there is no binding for a firing to match, whatever id it carries.
    let mut m = home();
    let stale = timer_id(&m.handle(&key(Key::KeyN)).0);
    let _ = m.handle(&key(Key::Escape));
    assert_eq!(m.handle(&fired(stale)), (vec![], false));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn typing_passes_any_key_through() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyT));
    assert_eq!(m.handle(&key(Key::KeyA)), (passed(Key::KeyA), true));
    assert_eq!(m.handle(&key(Key::KeyZ)), (passed(Key::KeyZ), true));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

#[test]
fn typing_passes_a_baked_modifier_through() {
    // A modifier baked onto the event itself, never arriving as its own key (an injected cmd-v,
    // or fn), rides through instead of being dropped.
    let mut m = home();
    let _ = m.handle(&key(Key::KeyT));
    let cmd_v = MercuryEvent::Key(KeyEvent {
        key: Key::KeyV,
        press: PressType::Down,
        flags: ModifierFlags::COMMAND,
    });
    assert_eq!(
        m.handle(&cmd_v),
        (
            vec![emit_with(
                Key::KeyV,
                PressType::Down,
                ModifierFlags::COMMAND
            )],
            true
        )
    );
}

#[test]
fn typing_plain_escape_passes_through() {
    // In typing, escape is a normal key: it passes through and stays in typing.
    let mut m = home();
    let _ = m.handle(&key(Key::KeyT));
    assert_eq!(m.handle(&key(Key::Escape)), (passed(Key::Escape), true));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

#[test]
fn typing_cmd_escape_types_the_escape() {
    // Typing binds nothing, so cmd-escape no longer leaves: both keys reach the app, and jk is the
    // only way out.
    let mut m = home();
    let _ = m.handle(&key(Key::KeyT));

    // cmd down arrives carrying the command flag (as its flagsChanged does). It is tracked in
    // held (for the exit sweep) and passed through with that flag.
    let cmd_down = MercuryEvent::Key(KeyEvent {
        key: Key::MetaLeft,
        press: PressType::Down,
        flags: ModifierFlags::COMMAND,
    });
    assert_eq!(
        m.handle(&cmd_down),
        (
            vec![emit_with(
                Key::MetaLeft,
                PressType::Down,
                ModifierFlags::COMMAND
            )],
            true
        )
    );

    let cmd_escape = MercuryEvent::Key(KeyEvent {
        key: Key::Escape,
        press: PressType::Down,
        flags: ModifierFlags::COMMAND,
    });
    assert_eq!(
        m.handle(&cmd_escape),
        (
            vec![emit_with(
                Key::Escape,
                PressType::Down,
                ModifierFlags::COMMAND
            )],
            true
        )
    );
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

// Nav is a one-shot chooser: picking an app emits the effect and lands in the in-app
// layer, with the navigation marked pending until the watcher reports the app.
#[test]
fn nav_c_foregrounds_chrome_and_enters_inapp() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyN));
    assert_eq!(
        m.handle(&key(Key::KeyC)),
        (
            // The gesture's units in call order: the foreground effect, then the layer's own.
            vec![
                MercuryEffect::Foreground(App::Chrome),
                shows("App"),
                return_home_timer(),
            ],
            true
        )
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    // The effect is inert: nothing is foregrounded until the watcher reports it, and
    // the navigation is pending until then.
    assert_eq!(m.foreground.app(), App::Other);
    assert!(m.foreground.navigating());
}

// Every nav choice lands in the in-app layer, not just Chrome's.
#[test]
fn every_nav_choice_enters_inapp() {
    for (k, app) in [
        (Key::KeyC, App::Chrome),
        (Key::KeyG, App::Ghostty),
        (Key::KeyZ, App::Zed),
    ] {
        let mut m = home();
        let _ = m.handle(&key(Key::KeyN));
        assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))));
        assert_eq!(
            m.handle(&key(k)),
            (
                vec![
                    MercuryEffect::Foreground(app),
                    shows("App"),
                    return_home_timer(),
                ],
                true
            )
        );
        assert!(
            matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))),
            "{app:?} left nav"
        );
        assert!(
            m.foreground.navigating(),
            "{app:?} did not mark the nav pending"
        );
    }
}

// `space` in nav opens Spotlight and leaves for typing, so the query reaches its field. The tap
// comes first, ahead of the transition's effects.
#[test]
fn nav_space_opens_spotlight_and_enters_typing() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyN));
    assert_eq!(
        m.handle(&key(Key::Space)),
        (
            vec![tap(Key::Space, ModifierFlags::COMMAND), shows("Typing")],
            true
        )
    );
    assert!(matches!(m.layer(), Layer::Typing(_)));
    // Nothing was foregrounded, and no navigation is pending: Spotlight is not an app choice.
    assert_eq!(m.foreground.app(), App::Other);
    assert!(!m.foreground.navigating());
    // Typing passes keys through, so what follows types itself into Spotlight.
    assert_eq!(m.handle(&key(Key::KeyC)), (passed(Key::KeyC), true));
}

// `n c` foregrounds Chrome and, once the watcher reports it, `r`
// refreshes it. No separate `i`.
#[test]
fn n_c_then_foreground_then_r_refreshes_chrome() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyN));
    assert_eq!(
        m.handle(&key(Key::KeyC)),
        (
            // The gesture's units in call order: the foreground effect, then the layer's own.
            vec![
                MercuryEffect::Foreground(App::Chrome),
                shows("App"),
                return_home_timer(),
            ],
            true
        )
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));

    let _ = m.handle(&foreground(App::Chrome)); // the watcher reports it
    assert_eq!(m.foreground.app(), App::Chrome);
    assert!(!m.foreground.navigating());
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(cmd_r()), true));
}

// While a nav is pending, the in-app level is empty: `foreground.app()` is still the old
// app, so its bindings must not apply in the gap. A key pressed before the foreground
// event lands is unbound; once the event lands, the chosen app's bindings apply.
#[test]
fn a_pending_nav_binds_nothing_until_the_foreground_event() {
    let mut m = home();
    // Ghostty is frontmost, an app that has in-app bindings.
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyN)); // home -> nav
    let _ = m.handle(&key(Key::KeyC)); // navigate to Chrome; the front app is still Ghostty
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert!(m.foreground.navigating());
    assert_eq!(m.foreground.app(), App::Ghostty);
    // Ghostty's `j` does not apply, even though Ghostty is still the (stale) front app.
    assert_eq!(m.handle(&key(Key::KeyJ)), (in_app(vec![]), false));
    // Chrome's `r` does not apply yet either: nothing binds while the nav is pending.
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(vec![]), false));

    let _ = m.handle(&foreground(App::Chrome)); // the watcher catches up
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Chrome);
    assert!(!m.foreground.navigating());
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(cmd_r()), true));
}

#[test]
fn foreground_records_the_app_without_changing_layer() {
    let mut m = home();
    assert_eq!(m.handle(&foreground(App::Zed)), (vec![], true));
    assert_eq!(m.foreground.app(), App::Zed);
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn i_enters_inapp_for_the_foregrounded_app() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    assert_eq!(
        m.handle(&key(Key::KeyI)),
        (vec![shows("App"), return_home_timer()], true)
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Chrome);
}

// Chrome in the in-app layer, with `url` reported for its front tab.
fn chrome_showing(url: &str) -> Mercury {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    let _ = m.handle(&tab(url.to_owned()));
    m
}

fn copies(text: &str) -> MercuryEffect {
    MercuryEffect::Copy(Copied::Text(text.to_owned()))
}

// `l` focuses the address bar and lands in typing, so the URL you type gets there.
#[test]
fn chrome_l_focuses_the_address_bar_and_enters_typing() {
    let mut m = chrome_showing("https://www.x.com/asdfasdf");
    assert_eq!(
        m.handle(&key(Key::KeyL)),
        (
            vec![tap(Key::KeyL, ModifierFlags::COMMAND), shows("Typing")],
            true
        )
    );
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

// `shift-l` copies the whole URL, out of the state rather than out of the address bar: no keys are
// sent to Chrome at all.
#[test]
fn chrome_shift_l_copies_the_url() {
    let mut m = chrome_showing("https://www.x.com/asdfasdf");
    assert_eq!(
        m.handle(&key_with(Key::KeyL, ModifierFlags::SHIFT)),
        (in_app(vec![copies("https://www.x.com/asdfasdf")]), true)
    );
    // It repeats, so it stays in the in-app layer.
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
}

// `cmd-l` copies the host, `www.` and all.
#[test]
fn chrome_cmd_l_copies_the_host() {
    let mut m = chrome_showing("https://www.x.com/asdfasdf");
    assert_eq!(
        m.handle(&key_with(Key::KeyL, ModifierFlags::COMMAND)),
        (in_app(vec![copies("www.x.com")]), true)
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
}

// The three are one key at three modifier combinations, and each does only its own thing.
#[test]
fn the_three_ls_do_not_shadow_each_other() {
    for (event, want) in [
        (
            key(Key::KeyL),
            vec![tap(Key::KeyL, ModifierFlags::COMMAND), shows("Typing")],
        ),
        (
            key_with(Key::KeyL, ModifierFlags::SHIFT),
            in_app(vec![copies("https://claude.ai/new")]),
        ),
        (
            key_with(Key::KeyL, ModifierFlags::COMMAND),
            in_app(vec![copies("claude.ai")]),
        ),
    ] {
        let mut m = chrome_showing("https://claude.ai/new");
        assert_eq!(m.handle(&event), (want, true), "{event:?}");
    }
}

// With no URL reported there is nothing to copy out of the state, so the copy asks Chrome instead.
// Which is the case for a tab the extension never saw, or no extension at all.
#[test]
fn a_copy_with_no_reported_url_asks_chrome() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(
        m.handle(&key_with(Key::KeyL, ModifierFlags::SHIFT)),
        (
            in_app(vec![MercuryEffect::Copy(Copied::FrontTabUrl(
                UrlPart::Whole
            ))]),
            true
        )
    );
    assert_eq!(
        m.handle(&key_with(Key::KeyL, ModifierFlags::COMMAND)),
        (
            in_app(vec![MercuryEffect::Copy(Copied::FrontTabUrl(
                UrlPart::Host
            ))]),
            true
        )
    );
}

// A URL with no host has no host to copy, and asking Chrome would get the same answer back.
#[test]
fn copying_the_host_of_a_hostless_url_copies_nothing() {
    let mut m = chrome_showing("about:blank");
    assert_eq!(
        m.handle(&key_with(Key::KeyL, ModifierFlags::COMMAND)),
        (in_app(vec![]), true)
    );
}

// The `l` bindings are Chrome's, not the in-app layer's: another app's level does not get them.
#[test]
fn the_ls_are_chromes_alone() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::KeyL)), (in_app(vec![]), false));
    assert_eq!(
        m.handle(&key_with(Key::KeyL, ModifierFlags::SHIFT)),
        (in_app(vec![]), false)
    );
}

// Chrome in the site layer, with `url` reported for its front tab.
fn site_showing(url: &str) -> Mercury {
    let mut m = chrome_showing(url);
    let _ = m.handle(&key(Key::KeyS));
    m
}

// `n` on claude.ai sends the site's own new-chat shortcut and lands in typing, so what you type
// reaches the prompt box the new chat opened in.
#[test]
fn claude_ai_n_starts_a_new_chat_and_enters_typing() {
    let mut m = site_showing("https://claude.ai/new");
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        (
            vec![
                tap(Key::KeyO, ModifierFlags::COMMAND | ModifierFlags::SHIFT),
                shows("Typing")
            ],
            true
        )
    );
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

// The new-chat binding is claude.ai's alone: another site's front tab leaves `n` unbound in the
// site layer.
#[test]
fn n_is_claude_ais_alone() {
    let mut m = site_showing("https://www.x.com/asdfasdf");
    // Swallowed, and the site layer treats the keypress as activity: its return-home timer resets.
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        (vec![return_home_timer()], false)
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Site(_))));
}

// `s` in the in-app layer reaches the site layer, the way `u` does from home: `i` is what the
// front app can do, `s` is what the site in its front tab can do, with no trip through home.
#[test]
fn inapp_s_enters_site() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(
        m.handle(&key(Key::KeyS)),
        (vec![shows("Site"), return_home_timer()], true)
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Site(_))));
}

// The in-app layer works like home for entering nav and typing: `n` and `t` reach
// past the app's own bindings (which bind neither) to the layer's.
#[test]
fn inapp_n_enters_nav() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        (vec![shows("Nav"), return_home_timer()], true)
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))));
}

#[test]
fn inapp_t_enters_typing() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.handle(&key(Key::KeyT)), (vec![shows("Typing")], true));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

// The app's own bindings still win over the layer's: Ghostty binds `j`, so `j` walks
// its windows rather than doing nothing, and `n`/`t` are the only keys the in-app
// layer adds on top.
#[test]
fn inapp_app_bindings_still_take_precedence() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(
        m.handle(&key(Key::KeyJ)),
        (in_app(tmux(ModifierFlags::empty(), Key::KeyP)), true)
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
}

#[test]
fn chrome_r_refreshes() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(cmd_r()), true));
}

#[test]
fn inapp_other_app_ignores_keys() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Zed));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert!(matches!(m.foreground.app(), App::Zed | App::Other));
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(vec![]), false));
}

#[test]
fn unbound_key_is_none() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::KeyX)), (vec![], false));
}

// ---- ghostty: j/k walk tmux's windows, digits jump to one ----

// tmux's prefix is a chord, and the command is a bare tap. If the prefix were held
// through the command, tmux would see `ctrl-p` rather than `p`.
fn tmux(flags: ModifierFlags, command: Key) -> Vec<MercuryEffect> {
    vec![tap(Key::KeyA, ModifierFlags::CONTROL), tap(command, flags)]
}

#[test]
fn i_enters_ghostty_in_app_when_ghostty_is_frontmost() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Ghostty);
}

#[test]
fn ghostty_j_is_previous_window_and_k_is_next() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));

    assert_eq!(
        m.handle(&key(Key::KeyJ)),
        (in_app(tmux(ModifierFlags::empty(), Key::KeyP)), true)
    );
    assert_eq!(
        m.handle(&key(Key::KeyK)),
        (in_app(tmux(ModifierFlags::empty(), Key::KeyN)), true)
    );
    // Still in Ghostty's layer, so windows can be walked without re-entering.
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Ghostty);
}

// The command carries no modifiers. Emitting it inside the prefix chord would make tmux see
// `ctrl-p` rather than `p`; the shape says so: the prefix is one tap and the command is another.
#[test]
fn the_tmux_command_is_a_bare_tap() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));
    let effects = m.handle(&key(Key::KeyJ)).0;

    // A prefix and a command, then the return-home timer reset (walking is in-app activity).
    assert_eq!(effects.len(), 3);
    assert_eq!(effects[0], tap(Key::KeyA, ModifierFlags::CONTROL));
    assert_eq!(effects[1], tap(Key::KeyP, ModifierFlags::empty()));
    assert_eq!(effects[2], return_home_timer());
}

// j and k belong to Ghostty, not to every app.
#[test]
fn j_and_k_are_unbound_in_chrome_in_app() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::KeyJ)), (in_app(vec![]), false));
    assert_eq!(m.handle(&key(Key::KeyK)), (in_app(vec![]), false));
}

// Foregrounding Ghostty while in-app retargets to its layer, so its bindings
// follow the front app the way Chrome's do.
#[test]
fn foregrounding_ghostty_retargets_the_inapp_layer() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Chrome);

    let _ = m.handle(&foreground(App::Ghostty));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Ghostty);
    assert_eq!(
        m.handle(&key(Key::KeyJ)),
        (in_app(tmux(ModifierFlags::empty(), Key::KeyP)), true)
    );
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
        let mut m = home();
        let _ = m.handle(&foreground(App::Ghostty));
        let _ = m.handle(&key(Key::KeyI));

        assert_eq!(
            m.handle(&key(k)),
            (leaves(tmux(ModifierFlags::SHIFT, expected)), true),
            "{k:?}"
        );
        // Choosing a window is a choice, not something you repeat.
        assert!(
            matches!(m.layer(), Layer::Home(_)),
            "{k:?} stayed in ghostty"
        );
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
        let mut m = home();
        let _ = m.handle(&foreground(App::Ghostty));
        let _ = m.handle(&key(Key::KeyI));
        assert_eq!(
            m.handle(&key(digit)),
            (leaves(tmux(ModifierFlags::SHIFT, digit)), true),
            "{digit:?} is unbound"
        );
    }
}

// Walking windows repeats, so j and k stay; jumping to one does not, so it leaves.
#[test]
fn walking_stays_but_jumping_leaves() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));

    let _ = m.handle(&key(Key::KeyJ));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Ghostty);
    let _ = m.handle(&key(Key::Num3));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// In-app activity resets the return-home timer: a key that stays in the layer re-emits the
// scheduling effect, restarting the idle clock, while a key that leaves does not.
#[test]
fn inapp_activity_resets_the_return_home_timer() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Ghostty));
    let _ = m.handle(&key(Key::KeyI));
    // Walking a window stays in-app, so the timer is reset.
    assert_eq!(
        m.handle(&key(Key::KeyJ)),
        (in_app(tmux(ModifierFlags::empty(), Key::KeyP)), true)
    );
    // Jumping to a window leaves for home, so nothing re-schedules it.
    assert_eq!(
        m.handle(&key(Key::Num3)),
        (leaves(tmux(ModifierFlags::SHIFT, Key::Num3)), true)
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// The digits belong to Ghostty, not to home or to Chrome.
#[test]
fn the_digits_are_unbound_outside_ghostty() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::Num1)), (vec![], false));

    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::Num1)), (in_app(vec![]), false));
}

// ---- resize: `r` from home, then the arrows place the focused window ----

#[test]
fn home_r_enters_resize() {
    let mut m = home();
    assert_eq!(
        m.handle(&key(Key::KeyR)),
        (vec![shows("Resize"), return_home_timer()], true)
    );
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Resize(_))));
}

// Resize is a one-shot chooser, like nav: each arrow emits the rectangle its window is
// going to and lands back in home, so `r up` maximizes and leaves you where you started.
#[test]
fn the_arrows_place_the_window_and_return_home() {
    for (k, frame) in [
        (Key::UpArrow, SCREEN.visible),
        (
            Key::LeftArrow,
            Frame {
                width: 800.0,
                ..SCREEN.visible
            },
        ),
        (
            Key::RightArrow,
            Frame {
                x: 800.0,
                width: 800.0,
                ..SCREEN.visible
            },
        ),
    ] {
        let mut m = home_with_a_window();
        let _ = m.handle(&key(Key::KeyR));
        assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Resize(_))));

        assert_eq!(
            m.handle(&key(k)),
            (
                leaves(vec![
                    MercuryEffect::SetFrame(WindowFrame {
                        window: WINDOW,
                        frame,
                    }),
                    settle_timer(),
                ]),
                true
            ),
            "{k:?}"
        );
        assert!(
            matches!(m.layer(), Layer::Home(_)),
            "{k:?} stayed in resize"
        );
    }
}

// With nothing focused there is nothing to place, so the key is spent and the layer is
// left, but no window moves.
#[test]
fn a_placement_with_no_focused_window_asks_for_nothing() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(m.handle(&key(Key::UpArrow)), (leaves(vec![]), true));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// Escape leaves resize without placing anything.
#[test]
fn escape_leaves_resize() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyR));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Resize(_))));

    assert_eq!(m.handle(&key(Key::Escape)), (vec![shows("Home")], true));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// Placing twice means entering resize twice: `r up r left`.
#[test]
fn placing_twice_re_enters_resize() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::UpArrow)),
        (
            leaves(vec![
                MercuryEffect::SetFrame(WindowFrame {
                    window: WINDOW,
                    frame: SCREEN.visible,
                }),
                settle_timer(),
            ]),
            true
        )
    );
    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::LeftArrow)),
        (
            leaves(vec![
                MercuryEffect::SetFrame(WindowFrame {
                    window: WINDOW,
                    frame: Frame {
                        width: 800.0,
                        ..SCREEN.visible
                    },
                }),
                settle_timer(),
            ]),
            true
        )
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// The arrows mean nothing outside resize, so they do not place a window by
// accident from home.
#[test]
fn the_arrows_are_unbound_in_home() {
    let mut m = home();
    assert_eq!(m.handle(&key(Key::UpArrow)), (vec![], false));
    assert_eq!(m.handle(&key(Key::LeftArrow)), (vec![], false));
    assert_eq!(m.handle(&key(Key::RightArrow)), (vec![], false));
}

// `r` is Chrome's refresh in the in-app layer, and resize's entry from home. The
// layers keep them apart.
#[test]
fn r_still_refreshes_chrome_in_app() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(cmd_r()), true));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Chrome);
}

// ---- loop: driving a bind::SimpleRunner ----

/// Drain the runner, recording each effect and reporting a foregrounded app back
/// the way the OS watcher would (a `Foreground` effect becomes a foreground
/// event).
fn settle(
    runner: &mut SimpleRunner<'_, MercuryStruct, Mercury>,
    performed: &mut Vec<MercuryEffect>,
) {
    while let Some((effects, _handled)) = runner.next() {
        for effect in effects {
            if let MercuryEffect::Foreground(app) = &effect {
                runner.queue_event(foreground(*app));
            }
            performed.push(effect);
        }
    }
}

// Foregrounding an app from nav emits the effect, and the reported-back event
// records it: after n, c the effect is Foreground(Chrome) and Chrome is recorded.
#[test]
fn foregrounding_chrome_is_reported_back() {
    let mut m = home();
    let mut performed = Vec::new();
    {
        let mut runner = SimpleRunner::<MercuryStruct, _>::new(&mut m);
        for k in [Key::KeyN, Key::KeyC] {
            runner.queue_event(key(k));
            settle(&mut runner, &mut performed);
        }
    }
    assert_eq!(
        performed,
        vec![
            shows("Nav"),
            return_home_timer(),
            MercuryEffect::Foreground(App::Chrome),
            shows("App"),
            return_home_timer(),
        ]
    );
    assert_eq!(m.foreground.app(), App::Chrome);
    // Nav landed in Chrome's in-app layer, and the reported-back event cleared the
    // pending flag so Chrome's bindings are live.
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert!(!m.foreground.navigating());
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

// The in-app layer holds no app. Its bindings come from `root.foreground`, read on every
// dispatch, so changing the root changes what binds WITHOUT anything re-entering the layer
// and without any resync. There is no copy to go stale.
#[test]
fn the_inapp_layers_bindings_follow_the_root_with_no_resync() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyI)); // enter the in-app layer
    m.foreground.set_front_app(App::Chrome);
    // Chrome binds `r`.
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(cmd_r()), true));

    // Write the ROOT directly. Nothing touches the layer.
    m.foreground.set_front_app(App::Ghostty);

    // Chrome's `r` is gone and Ghostty's `j` is live, with no re-entry and no resync.
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(vec![]), false));
    assert!(m.handle(&key(Key::KeyJ)).1);

    // An app with no bindings has no level at all.
    m.foreground.set_front_app(App::Zed);
    assert_eq!(m.handle(&key(Key::KeyJ)), (in_app(vec![]), false));
}

// In the in-app layer, foregrounding a different app retargets the layer to it, so
// the old app's bindings drop and the new app's apply.
#[test]
fn foreground_retargets_the_inapp_layer() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Chrome);

    assert_eq!(m.handle(&foreground(App::Zed)), (vec![], true));
    assert_eq!(m.foreground.app(), App::Zed);
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert!(matches!(m.foreground.app(), App::Zed | App::Other));
    // Chrome's refresh is gone now that Chrome is not the front app.
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(vec![]), false));
}

// Foregrounding Chrome again while in-app restores its bindings.
#[test]
fn foreground_back_to_chrome_restores_its_bindings() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Zed));
    let _ = m.handle(&key(Key::KeyI));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert!(matches!(m.foreground.app(), App::Zed | App::Other));

    let _ = m.handle(&foreground(App::Chrome));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Chrome);
    assert_eq!(m.handle(&key(Key::KeyR)), (in_app(cmd_r()), true));
}

// Outside the in-app layer, foregrounding records the app but never moves you
// between layers.
#[test]
fn foreground_outside_inapp_does_not_change_layer() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyN));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))));

    assert_eq!(m.handle(&foreground(App::Chrome)), (vec![], true));
    assert_eq!(m.foreground.app(), App::Chrome);
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))));
}

// The full loop: foreground Chrome from nav (reported back), enter its in-app
// layer, then the OS switches the front app to Zed and the in-app layer follows.
#[test]
fn inapp_follows_the_front_app_across_a_switch() {
    let mut m = home();
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
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert_eq!(m.foreground.app(), App::Chrome);
    // The user switches to Zed outside mercury; the watcher reports it.
    let _ = m.handle(&foreground(App::Zed));
    assert_eq!(m.foreground.app(), App::Zed);
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))));
    assert!(matches!(m.foreground.app(), App::Zed | App::Other));
}

// ---- jk: the sequence that leaves typing ----

// Opening a run arms its window; this is the effect that schedules it. Equality under `testing`
// compares the delay and the fire event, so a rebuilt one matches what the run produced.
fn jk_timer() -> MercuryEffect {
    let (_guard, effect) = freddie::timer_effect_and_guard(JK_TIMEOUT, fired);
    MercuryEffect::Timer(effect)
}

// A mercury in typing, the passthrough layer, with the jk run idle.
fn typing() -> Mercury {
    Mercury::new(App::Other, Windows::default())
}

// The jk run, which lives on the typing layer, so asking for it in any other layer is a test bug.
fn jk(m: &Mercury) -> &freddie::KeySequence {
    match m.layer() {
        Layer::Typing(t) => &t.jk,
        other => panic!("not in typing: {other:?}"),
    }
}

#[test]
fn jk_typed_one_key_at_a_time_leaves_for_home() {
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert!(!jk(&m).is_idle());
    assert_eq!(m.handle(&up(Key::KeyJ)), (vec![], true));
    assert_eq!(m.handle(&key(Key::KeyK)), (vec![shows("Home")], true));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn jk_rolled_leaves_for_home_and_the_ups_land_in_home() {
    // k goes down before j comes up. The two ups that follow arrive in Home, which binds neither
    // and is not a passthrough layer, so they are swallowed rather than reaching the app as ups
    // with no downs.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(m.handle(&key(Key::KeyK)), (vec![shows("Home")], true));
    assert!(matches!(m.layer(), Layer::Home(_)));
    assert_eq!(m.handle(&up(Key::KeyJ)), (vec![], false));
    assert_eq!(m.handle(&up(Key::KeyK)), (vec![], false));
}

#[test]
fn a_j_tap_then_another_key_types_the_j_first() {
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(m.handle(&up(Key::KeyJ)), (vec![], true));
    assert_eq!(
        m.handle(&key(Key::KeyA)),
        (
            vec![
                emit(Key::KeyJ, PressType::Down),
                emit(Key::KeyJ, PressType::Up),
                emit(Key::KeyA, PressType::Down),
            ],
            true
        )
    );
}

#[test]
fn a_held_j_then_another_key_replays_only_its_down() {
    // Only the j down was swallowed, so only it replays. The real j up passes through later, with
    // the run already idle.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(
        m.handle(&key(Key::KeyA)),
        (
            vec![
                emit(Key::KeyJ, PressType::Down),
                emit(Key::KeyA, PressType::Down),
            ],
            true
        )
    );
    assert_eq!(
        m.handle(&up(Key::KeyJ)),
        (vec![emit(Key::KeyJ, PressType::Up)], true)
    );
}

#[test]
fn a_j_carrying_a_modifier_never_opens_the_run() {
    let mut m = typing();
    assert_eq!(
        m.handle(&key_with(Key::KeyJ, ModifierFlags::COMMAND)),
        (
            vec![emit_with(
                Key::KeyJ,
                PressType::Down,
                ModifierFlags::COMMAND
            )],
            true
        )
    );
    assert!(jk(&m).is_idle());
}

#[test]
fn a_modifier_arriving_mid_run_breaks_it() {
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(
        m.handle(&key_with(Key::MetaLeft, ModifierFlags::COMMAND)),
        (
            vec![
                emit(Key::KeyJ, PressType::Down),
                emit_with(Key::MetaLeft, PressType::Down, ModifierFlags::COMMAND),
            ],
            true
        )
    );
    assert!(jk(&m).is_idle());
}

#[test]
fn a_held_js_auto_repeat_breaks_the_run() {
    // The swallowed down replays ahead of the repeat, so the app sees the same two downs it would
    // have seen unwatched, and the k after it is an ordinary k.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(
        m.handle(&key(Key::KeyJ)),
        (
            vec![
                emit(Key::KeyJ, PressType::Down),
                emit(Key::KeyJ, PressType::Down),
            ],
            true
        )
    );
    assert!(jk(&m).is_idle());
    assert_eq!(m.handle(&key(Key::KeyK)), (passed(Key::KeyK), true));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

#[test]
fn escape_in_typing_breaks_the_run_and_reaches_the_app() {
    // Typing binds nothing, so escape runs through the sequence like any other key and the j
    // replays AHEAD of it.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(m.handle(&up(Key::KeyJ)), (vec![], true));
    assert_eq!(
        m.handle(&key(Key::Escape)),
        (
            vec![
                emit(Key::KeyJ, PressType::Down),
                emit(Key::KeyJ, PressType::Up),
                emit(Key::Escape, PressType::Down),
            ],
            true
        )
    );
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

#[test]
fn leaving_typing_abandons_a_held_j() {
    // The layer change drops the run with the layer, and the j is dropped rather than typed: the
    // app never saw its down, and its up will be swallowed by the command layer.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(m.handle(&key(Key::KeyK)), (vec![shows("Home")], true));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn j_and_k_still_type_themselves_when_they_are_not_a_run() {
    // j, j, k: the second j breaks the first run and does not open a second, so all three type.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(m.handle(&up(Key::KeyJ)), (vec![], true));
    assert_eq!(
        m.handle(&key(Key::KeyJ)),
        (
            vec![
                emit(Key::KeyJ, PressType::Down),
                emit(Key::KeyJ, PressType::Up),
                emit(Key::KeyJ, PressType::Down),
            ],
            true
        )
    );
    assert_eq!(
        m.handle(&up(Key::KeyJ)),
        (vec![emit(Key::KeyJ, PressType::Up)], true)
    );
    assert_eq!(m.handle(&key(Key::KeyK)), (passed(Key::KeyK), true));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

#[test]
fn a_half_typed_run_types_itself_when_the_window_elapses() {
    let mut m = typing();
    let opened = m.handle(&key(Key::KeyJ)).0;
    assert_eq!(opened, vec![jk_timer()]);
    assert_eq!(
        m.handle(&fired(timer_id(&opened))),
        (vec![emit(Key::KeyJ, PressType::Down)], true)
    );
    assert!(jk(&m).is_idle());
    // The k that follows is an ordinary k, not the second half of anything.
    assert_eq!(m.handle(&key(Key::KeyK)), (passed(Key::KeyK), true));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

#[test]
fn a_full_tap_types_itself_when_the_window_elapses() {
    // Both halves were swallowed, so both replay, in the order they arrived.
    let mut m = typing();
    let opened = m.handle(&key(Key::KeyJ)).0;
    assert_eq!(opened, vec![jk_timer()]);
    assert_eq!(m.handle(&up(Key::KeyJ)), (vec![], true));
    assert_eq!(
        m.handle(&fired(timer_id(&opened))),
        (
            vec![
                emit(Key::KeyJ, PressType::Down),
                emit(Key::KeyJ, PressType::Up),
            ],
            true
        )
    );
}

#[test]
fn a_firing_from_a_run_that_ended_matches_nothing() {
    // Open a run, break it, open another: the first window's firing arrives late. It must not
    // interrupt the run that replaced it.
    let mut m = typing();
    let first = timer_id(&m.handle(&key(Key::KeyJ)).0);
    let _ = m.handle(&key(Key::KeyA)); // breaks it
    let second = timer_id(&m.handle(&key(Key::KeyJ)).0);
    assert_ne!(first, second, "each run sets its own window");

    assert_eq!(
        m.handle(&fired(first)),
        (vec![], false),
        "no binding matches a stale firing"
    );
    assert!(!jk(&m).is_idle(), "the live run is untouched");

    assert_eq!(
        m.handle(&fired(second)),
        (vec![emit(Key::KeyJ, PressType::Down)], true)
    );
}

#[test]
fn a_firing_with_no_run_in_progress_matches_nothing() {
    let mut m = typing();
    let stale = timer_id(&m.handle(&key(Key::KeyJ)).0);
    let _ = m.handle(&key(Key::KeyA)); // breaks it, so nothing is live
    assert_eq!(m.handle(&fired(stale)), (vec![], false));
}

#[test]
fn the_window_is_armed_once_per_run_not_once_per_key() {
    // The j up advances the run without re-arming: the window runs from the first key.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), (vec![jk_timer()], true));
    assert_eq!(m.handle(&up(Key::KeyJ)), (vec![], true));
}

// ---- the overlay: `o` shows the active layer's keymap ----

// The effect `o` produces beside the text: the overlay's hide timer. Equality under `testing`
// compares the delay and the fire event, and a firing compares equal whatever its id.
fn overlay_hide_timer() -> MercuryEffect {
    let (_guard, effect) = freddie::timer_effect_and_guard(OVERLAY_DWELL, fired);
    MercuryEffect::Timer(effect)
}

// Where the overlay's own effects start in a batch. In a timed layer the deadline post runs
// during the ascent below the root, so its rearm precedes them; in home there is no such timer
// and the overlay's text is first.
fn shown_at(effects: &[MercuryEffect]) -> usize {
    effects
        .iter()
        .position(|e| matches!(e, MercuryEffect::ShowOverlay(_)))
        .unwrap_or_else(|| panic!("o shows the overlay: {effects:?}"))
}

// The keymap `o` put up, asserted by its heading rather than in full, so re-wording a row does not
// rewrite the test table. Its dwell follows it.
fn shown_heading(effects: &[MercuryEffect]) -> &'static str {
    let at = shown_at(effects);
    match &effects[at..] {
        [
            MercuryEffect::ShowOverlay(text),
            MercuryEffect::Timer(_),
            ..,
        ] => text.lines().next().expect("a keymap has a heading"),
        other => panic!("o shows the overlay and sets its hide: {other:?}"),
    }
}

// The id of the dwell `o` set, which is the timer that follows the overlay's text.
fn dwell_id(effects: &[MercuryEffect]) -> freddie::TimerId {
    timer_id(&effects[shown_at(effects)..])
}

#[test]
fn o_shows_the_layers_keymap() {
    for (enter, heading) in [
        (None, "  HOME"),
        (Some(Key::KeyN), "  NAV"),
        (Some(Key::KeyR), "  RESIZE"),
    ] {
        let mut m = home();
        if let Some(k) = enter {
            let _ = m.handle(&key(k));
        }
        let effects = m.handle(&key(Key::KeyO)).0;
        assert_eq!(shown_heading(&effects), heading);
        assert_eq!(effects[shown_at(&effects) + 1], overlay_hide_timer());
    }
}

// `o` shows the overlay and keeps you in the layer, so in a chooser layer it is activity: the
// deadline post rearms that layer's return-home timer. It runs on the wrapper below the root, so
// its effect leads the batch, ahead of the overlay `o` itself puts up. Home has no such timer, so
// its `o` is only the overlay and its dwell.
#[test]
fn showing_the_overlay_rearms_the_return_home_timer() {
    for enter in [Key::KeyN, Key::KeyR] {
        let mut m = home();
        let _ = m.handle(&key(enter));
        let effects = m.handle(&key(Key::KeyO)).0;
        assert_eq!(
            effects.len(),
            3,
            "the rearm, then the overlay and its dwell"
        );
        assert_eq!(effects[0], return_home_timer());
        assert_eq!(effects[2], overlay_hide_timer());
    }

    let mut m = home();
    let effects = m.handle(&key(Key::KeyO)).0;
    assert_eq!(effects.len(), 2, "home has no return-home timer to rearm");
    assert_eq!(effects[1], overlay_hide_timer());
}

#[test]
fn the_in_app_keymap_is_the_front_apps() {
    // The in-app layer's bindings are the app's, so its keymap has to be too.
    for (app, heading) in [
        (App::Chrome, "  CHROME"),
        (App::Ghostty, "  GHOSTTY"),
        (App::Zed, "  IN-APP"),
    ] {
        let mut m = home();
        let _ = m.handle(&foreground(app));
        let _ = m.handle(&key(Key::KeyI));
        let effects = m.handle(&key(Key::KeyO)).0;
        assert_eq!(shown_heading(&effects), heading, "{app:?}");
    }
}

#[test]
fn the_overlay_hides_after_the_dwell() {
    let mut m = home();
    let shown = m.handle(&key(Key::KeyO)).0;
    assert_eq!(
        m.handle(&fired(dwell_id(&shown))),
        (vec![MercuryEffect::HideOverlay], true)
    );
    // And again matches nothing: the field was taken, so no binding names that guard.
    assert_eq!(m.handle(&fired(dwell_id(&shown))), (vec![], false));
}

#[test]
fn o_again_takes_it_down() {
    // `o` is the key you press to ask what is bound, so it is the key you press when you are done.
    let mut m = home();
    let _ = m.handle(&key(Key::KeyO));
    assert_eq!(
        m.handle(&key(Key::KeyO)),
        (vec![MercuryEffect::HideOverlay], true)
    );
    // And a third press puts it back up.
    let effects = m.handle(&key(Key::KeyO)).0;
    assert_eq!(shown_heading(&effects), "  HOME");
}

#[test]
fn a_dwell_from_a_showing_already_gone_matches_nothing() {
    // Show one in home, leave for nav (which takes it down), and show nav's. The first showing's
    // dwell arrives late and must not take the live one down.
    let mut m = home();
    let first = dwell_id(&m.handle(&key(Key::KeyO)).0);
    let _ = m.handle(&key(Key::KeyN));
    let second = dwell_id(&m.handle(&key(Key::KeyO)).0);
    assert_ne!(first, second, "each showing sets its own dwell");

    assert_eq!(m.handle(&fired(first)), (vec![], false));
    assert_eq!(
        m.handle(&fired(second)),
        (vec![MercuryEffect::HideOverlay], true)
    );
}

#[test]
fn changing_layers_takes_the_overlay_down() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyO));
    // Entering nav hides it, ahead of naming the layer and setting nav's own timer.
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        (
            vec![
                MercuryEffect::HideOverlay,
                shows("Nav"),
                return_home_timer(),
            ],
            true
        )
    );
}

#[test]
fn a_transition_with_no_overlay_hides_nothing() {
    let mut m = home();
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        (vec![shows("Nav"), return_home_timer()], true)
    );
}

#[test]
fn o_in_typing_is_typed() {
    // Typing binds nothing, so `o` falls to the root and reaches the app.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyO)), (passed(Key::KeyO), true));
}

// ---- the window source: `Windows` is a pure function of the changes reported to it ----

const SCREEN: Monitor = Monitor {
    full: Frame {
        x: 0.0,
        y: 0.0,
        width: 1600.0,
        height: 925.0,
    },
    visible: Frame {
        x: 0.0,
        y: 25.0,
        width: 1600.0,
        height: 900.0,
    },
};
const WINDOW: WindowId = WindowId(7);
const WINDOW_FRAME: Frame = Frame {
    x: 100.0,
    y: 100.0,
    width: 400.0,
    height: 300.0,
};

const fn windows(change: WindowChange) -> MercuryEvent {
    MercuryEvent::Window(WindowEvent { change })
}

const fn opened(window: WindowId, frame: Frame) -> WindowChange {
    WindowChange::Opened(WindowFrame { window, frame })
}

// A mercury told about one screen and one focused window.
fn home_with_a_window() -> Mercury {
    let mut m = home();
    let _ = m.handle(&windows(WindowChange::Screens(vec![SCREEN])));
    let _ = m.handle(&windows(opened(WINDOW, WINDOW_FRAME)));
    let _ = m.handle(&windows(WindowChange::Focused(Some(WINDOW))));
    m
}

#[test]
fn an_opened_window_is_recorded_with_its_frame() {
    let m = home_with_a_window();
    assert_eq!(
        m.windows.focused(),
        Some(WindowFrame {
            window: WINDOW,
            frame: WINDOW_FRAME
        })
    );
}

// A window event records and asks for nothing: the source keeps the state true, and a key is
// what acts on it.
#[test]
fn a_window_change_produces_no_effects() {
    let mut m = home();
    assert_eq!(
        m.handle(&windows(opened(WINDOW, WINDOW_FRAME))),
        (vec![], true)
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// A move and a resize are the same to mercury, which keeps a frame and nothing else.
#[test]
fn a_move_and_a_resize_both_replace_the_frame() {
    for change in [
        WindowChange::Moved(WindowFrame {
            window: WINDOW,
            frame: SCREEN.visible,
        }),
        WindowChange::Resized(WindowFrame {
            window: WINDOW,
            frame: SCREEN.visible,
        }),
    ] {
        let mut m = home_with_a_window();
        let _ = m.handle(&windows(change));
        assert_eq!(
            m.windows.focused().expect("still focused").frame,
            SCREEN.visible
        );
    }
}

#[test]
fn a_closed_window_leaves_no_frame_and_no_focus() {
    let mut m = home_with_a_window();
    let _ = m.handle(&windows(WindowChange::Closed(WINDOW)));
    assert_eq!(m.windows.focused(), None);
}

// A focus report can name a window no `Opened` ever did, and a window with no frame is not
// something a placement can start from.
#[test]
fn focus_on_an_unknown_window_yields_nothing_focused() {
    let mut m = home_with_a_window();
    let _ = m.handle(&windows(WindowChange::Focused(Some(WindowId(999)))));
    assert_eq!(m.windows.focused(), None);
}

// Applying a change twice lands where applying it once does, which is what makes the boot
// ordering safe: a change during boot arrives in the snapshot and again as an event.
#[test]
fn recording_a_change_twice_is_recording_it_once() {
    let mut once = home_with_a_window();
    let mut twice = home_with_a_window();
    let _ = twice.handle(&windows(opened(WINDOW, WINDOW_FRAME)));
    assert_eq!(once.windows.focused(), twice.windows.focused());

    let _ = once.handle(&windows(WindowChange::Focused(Some(WINDOW))));
    let _ = twice.handle(&windows(WindowChange::Focused(Some(WINDOW))));
    let _ = twice.handle(&windows(WindowChange::Focused(Some(WINDOW))));
    assert_eq!(once.windows.focused(), twice.windows.focused());
}

// A window's corner picks the screen it is on, which is what a placement measures against.
#[test]
fn the_monitor_is_the_one_the_window_is_on() {
    let m = home_with_a_window();
    assert_eq!(m.windows.monitor_for(WINDOW_FRAME), Some(SCREEN));

    // Off every screen: the first one, rather than nothing to place against.
    let off = Frame {
        x: 9000.0,
        ..WINDOW_FRAME
    };
    assert_eq!(m.windows.monitor_for(off), Some(SCREEN));
}

// Before any `Screens` report there is no screen to measure against, and a placement has to
// see that rather than invent one.
#[test]
fn no_screens_reported_means_no_monitor() {
    let m = home();
    assert_eq!(m.windows.monitor_for(WINDOW_FRAME), None);
}

// A window on the second display fills that display, not the one it started on. This is
// what `monitor_for` is for, and it only shows up with more than one screen.
#[test]
fn a_placement_uses_the_screen_the_window_is_on() {
    const SECOND: Monitor = Monitor {
        full: Frame {
            x: 1600.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        },
        visible: Frame {
            x: 1600.0,
            y: 25.0,
            width: 1000.0,
            height: 775.0,
        },
    };
    let on_second = Frame {
        x: 1700.0,
        ..WINDOW_FRAME
    };

    let mut m = home();
    let _ = m.handle(&windows(WindowChange::Screens(vec![SCREEN, SECOND])));
    let _ = m.handle(&windows(opened(WINDOW, on_second)));
    let _ = m.handle(&windows(WindowChange::Focused(Some(WINDOW))));
    let _ = m.handle(&key(Key::KeyR));

    assert_eq!(
        m.handle(&key(Key::UpArrow)),
        (
            leaves(vec![
                MercuryEffect::SetFrame(WindowFrame {
                    window: WINDOW,
                    frame: SECOND.visible,
                }),
                settle_timer(),
            ]),
            true
        )
    );
}

// ---- restore: `r` in resize puts the window back ----

// Maximize, let it land, then `r`: back to the frame it had before the placement.
#[test]
fn resize_r_restores_the_frame_from_before_the_placement() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));
    let _ = m.handle(&windows(WindowChange::Moved(WindowFrame {
        window: WINDOW,
        frame: SCREEN.visible,
    })));

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::KeyR)),
        (
            leaves(vec![
                MercuryEffect::SetFrame(WindowFrame {
                    window: WINDOW,
                    frame: WINDOW_FRAME,
                }),
                settle_timer(),
            ]),
            true
        )
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// A run of placements restores to where the window was before the first of them, not to
// the frame the previous placement left.
#[test]
fn a_second_placement_does_not_move_the_remembered_frame() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));
    let _ = m.handle(&windows(WindowChange::Moved(WindowFrame {
        window: WINDOW,
        frame: SCREEN.visible,
    })));
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::LeftArrow));

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::KeyR)),
        (
            leaves(vec![
                MercuryEffect::SetFrame(WindowFrame {
                    window: WINDOW,
                    frame: WINDOW_FRAME,
                }),
                settle_timer(),
            ]),
            true
        )
    );
}

// The reports one placement produces are the position and the size, each written twice, so
// the frames in between are ones nobody asked for. None of them counts as a move by hand.
#[test]
fn the_intermediate_frames_of_a_placement_are_not_a_move_by_hand() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));

    for frame in [
        // The position landed, the size has not.
        Frame {
            x: 0.0,
            y: 25.0,
            ..WINDOW_FRAME
        },
        SCREEN.visible,
    ] {
        let _ = m.handle(&windows(WindowChange::Moved(WindowFrame {
            window: WINDOW,
            frame,
        })));
    }

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::KeyR)),
        (
            leaves(vec![
                MercuryEffect::SetFrame(WindowFrame {
                    window: WINDOW,
                    frame: WINDOW_FRAME,
                }),
                settle_timer(),
            ]),
            true
        )
    );
}

// `set_frame` writes the position and the size twice, so the frame that was asked for is
// reported more than once. Every one of those is still mercury's own: ending the wait on the
// first would leave the rest looking like a drag, and `r` would have nothing to go back to.
#[test]
fn the_target_frame_reported_twice_is_still_not_a_move_by_hand() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));

    for _ in 0..2 {
        let _ = m.handle(&windows(WindowChange::Moved(WindowFrame {
            window: WINDOW,
            frame: SCREEN.visible,
        })));
        let _ = m.handle(&windows(WindowChange::Resized(WindowFrame {
            window: WINDOW,
            frame: SCREEN.visible,
        })));
    }

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::KeyR)),
        (
            leaves(vec![
                MercuryEffect::SetFrame(WindowFrame {
                    window: WINDOW,
                    frame: WINDOW_FRAME,
                }),
                settle_timer(),
            ]),
            true
        )
    );
}

// A move mercury did not ask for forgets the remembered frame, so `r` afterwards does
// nothing rather than dragging the window off where the user just put it.
#[test]
fn a_move_by_hand_forgets_the_remembered_frame() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let effects = m.handle(&key(Key::UpArrow)).0;
    // The settle wait ends, so the window is the user's again.
    let _ = m.handle(&fired(timer_id(&effects)));

    let _ = m.handle(&windows(WindowChange::Moved(WindowFrame {
        window: WINDOW,
        frame: Frame {
            x: 700.0,
            ..WINDOW_FRAME
        },
    })));

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(m.handle(&key(Key::KeyR)), (leaves(vec![]), true));
}

// Restoring takes the frame, so a second `r` has nothing to put back.
#[test]
fn restoring_twice_asks_for_nothing_the_second_time() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::KeyR));

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(m.handle(&key(Key::KeyR)), (leaves(vec![]), true));
}

// `r` in resize is restore, not a second entry into resize.
#[test]
fn r_in_resize_does_not_re_enter_resize() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyR));
    assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Resize(_))));
    let _ = m.handle(&key(Key::KeyR));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// A closed window takes its remembered frame with it, so a reused `CGWindowID` cannot
// restore a new window to a closed one's frame.
#[test]
fn a_closed_window_is_forgotten() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));

    let _ = m.handle(&windows(WindowChange::Closed(WINDOW)));
    let _ = m.handle(&windows(opened(WINDOW, SCREEN.visible)));
    let _ = m.handle(&windows(WindowChange::Focused(Some(WINDOW))));

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(m.handle(&key(Key::KeyR)), (leaves(vec![]), true));
}
