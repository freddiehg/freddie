//! The display watcher's reported vocabulary: the pure data its reports carry.

/// A display present according to macOS.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Display {
    /// The `CGDirectDisplayID`, stable for the life of the connection — which is all a consumer
    /// correlates over, since every report carries the full current set.
    pub id: DisplayId,
    /// `CGDisplayIsBuiltin`: whether this is the laptop's own panel.
    pub builtin: bool,
    /// The display's localized name (`NSScreen.localizedName`), which is what `BetterDisplay`'s
    /// `-name=` addresses.
    pub name: String,
}

/// A display's id for correlation within a connection session.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DisplayId(pub u32);
