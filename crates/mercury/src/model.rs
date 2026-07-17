//! The unified trigger, event, and marker the bindings hang off.

use bind::Bindings;
use freddie_keys::{Key, KeyEvent, KeyPress};

use crate::{
    AnyModifierKey, AnyNonModifierKey, ForegroundEvent, Foregrounded, MercuryEffect, Quit,
};

/// Every trigger Mercury can register, one variant per source.
#[derive(Clone, PartialEq, Eq, Hash, Debug, derive_more::From)]
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    AnyModifierKey(AnyModifierKey),
    AnyNonModifierKey(AnyNonModifierKey),
    Foregrounded(Foregrounded),
    Quit(Quit),
}

/// Every event Mercury can dispatch, one variant per source.
///
/// `TryInto` gives the `TryFrom<&MercuryEvent> for &SourceEvent` that dispatch uses to narrow
/// the unified event to the one a trigger cares about.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug, derive_more::TryInto)]
#[try_into(ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(Quit),
}

/// The marker tying the trigger, event, and output types together.
pub struct MercuryStruct;
impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
    type Event = MercuryEvent;
    type Output = Vec<MercuryEffect>;
}
