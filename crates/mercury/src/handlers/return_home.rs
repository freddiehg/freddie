//! The return-home deadline: the one concern the wrapper node owns.

use freddie_keys::KeyEvent;
use laserbeam::{Complete, Completed, PathMut};

use crate::MercuryEffect;
use crate::state::{AndReturnHome, LayerPath, arm_return_home};

/// Any key, whoever claimed it: push the deadline out if you are still in the layer.
///
/// A post, so it runs beside whatever gesture claimed the key, and it reads the descent's answer
/// rather than the claim. On a stay it overwrites the guard with a freshly armed one, and that
/// overwrite IS the cancel: dropping a guard cancels its timer through freddie's cancel channel.
/// On a leave there is nothing to do, because `set_layer` already swapped this node away and
/// dropped the guard with it.
// Bound on the generic wrapper, so it is generic over the layers it does not inspect.
pub(crate) fn home_deadline<'x, Next: 'static>(
    _ev: &KeyEvent,
    _snap: (),
    mut p: PathMut<AndReturnHome<Next>, LayerPath<'x>>,
) -> (
    Vec<MercuryEffect>,
    Completed<PathMut<AndReturnHome<Next>, LayerPath<'x>>>,
) {
    let (guard, arm) = arm_return_home();
    p.get_mut().guard = guard;
    (vec![arm], p.complete())
}
