//! Two kinds of test. The per-event ones send one event and assert the effect
//! (and resulting state) straight from `handle`. The loop ones go through
//! `drive` with a `Recorder` effect handler, which performs effects and — only
//! for apps it "has installed" — re-dispatches the foreground follow-up, the way
//! the real machine would.

use mercury::{
    App, AppLayer, EffectHandler, Layer, Mercury, MercuryEffect, drive, foreground, key,
};

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
fn terminals_rebind_d_to_command() {
    let mut m = Mercury::default();
    m.handle(&foreground(App::Tty));
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

// ---- loop: drive through a fallible effect handler ----

/// Records every effect it performs. A `Foreground` effect only produces the
/// follow-up foreground event when the app is installed; opening a missing app
/// records the attempt and stops there, so the state never enters that in-app
/// layer.
struct Recorder {
    performed: Vec<MercuryEffect>,
    installed: Vec<App>,
}

impl Recorder {
    fn new(installed: &[App]) -> Self {
        Self {
            performed: Vec::new(),
            installed: installed.to_vec(),
        }
    }
}

impl EffectHandler for Recorder {
    fn handle(&mut self, effect: &MercuryEffect, state: &mut Mercury) -> Vec<MercuryEffect> {
        self.performed.push(effect.clone());
        if let MercuryEffect::Foreground(app) = effect
            && self.installed.contains(app)
        {
            return state.handle(&foreground(*app)).unwrap_or_default();
        }
        Vec::new()
    }
}

fn run(state: &mut Mercury, rec: &mut Recorder, keys: &[&'static str]) {
    for k in keys {
        drive(state, &key(k), rec);
    }
}

// Everything in tandem: home -> n (nav) -> c (open Chrome; the app comes up and
// its foreground event re-enters, moving to the in-app layer) -> r (restart) ->
// escape (home) -> space (typing) -> a (typed).
#[test]
fn kitchen_sink() {
    let mut m = Mercury::default();
    let mut rec = Recorder::new(&[App::Chrome]);
    run(
        &mut m,
        &mut rec,
        &["n", "c", "r", "escape", "space", "a"],
    );
    assert_eq!(
        rec.performed,
        vec![
            MercuryEffect::Foreground(App::Chrome),
            MercuryEffect::Command("r"),
            MercuryEffect::Type("a"),
        ]
    );
    assert!(matches!(m.layer, Layer::Typing(_)));
    assert_eq!(m.foregrounded, App::Chrome);
}

// If the handler cannot open the app, there is no foreground follow-up: the
// state stays in nav, and the app's in-app key does nothing.
#[test]
fn opening_a_missing_app_does_not_enter_in_app() {
    let mut m = Mercury::default();
    let mut rec = Recorder::new(&[]); // nothing installed
    run(&mut m, &mut rec, &["n", "c", "r"]);
    assert_eq!(rec.performed, vec![MercuryEffect::Foreground(App::Chrome)]);
    assert!(matches!(m.layer, Layer::Nav(_)));
    assert_eq!(m.foregrounded, App::Other);
}
