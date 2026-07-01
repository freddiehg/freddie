// A trigger expression must lift into the marker's `Trigger` via `Into`.
use bind::{Bind, Bindings};

#[derive(Clone, PartialEq, Eq, Hash)]
struct Trig;

struct M;
impl Bindings for M {
    type Trigger = Trig;
}

// `Weird` has no `From`/`Into` for `Trig`.
struct Weird;

#[derive(Bind)]
#[binds(M)]
#[bind(Weird => noop)]
struct Nav {}

fn main() {}
