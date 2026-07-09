//! Two kinds of test. The per-event ones send one event and assert the effect
//! (and resulting state) straight from `handle`. The loop one drives a
//! `bind::SimpleRunner`, recording effects and, for a `Foreground` effect,
//! reporting the app back the way the OS watcher would.

use bind::SimpleRunner;
use mercury::{
    App, AppLayer, Key, KeyEvent, Layer, Mercury, MercuryEffect, MercuryStruct, PressType,
    foreground, key,
};

const fn emit(key: Key, press: PressType) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press })
}

// A key passed straight through: the one event it arrived as.
fn passed(key: Key) -> Vec<MercuryEffect> {
    vec![emit(key, PressType::Down)]
}

// cmd+r: cmd down, r down, r up, cmd up.
fn cmd_r() -> Vec<MercuryEffect> {
    vec![
        emit(Key::MetaLeft, PressType::Down),
        emit(Key::KeyR, PressType::Down),
        emit(Key::KeyR, PressType::Up),
        emit(Key::MetaLeft, PressType::Up),
    ]
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
    m.handle(&key(Key::KeyN));
    assert!(matches!(m.layer, Layer::Nav(_)));
    assert_eq!(m.handle(&key(Key::Escape)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));
}

#[test]
fn typing_passes_any_key_through() {
    let mut m = Mercury::default();
    m.handle(&key(Key::KeyT));
    assert_eq!(m.handle(&key(Key::KeyA)), Some(passed(Key::KeyA)));
    assert_eq!(m.handle(&key(Key::KeyZ)), Some(passed(Key::KeyZ)));
    assert!(matches!(m.layer, Layer::Typing(_)));
}

#[test]
fn typing_escape_goes_home() {
    // Typing binds escape explicitly so its catch-all does not shadow the go-home
    // binding.
    let mut m = Mercury::default();
    m.handle(&key(Key::KeyT));
    assert_eq!(m.handle(&key(Key::Escape)), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));
}

#[test]
fn nav_c_foregrounds_chrome_without_changing_state() {
    let mut m = Mercury::default();
    m.handle(&key(Key::KeyN));
    assert_eq!(
        m.handle(&key(Key::KeyC)),
        Some(vec![MercuryEffect::Foreground(App::Chrome)])
    );
    // The effect is inert: still in nav, nothing foregrounded yet.
    assert!(matches!(m.layer, Layer::Nav(_)));
    assert_eq!(m.foregrounded, App::Other);
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
    m.handle(&foreground(App::Chrome));
    assert_eq!(m.handle(&key(Key::KeyI)), Some(vec![]));
    assert!(matches!(m.layer, Layer::InApp(AppLayer::Chrome(_))));
}

#[test]
fn chrome_r_refreshes() {
    let mut m = Mercury::default();
    m.handle(&foreground(App::Chrome));
    m.handle(&key(Key::KeyI));
    assert_eq!(m.handle(&key(Key::KeyR)), Some(cmd_r()));
}

#[test]
fn inapp_other_app_ignores_keys() {
    let mut m = Mercury::default();
    m.handle(&foreground(App::Zed));
    m.handle(&key(Key::KeyI));
    assert!(matches!(m.layer, Layer::InApp(AppLayer::Other(_))));
    assert_eq!(m.handle(&key(Key::KeyR)), None);
}

#[test]
fn unbound_key_is_none() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key(Key::KeyX)), None);
}

// ---- loop: driving a bind::SimpleRunner ----

/// Drain the runner, recording each effect and reporting a foregrounded app back
/// the way the OS watcher would (a `Foreground` effect becomes a foreground
/// event).
fn settle(runner: &mut SimpleRunner<'_, MercuryStruct, Mercury>, performed: &mut Vec<MercuryEffect>) {
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
    assert!(matches!(m.layer, Layer::Nav(_)));
}
