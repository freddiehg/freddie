//! Two kinds of test. The per-event ones send one event and assert the effect
//! (and resulting state) straight from `handle`. The loop ones go through a
//! little `drive` loop (a synchronous drain loop for the tests; the real CLI's
//! loop is different) with a handler that performs effects and — only for apps it
//! "has installed" — pushes the foreground follow-up, the way the real machine would.

use std::collections::VecDeque;

use mercury::{App, AppLayer, Layer, Mercury, MercuryEffect, MercuryEvent, foreground, key};

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

// ---- loop: the little event loop a consumer writes ----

/// Drain `events`, and for each effect call `handle`, which may push follow-up
/// events onto the queue. This is the synchronous loop the tests use; the real
/// CLI runs a different one, fed by OS listeners and blocking when idle. freddie
/// has no generic version; `Mercury::handle` (dispatch) is the piece underneath.
fn drive(
    m: &mut Mercury,
    events: impl IntoIterator<Item = MercuryEvent>,
    mut handle: impl FnMut(MercuryEffect, &mut VecDeque<MercuryEvent>),
) {
    let mut queue: VecDeque<MercuryEvent> = events.into_iter().collect();
    while let Some(event) = queue.pop_front() {
        if let Some(output) = m.handle(&event) {
            for effect in output {
                handle(effect, &mut queue);
            }
        }
    }
}

// The whole flow in tandem: home -> n (nav) -> c (open Chrome; the app comes up,
// its foreground event is pushed and moves to the in-app layer) -> r (restart)
// -> escape (home) -> space (typing) -> a (typed). Chrome is "installed", so the
// open produces the foreground follow-up.
#[test]
fn kitchen_sink() {
    let mut m = Mercury::default();
    let installed = [App::Chrome];
    let mut performed = Vec::new();
    drive(
        &mut m,
        [
            key("n"),
            key("c"),
            key("r"),
            key("escape"),
            key("space"),
            key("a"),
        ],
        |effect, queue| {
            if let MercuryEffect::Foreground(app) = &effect
                && installed.contains(app)
            {
                queue.push_front(foreground(*app));
            }
            performed.push(effect);
        },
    );
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

// If the handler cannot open the app, it pushes no follow-up, so there is no
// foreground event: the state stays in nav and the app's in-app key does nothing.
#[test]
fn opening_a_missing_app_does_not_enter_in_app() {
    let mut m = Mercury::default();
    let mut performed = Vec::new();
    drive(&mut m, [key("n"), key("c"), key("r")], |effect, _queue| {
        performed.push(effect); // nothing installed: never a follow-up
    });
    assert_eq!(performed, vec![MercuryEffect::Foreground(App::Chrome)]);
    assert!(matches!(m.layer, Layer::Nav(_)));
    assert_eq!(m.foregrounded, App::Other);
}
