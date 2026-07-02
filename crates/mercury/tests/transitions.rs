//! Two kinds of test. The per-event ones send one event and assert the effect
//! (and resulting state) straight from `handle`. The loop one drives a
//! `bind::SimpleRunner`, recording effects and, for a `Foreground` effect,
//! reporting the app back the way the OS watcher would.

use bind::SimpleRunner;
use mercury::{App, AppLayer, Layer, Mercury, MercuryEffect, MercuryStruct, foreground, key};

// ---- per-event: send an event, assert the effect ----

#[test]
fn home_n_enters_nav() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key("n")), Some(vec![]));
    assert!(matches!(m.layer, Layer::Nav(_)));
}

#[test]
fn home_t_enters_typing() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key("t")), Some(vec![]));
    assert!(matches!(m.layer, Layer::Typing(_)));
}

#[test]
fn typing_passes_any_key_through() {
    let mut m = Mercury::default();
    m.handle(&key("t"));
    assert_eq!(m.handle(&key("a")), Some(vec![MercuryEffect::Type("a")]));
    // Not just a/s/d/f now: any key passes through.
    assert_eq!(m.handle(&key("q")), Some(vec![MercuryEffect::Type("q")]));
}

#[test]
fn typing_still_quits_and_goes_home() {
    let mut m = Mercury::default();
    m.handle(&key("t"));
    assert_eq!(m.handle(&key("return")), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));

    let mut m = Mercury::default();
    m.handle(&key("t"));
    assert_eq!(m.handle(&key("escape")), Some(vec![MercuryEffect::Kill]));
}

#[test]
fn nav_c_foregrounds_chrome_without_changing_state() {
    let mut m = Mercury::default();
    m.handle(&key("n"));
    assert_eq!(
        m.handle(&key("c")),
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
    assert_eq!(m.handle(&key("i")), Some(vec![]));
    assert!(matches!(m.layer, Layer::InApp(AppLayer::Chrome(_))));
}

#[test]
fn chrome_r_refreshes() {
    let mut m = Mercury::default();
    m.handle(&foreground(App::Chrome));
    m.handle(&key("i"));
    assert_eq!(m.handle(&key("r")), Some(vec![MercuryEffect::Command("r")]));
}

#[test]
fn inapp_other_app_ignores_keys() {
    let mut m = Mercury::default();
    m.handle(&foreground(App::Zed));
    m.handle(&key("i"));
    assert!(matches!(m.layer, Layer::InApp(AppLayer::Other(_))));
    assert_eq!(m.handle(&key("r")), None);
}

#[test]
fn return_goes_home_from_anywhere() {
    let mut m = Mercury::default();
    m.handle(&key("n"));
    assert!(matches!(m.layer, Layer::Nav(_)));
    assert_eq!(m.handle(&key("return")), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));
}

#[test]
fn escape_quits_from_anywhere() {
    let mut m = Mercury::default();
    m.handle(&key("n"));
    assert_eq!(m.handle(&key("escape")), Some(vec![MercuryEffect::Kill]));
}

#[test]
fn unbound_key_is_none() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key("q")), None);
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
        for k in ["n", "c"] {
            runner.queue_event(key(k));
            settle(&mut runner, &mut performed);
        }
    }
    assert_eq!(performed, vec![MercuryEffect::Foreground(App::Chrome)]);
    assert_eq!(m.foregrounded, App::Chrome);
    assert!(matches!(m.layer, Layer::Nav(_)));
}
