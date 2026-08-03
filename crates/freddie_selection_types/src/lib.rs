//! The selection watcher's reported vocabulary: the pure data its facts and answers carry.

use freddie_windows_types::Pid;

/// What an app's focused element said when asked for its selected text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Selection {
    /// The selected text as the app reports it. Never empty: an empty answer is
    /// [`Empty`](Self::Empty).
    Text(String),
    /// The element answers the question, and nothing is selected.
    Empty,
    /// There is no focused element, or the focused element does not expose its selection
    /// through Accessibility.
    Unsupported,
}

/// What the watcher can tell you. Facts only: no values, no tokens — the consumer's model
/// requests the value as a read effect when it hears a fact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionChange {
    /// This app's selection changed (or its focus moved between elements): whatever the
    /// consumer knew for this pid is dead.
    Changed(Pid),
    /// The app is gone: remove the entry.
    AppGone(Pid),
}
