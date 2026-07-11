// A trigger expression must lift into the marker's `Trigger` via `Into` (the
// accumulate half). `Weird` has `EventTrigger` and a valid handler, so dispatch
// is satisfied and the only failure is the missing `Into`.
use bind::{Bind, Bindings, EventTrigger};
use laserbeam::Laserbeam;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Trig;

struct Ev;
struct KeyEv;
impl<'a> TryFrom<&'a Ev> for &'a KeyEv {
    type Error = ();
    fn try_from(_: &'a Ev) -> Result<Self, ()> {
        Err(())
    }
}

struct M;
impl Bindings for M {
    type Trigger = Trig;
    type Event = Ev;
    type Output = ();
}

// `Weird` matches events but has no `From`/`Into` for `Trig`.
struct Weird;
impl EventTrigger for Weird {
    type Event = KeyEv;
    fn try_match(&self, _: &KeyEv) -> bind::Match {
        bind::Match::DontHandle
    }
}

fn handler(_: &KeyEv, _path: impl Sized) {}

#[derive(Laserbeam, Bind)]
#[laserbeam_root(resolved = R)]
#[binds(M)]
#[bind(Weird => handler)]
struct Nav {}

enum R<'a> {
    Nav(&'a mut Nav),
}

fn main() {}
