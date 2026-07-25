// A trigger expression must lift into the marker's `Trigger` via `Into` (the
// accumulate half). `Weird` has `EventTrigger` and a valid handler, so dispatch
// is satisfied and the only failure is the missing `Into`.
use bind::{AscendState, Bind, Bindings, EventTrigger};
use laserbeam::Completed;

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
    type Output = Vec<usize>;
}

// `Weird` matches events but has no `From`/`Into` for `Trig`.
struct Weird;
impl EventTrigger for Weird {
    type Event = KeyEv;
    fn is_matching(&self, _: &KeyEv) -> bool {
        false
    }
}

fn handler<'x>(
    _: &KeyEv,
    _snap: (),
    st: AscendState<'_, &'x mut Nav>,
) -> (Vec<usize>, Completed<&'x mut Nav>) {
    (vec![0], st.complete())
}

#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[bind(Weird => handler)]
struct Nav {}

enum R<'a> {
    Nav(&'a mut Nav),
}

fn main() {}
