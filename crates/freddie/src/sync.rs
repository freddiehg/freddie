//! [`Synced`]: one type for the two-phase sync of a mirrored value.

/// The generation of one synced value: minted by a watcher from its [`GenerationMinter`],
/// globally monotonic within that watcher for the life of the process.
///
/// A generation never repeats, so a stale value — however late it lands, whatever key reuse
/// happened meanwhile — names a generation no entry can be waiting for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Generation(u64);

/// A watcher's generation mint.
///
/// One per watcher, never per key: per-key counters restart when a key (a pid, a window id) is
/// reused by the OS, and a restarted counter can alias a zombie read into a fresh entry. One
/// counter makes that unrepresentable.
#[derive(Default, Debug)]
pub struct GenerationMinter(u64);

impl GenerationMinter {
    pub fn mint(&mut self) -> Generation {
        self.0 += 1;
        Generation(self.0)
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    pub fn commit(&mut self, generation: Generation, value: V) {
        if matches!(self, Self::Pending(g) if *g == generation) {
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
        let generation = mint.mint();
        let mut entry: Synced<u32> = Synced::Pending(generation);
        entry.commit(generation, 7);
        assert_eq!(entry.known(), Some(&7));
    }

    #[test]
    fn a_commit_after_a_newer_fact_changes_nothing() {
        let mut mint = GenerationMinter::default();
        let first = mint.mint();
        let mut entry: Synced<u32> = Synced::Pending(first);
        let second = mint.mint();
        entry.change(second);
        entry.commit(first, 7);
        assert_eq!(entry, Synced::Pending(second));
        entry.commit(second, 9);
        assert_eq!(entry.known(), Some(&9));
    }

    #[test]
    fn the_same_commit_twice_equals_once() {
        let mut mint = GenerationMinter::default();
        let generation = mint.mint();
        let mut entry: Synced<u32> = Synced::Pending(generation);
        entry.commit(generation, 7);
        entry.commit(generation, 8);
        assert_eq!(entry.known(), Some(&7));
    }

    #[test]
    fn a_commit_on_a_reinserted_entry_changes_nothing() {
        let mut mint = GenerationMinter::default();
        let mut map: HashMap<&str, Synced<u32>> = HashMap::new();
        let old = mint.mint();
        map.insert("key", Synced::Pending(old));
        map.remove("key");
        let fresh = mint.mint();
        map.insert("key", Synced::Pending(fresh));
        map.get_mut("key")
            .expect("the entry was just inserted")
            .commit(old, 7);
        assert_eq!(map["key"], Synced::Pending(fresh));
    }
}
