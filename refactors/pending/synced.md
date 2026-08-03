# `Synced<V>`: one type for the two-phase sync

Every mirror that syncs in two phases — a fact event saying the old value died, a value event carrying what it became — models its entry the same way and guards it against the same failure: a read that takes arbitrarily long and lands after newer facts must not clobber the newer state. Today that shape is about to be hand-rolled three times (`freddie_selection`'s entries, the window frames, the focused window). It is one generic type in the `freddie` crate, beside the timer machinery, because it is the same kind of thing: a pure model building block every consumer shares.

`crates/freddie/src/sync.rs`, exported from the crate root:

```rust
/// The generation of one synced value: minted by a watcher from its [`GenMinter`], globally
/// monotonic within that watcher for the life of the process. A generation never repeats, so a
/// stale value — however late it lands, whatever key reuse happened meanwhile — names a
/// generation no entry can be waiting for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Gen(u64);

/// A watcher's generation mint. One per watcher, never per key: per-key counters restart when
/// a key (a pid, a window id) is reused by the OS, and a restarted counter can alias a zombie
/// read into a fresh entry. One counter makes that unrepresentable.
#[derive(Default, Debug)]
pub struct GenMinter(u64);

impl GenMinter {
    pub fn next(&mut self) -> Gen {
        self.0 += 1;
        Gen(self.0)
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
    Pending(Gen),
    /// What the last landed read answered.
    Known(V),
}

impl<V> Synced<V> {
    /// The fact: the old value dies, this generation's read is awaited.
    pub fn changed(&mut self, gen: Gen) {
        *self = Self::Pending(gen);
    }

    /// The value: applied iff the entry still awaits exactly this generation. A slow read that
    /// lands after a newer fact names a generation the entry has moved past and changes
    /// nothing; applying the same landing twice equals once (the second finds `Known`).
    pub fn landed(&mut self, gen: Gen, value: V) {
        if matches!(self, Self::Pending(g) if *g == gen) {
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
```

Per-key collections are plain maps of `Synced<V>`: the fact arm does `entry.changed(gen)` (inserting `Pending(gen)` for a new key), the value arm `entry.landed(gen, v)` against an existing entry, and the key's disappearance removes the entry — removal is the map's, staleness is the entry's, and neither needs the other's logic.

Tests, in `sync.rs`: the happy pair lands; a landing after a newer fact changes nothing; the same landing twice equals once; a landing on a removed-and-reinserted entry (fresh `Pending`, new generation) changes nothing — the alias the global mint exists to kill.

## Adopters

The pending watcher docs consume this instead of their bespoke pieces, amended in the same commit that lands this crate change:

- `selection-watcher.md`: `SelectionGen` and `SelectionEntry` are replaced by `Gen` and `Synced<Selection>`; the watcher's per-pid counter table is replaced by one `GenMinter`; its callback section states the fact-before-request invariant.
- `windows-watcher-fixes.md`: `ReadGen`, `FrameEntry`, and `FocusEntry` likewise; same minter, same invariant.

figaro's `read-selection.md` and `sync-fixes.md` consume `freddie::{Gen, Synced}` through the same events.

## Order of changes

One change: the module and its tests. Pure, dependency-free, shippable ahead of every adopter.
