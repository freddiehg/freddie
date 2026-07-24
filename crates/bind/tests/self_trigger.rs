//! [`bind::self_trigger!`] implements [`EventTrigger`] with `Event = Self` and match by
//! [`PartialEq`]. Unit signals always match; multi-variant tags only match their own variant.

use bind::EventTrigger;

/// Unit signal: one value, so equality is always true (mercury `Quit`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Quit;

bind::self_trigger!(Quit);

/// Tag enum: variants discriminate (figaro `DeviceClass`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    A,
    B,
}

bind::self_trigger!(Class);

#[test]
fn unit_signal_always_matches_itself() {
    assert!(Quit.is_matching(&Quit));
}

#[test]
fn tag_enum_matches_only_its_own_variant() {
    assert!(Class::A.is_matching(&Class::A));
    assert!(Class::B.is_matching(&Class::B));
    assert!(!Class::A.is_matching(&Class::B));
    assert!(!Class::B.is_matching(&Class::A));
}

#[test]
fn event_type_is_self() {
    // Compiles only if Event = Self for both shapes.
    fn assert_event_is_self<T: EventTrigger<Event = T>>() {}
    assert_event_is_self::<Quit>();
    assert_event_is_self::<Class>();
}
