use bind::{Bind, if_not_invalidated};
use freddie::TimerGuard;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{AnyKey, MercuryEffect, MercuryStruct};

use super::{
    AndReturnHomePath, AppLayer, LayerPath, NavLayer, ResizeLayer, SiteLayer, arm_return_home,
};

/// The layers that return home after [`RETURN_TO_HOME_TIMEOUT`](super::RETURN_TO_HOME_TIMEOUT) of
/// idle, wrapped in the one timer they share.
///
/// It owns the guard, the firing that goes home, and the deadline post that pushes the deadline
/// out on a key that kept you in the layer; its [`layers`](Self::layers) is which such layer is
/// active. Home and typing are not here: home is the destination the timer returns to, and typing
/// is passthrough, so neither carries a timer. An untimed layer is therefore unrepresentable in
/// the deadline's domain.
///
/// The post sits HERE rather than on the leaves, and that placement is what makes it correct: a
/// leaf's own `go_home` claim happens inside this node's descent, so the post sees the leave and
/// does nothing, where on a leaf it would run before that leaf's binds and rearm a layer about to
/// die.
#[derive(Bind, Debug)]
#[node(parent_path = LayerPath)]
#[binds(MercuryStruct)]
#[post(AnyKey => if_not_invalidated(home_deadline))]
#[bind(|path| path.get().guard.trigger() => if_not_invalidated(go_home))]
pub struct AndReturnHome<Next> {
    #[child]
    layers: Next,
    /// Read by the trigger matching its firing, and held for its `Drop`: dropping the guard
    /// cancels the return-home timer, which is how every rearm and every layer swap cancels.
    pub(crate) guard: TimerGuard,
}

impl<Next> AndReturnHome<Next> {
    /// Enter a return-home layer with its timer armed, returning the wrapper and the effect that
    /// schedules it.
    #[must_use]
    pub(crate) fn new(layers: impl Into<Next>) -> (Self, MercuryEffect) {
        let (guard, timer) = arm_return_home();
        (
            Self {
                layers: layers.into(),
                guard,
            },
            timer,
        )
    }

    /// `pub` because the integration test crate reads it to assert which layer is active.
    #[must_use]
    pub const fn layers(&self) -> &Next {
        &self.layers
    }
}

/// Which return-home layer is active. `derive_more::From` gives each leaf an
/// `Into<ReturnHomeLayers>`, so `AndReturnHome::new(NavLayer::new())` and the like construct it.
#[derive(Bind, Debug, derive_more::From)]
#[node(parent_path = AndReturnHomePath)]
#[binds(MercuryStruct)]
pub enum ReturnHomeLayers {
    Nav(NavLayer),
    Resize(ResizeLayer),
    InApp(AppLayer),
    Site(SiteLayer),
}
