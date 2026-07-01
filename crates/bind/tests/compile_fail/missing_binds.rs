// A node with `#[bind]` but no `#[binds(Marker)]` is rejected by the derive.
use bind::Bind;

#[derive(Bind)]
#[bind(Keyboard("g") => noop)]
struct Nav {}

fn main() {}
