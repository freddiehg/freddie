//! The binding layer: accumulate the active trigger set over a laserbeam tree.
//!
//! A node derives [`Bind`] and declares its marker with `#[binds(Marker)]` and
//! its bindings with `#[bind(trigger, handler)]`. The derive implements
//! [`EventHandler`], whose [`accumulate`](EventHandler::accumulate) inserts the
//! node's own triggers and recurses into its `#[resolve_into]` fields and active
//! enum variant. [`accumulate()`] runs it from the root.
#![allow(clippy::implicit_hasher)]

use std::collections::HashSet;
use std::hash::Hash;

pub use bind_macro::Bind;

/// The marker an app implements on one type to name its unified trigger enum.
pub trait Bindings {
    /// The unified enum of every trigger the app can register.
    type Trigger: Clone + Eq + Hash;
}

/// A bindable node. `#[derive(Bind)]` implements it.
pub trait EventHandler<M: Bindings> {
    /// Adds this node's triggers, and those of its active descendants, to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`BindError::DuplicateTrigger`] when a trigger is bound at more
    /// than one node on the active path.
    fn accumulate(&self, out: &mut HashSet<M::Trigger>) -> Result<(), BindError>;
}

/// The error [`accumulate()`] can produce.
#[derive(Debug, PartialEq, Eq)]
pub enum BindError {
    /// A trigger was bound at more than one node on the active path.
    DuplicateTrigger,
}

/// Inserts `t` into `out`, failing when it is already present.
///
/// # Errors
///
/// Returns [`BindError::DuplicateTrigger`] when `t` is already in `out`.
pub fn insert_or_error<T: Eq + Hash>(out: &mut HashSet<T>, t: T) -> Result<(), BindError> {
    if out.insert(t) {
        Ok(())
    } else {
        Err(BindError::DuplicateTrigger)
    }
}

/// Accumulates the active trigger set for the tree rooted at `root`.
///
/// # Errors
///
/// Propagates [`BindError::DuplicateTrigger`] from [`EventHandler::accumulate`].
pub fn accumulate<M, N>(root: &N) -> Result<HashSet<M::Trigger>, BindError>
where
    M: Bindings,
    N: EventHandler<M>,
{
    let mut out = HashSet::new();
    root.accumulate(&mut out)?;
    Ok(out)
}
