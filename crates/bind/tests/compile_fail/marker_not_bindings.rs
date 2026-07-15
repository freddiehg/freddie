// The `#[binds(..)]` type must implement `Bindings`. The node is a full laserbeam
// node, so the only failure is the missing `Bindings` impl.
use bind::Bind;

struct NotBindings;

#[derive(Bind)]
#[laserbeam_root]
#[binds(NotBindings)]
struct Nav {}

enum R<'a> {
    Nav(&'a mut Nav),
}

fn main() {}
