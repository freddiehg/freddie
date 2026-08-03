//! [`Synced`]: one type for the two-phase sync of a mirrored value.

/// One half of a correlation token: minted only in pairs by [`GenerationMinter`], compared and
/// nothing else.
///
/// Opaque and deliberately not `Copy`: one half lives in the entry that awaits a read, the
/// other rides the read and comes home in its report, and `commit` is the two halves meeting.
/// A generation never repeats within its minter, so a stale value — however late it lands,
/// whatever key reuse happened meanwhile — carries a half no entry can be holding.
///
/// `Clone` exists for exactly one hop: an event is dispatched by reference, so the half a fact
/// event carries is cloned once into the entry. It is not a license to fan out.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Generation(u64);

/// A watcher's generation mint.
///
/// One per watcher, never per key: per-key counters restart when a key (a pid, a window id) is
/// reused by the OS, and a restarted counter can alias a zombie read into a fresh entry. One
/// counter makes that unrepresentable.
#[derive(Default, Debug)]
pub struct GenerationMinter(u64);

impl GenerationMinter {
    /// The matched pair: one half for the fact, one to ride the value read home.
    pub fn mint(&mut self) -> (Generation, Generation) {
        self.0 += 1;
        (Generation(self.0), Generation(self.0))
    }
}

/// One value synced from the outside world in two phases.
///
/// The fact ("it changed") empties the entry: the moment the fact exists, the old value is a
/// lie, and there is no current value until the read lands. The value ("this is what it
/// changed to") fills it — but only the value the fact queued: anything else is a read that
/// outlived its truth.
///
/// The guard depends on one ordering invariant the watcher owes: the fact must be reported to
/// the consumer before the value read is queued, so the value can never reach the consumer's
/// queue ahead of its fact. A value arriving with no matching `Pending` is dropped, which is
/// correct for a superseded read and would wedge the entry for a reordered one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Synced<V> {
    /// The fact at this generation arrived; its value is in flight. No current value exists.
    Pending(Generation),
    /// What the last landed read answered.
    Known(V),
}

impl<V> Synced<V> {
    /// The fact: the old value dies, this generation's read is awaited.
    pub fn change(&mut self, generation: Generation) {
        *self = Self::Pending(generation);
    }

    /// The value: applied iff the entry still awaits exactly this generation. A slow read that
    /// lands after a newer fact names a generation the entry has moved past and changes
    /// nothing; applying the same landing twice equals once (the second finds `Known`).
    pub fn commit(&mut self, generation: &Generation, value: V) {
        if matches!(self, Self::Pending(g) if g == generation) {
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
    use std::collections::HashMap;

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
    fn the_same_commit_twice_equals_once() {
        let mut mint = GenerationMinter::default();
        let (held, riding) = mint.mint();
        let mut entry: Synced<u32> = Synced::Pending(held);
        entry.commit(&riding, 7);
        entry.commit(&riding, 8);
        assert_eq!(entry.known(), Some(&7));
    }

    #[test]
    fn a_commit_on_a_reinserted_entry_changes_nothing() {
        let mut mint = GenerationMinter::default();
        let mut map: HashMap<&str, Synced<u32>> = HashMap::new();
        let (old_held, old_riding) = mint.mint();
        map.insert("key", Synced::Pending(old_held));
        map.remove("key");
        let (fresh_held, _fresh_riding) = mint.mint();
        map.insert("key", Synced::Pending(fresh_held));
        map.get_mut("key")
            .expect("the entry was just inserted")
            .commit(&old_riding, 7);
        assert_eq!(map["key"].known(), None);
    }
}
