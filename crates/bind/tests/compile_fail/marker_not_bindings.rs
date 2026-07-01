// The `#[binds(..)]` type must implement `Bindings`.
use bind::Bind;

struct NotBindings;

#[derive(Bind)]
#[binds(NotBindings)]
struct Nav {}

fn main() {}
