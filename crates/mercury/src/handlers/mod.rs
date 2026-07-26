//! The key and foreground handlers, one module per layer.
//!
//! Each is the one scheduled shape,
//! `fn(&SourceEvent, Snap, AscendState<P>) -> (Vec<MercuryEffect>, Completed<P>)`: the event, what
//! its pre snapped before the descent, and what the descent left of the path it is bound on. It
//! returns inert effects and the leave it completed to. `crate::state` glob-imports this module so
//! the derive-generated dispatch can name them.
//!
//! A handler is a UNIT: one job, and the whole of it. A user action with parts is not several
//! handlers on several triggers, and not a helper that chains two mutations; it is one bind whose
//! right-hand side is `and!(..)` of its units, so the parts share the one claim, their effects
//! land in call order, and each sees the state the one before it left. Nothing composes in
//! `Mercury::handle` after the fact.
//!
//! What a unit touches decides its shape. A pure effect is branch-free and completes where it
//! stands. A unit that writes the root consumes the state to get there
//! (`st.state.into_ancestor()`, total on both branches) and completes at the root, which is what
//! tells everything above it that the layer it was bound on is gone. A unit that only reads the
//! tree reaches up by shared reference and stays.

mod app;
mod foreground;
mod home;
mod nav;
mod overlay;
mod quit;
mod resize;
mod return_home;
mod root;
mod tab;
mod typing;
mod window;

pub(crate) use app::*;
pub(crate) use foreground::*;
pub(crate) use home::*;
pub(crate) use nav::*;
pub(crate) use overlay::*;
pub(crate) use quit::*;
pub(crate) use resize::*;
pub(crate) use return_home::*;
pub(crate) use root::*;
pub(crate) use tab::*;
pub(crate) use typing::*;
pub(crate) use window::*;
