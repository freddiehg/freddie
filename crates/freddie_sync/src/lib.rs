//! The two-phase sync of a mirrored value: [`Synced`], and the correlation token that pairs a
//! read with the placeholder awaiting it.

/// The placeholder's half of a correlation token: minted only beside its [`RidingGeneration`]
/// twin, moved once into the [`Synced::Pending`] placeholder, and spent when a riding half is
/// used against it.
///
/// Opaque: not `Copy`, not `Clone`, not comparable — no code outside this module can do
/// anything with one except put it where it belongs.
#[derive(Debug)]
pub struct HeldGeneration(u64);

/// The travelling half: minted beside its [`HeldGeneration`] twin, moved once into the read
/// effect the model emits, and carried home by the read's value event.
///
/// Opaque like its twin, and not publicly comparable: the only thing that can be done with one
/// is handing it to [`Synced::commit`]. It is used by reference, not consumed, because an
/// event may be handled multiple times, so dispatch hands every handler `&E` and nothing can
/// move a payload out of an event. Its single use is behavioral instead: it lives only inside
/// its value event, which dispatches once and is dropped, and a repeat meeting finds `Known`
/// or a mismatched `Pending` and does nothing.
#[derive(Debug)]
pub struct RidingGeneration(u64);

/// The model's generation mint: a counter on the root, so minting is a function of state — the
/// same shape as timer-guard creation, the model's one sanctioned impurity.
///
/// One per model, never per key: per-key counters restart when a key (a pid, a window id) is
/// reused by the OS, and a restarted counter could pair a zombie read with a fresh placeholder.
/// One counter makes that unrepresentable.
#[derive(Default, Debug)]
pub struct GenerationMinter(u64);

impl GenerationMinter {
    /// The matched pair: the placeholder's half, and the half that rides the read effect home.
    pub fn mint(&mut self) -> (HeldGeneration, RidingGeneration) {
        self.0 += 1;
        (HeldGeneration(self.0), RidingGeneration(self.0))
    }
}

/// One value synced from the outside world in two phases.
///
/// The fact ("it changed") empties the entry: the moment the fact exists, the old value is a
/// lie, and the placeholder holds one half of the token whose read is in flight. The value
/// ("this is what it changed to") fills the entry iff it brings the matching half home.
///
/// No ordering is owed by anyone: the placeholder is written in the same dispatch that emits
/// the read effect, so a value event cannot exist before its placeholder does.
#[derive(Debug)]
pub enum Synced<V> {
    /// The fact arrived; the read carrying this token's twin is in flight. No current value
    /// exists.
    Pending(HeldGeneration),
    /// What the last landed read answered.
    Known(V),
}

impl<V> Synced<V> {
    /// The fact: the old value dies, and the placeholder takes its half of the pair.
    pub fn change(&mut self, held: HeldGeneration) {
        *self = Self::Pending(held);
    }

    /// The value: applied iff the riding half matches the placeholder's. A slow read that
    /// lands after a newer fact brings a half whose twin was replaced and changes nothing.
    pub fn commit(&mut self, riding: &RidingGeneration, value: V) {
        if matches!(self, Self::Pending(HeldGeneration(held)) if *held == riding.0) {
            *self = Self::Known(value);
        }
    }

    /// The current value, if the sync has one. `Pending` is "no value right now", never the
    /// old one.
    #[must_use]
    pub const fn known(&self) -> Option<&V> {
        match self {
            Self::Known(v) => Some(v),
            Self::Pending(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GenerationMinter, Synced};

    #[test]
    fn the_happy_pair_lands() {
        let mut mint = GenerationMinter::default();
        let (held, riding) = mint.mint();
        let mut entry: Synced<u32> = Synced::Pending(held);
        entry.commit(&riding, 7);
        assert_eq!(entry.known(), Some(&7));
    }

    #[test]
    fn a_commit_after_a_newer_fact_changes_nothing() {
        let mut mint = GenerationMinter::default();
        let (first_held, first_riding) = mint.mint();
        let mut entry: Synced<u32> = Synced::Pending(first_held);
        let (second_held, second_riding) = mint.mint();
        entry.change(second_held);
        entry.commit(&first_riding, 7);
        assert_eq!(entry.known(), None);
        entry.commit(&second_riding, 9);
        assert_eq!(entry.known(), Some(&9));
    }

    #[test]
    fn a_foreign_riding_half_does_not_land() {
        let mut mint = GenerationMinter::default();
        let (held, _riding) = mint.mint();
        let (_other_held, other_riding) = mint.mint();
        let mut entry: Synced<u32> = Synced::Pending(held);
        entry.commit(&other_riding, 7);
        assert_eq!(entry.known(), None);
    }

    #[test]
    fn a_late_half_meets_a_known_entry_and_nothing_moves() {
        let mut mint = GenerationMinter::default();
        let (held, riding) = mint.mint();
        let mut entry: Synced<u32> = Synced::Pending(held);
        entry.commit(&riding, 7);
        let (_late_held, late_riding) = mint.mint();
        entry.commit(&late_riding, 8);
        assert_eq!(entry.known(), Some(&7));
    }
}
