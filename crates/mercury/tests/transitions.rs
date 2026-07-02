//! Two kinds of test. The per-event ones send one event and assert the effect
//! (and resulting state) straight from `handle`. The loop ones drive a
//! `bind::SimpleRunner` one event at a time: queue a key, let its effects settle
//! (recording them and, for an "installed" app, queueing the foreground
//! follow-up), then queue the next key — the way the real machine sees "press c,
//! Chrome comes up, press r".

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
fn typing_types_the_letter() {
    let mut m = Mercury::default();
    m.handle(&key("space"));
    assert_eq!(m.handle(&key("a")), Some(vec![MercuryEffect::Type("a")]));
    assert_eq!(m.handle(&key("f")), Some(vec![MercuryEffect::Type("f")]));
}

#[test]
fn nav_c_opens_chrome_without_mutating_state() {
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
fn foreground_event_records_the_app_and_enters_in_app() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&foreground(App::Zed)), Some(vec![]));
    assert_eq!(m.foregrounded, App::Zed);
    assert!(matches!(m.layer, Layer::InApp(AppLayer::Zed(_))));
}

#[test]
fn chrome_rebinds_r_to_command() {
    let mut m = Mercury::default();
    m.handle(&foreground(App::Chrome));
    assert_eq!(m.handle(&key("r")), Some(vec![MercuryEffect::Command("r")]));
}

#[test]
fn ghostty_rebinds_d_to_command() {
    let mut m = Mercury::default();
    m.handle(&foreground(App::Ghostty));
    assert_eq!(m.handle(&key("d")), Some(vec![MercuryEffect::Command("d")]));
}

#[test]
fn escape_returns_home() {
    let mut m = Mercury::default();
    m.handle(&key("n"));
    assert_eq!(m.handle(&key("escape")), Some(vec![]));
    assert!(matches!(m.layer, Layer::Home(_)));
}

#[test]
fn unknown_app_has_no_in_app_bindings() {
    let mut m = Mercury::default();
    m.handle(&foreground(App::Other));
    assert!(matches!(m.layer, Layer::InApp(AppLayer::Other(_))));
    assert_eq!(m.handle(&key("d")), None);
}

#[test]
fn unbound_key_is_none() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&key("q")), None);
}

// ---- loop: driving a bind::SimpleRunner one event at a time ----

/// Process everything queued: record each effect, and for an installed app queue
/// the foreground follow-up (the way the OS reports it coming up). Returns when
/// the queue drains.
fn settle(
    runner: &mut SimpleRunner<'_, MercuryStruct, Mercury>,
    performed: &mut Vec<MercuryEffect>,
    installed: &[App],
) {
    while let Some(dispatched) = runner.next() {
        if let Some(output) = dispatched {
            for effect in output {
                if let MercuryEffect::Foreground(app) = &effect
                    && installed.contains(app)
                {
                    runner.queue_event(foreground(*app));
                }
                performed.push(effect);
            }
        }
    }
}

// The whole flow in tandem: press n (nav), c (open Chrome; it comes up and the
// foreground event moves us to its in-app layer), r (restart), escape (home),
// space (typing), a (typed). Each key settles before the next, so `r` sees the
// in-app layer. Chrome is "installed", so opening it produces the follow-up.
#[test]
fn kitchen_sink() {
    let mut m = Mercury::default();
    let mut performed = Vec::new();
    {
        let mut runner = SimpleRunner::<MercuryStruct, _>::new(&mut m);
        for k in ["n", "c", "r", "escape", "space", "a"] {
            runner.queue_event(key(k));
            settle(&mut runner, &mut performed, &[App::Chrome]);
        }
    }
    assert_eq!(
        performed,
        vec![
            MercuryEffect::Foreground(App::Chrome),
            MercuryEffect::Command("r"),
            MercuryEffect::Type("a"),
        ]
    );
    assert!(matches!(m.layer, Layer::Typing(_)));
    assert_eq!(m.foregrounded, App::Chrome);
}

// If the app is not installed, `settle` queues no follow-up, so there is no
// foreground event: the state stays in nav and `r` there is unhandled.
#[test]
fn opening_a_missing_app_does_not_enter_in_app() {
    let mut m = Mercury::default();
    let mut performed = Vec::new();
    {
        let mut runner = SimpleRunner::<MercuryStruct, _>::new(&mut m);
        for k in ["n", "c", "r"] {
            runner.queue_event(key(k));
            settle(&mut runner, &mut performed, &[]);
        }
    }
    assert_eq!(performed, vec![MercuryEffect::Foreground(App::Chrome)]);
    assert!(matches!(m.layer, Layer::Nav(_)));
    assert_eq!(m.foregrounded, App::Other);
}
