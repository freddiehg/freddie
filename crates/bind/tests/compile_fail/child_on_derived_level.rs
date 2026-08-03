// A derived level's `data` is rebuilt every dispatch and dies with it, so it cannot hang a place
// child: that child's leave would have to fold through a `DerivedLevel`, which is not a path.
use bind::{Bind, Bindings, EventTrigger};
use laserbeam::PathMut;

struct Demo;
impl Bindings for Demo {
    type Trigger = Key;
    type Event = Key;
    type Output = Vec<usize>;
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Key;
impl EventTrigger for Key {
    type Event = Self;
    fn is_matching(&self, _event: &Self) -> bool {
        true
    }
}

#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
struct Root {
    #[child]
    shell: Shell,
}

#[derive(Bind)]
#[node(parent_path = RootPath)]
#[binds(Demo)]
struct Shell;

#[derive(Bind)]
#[derived_node(parent_path = ShellPath)]
#[binds(Demo)]
struct Level {
    #[child]
    kept: Kept,
}

struct Kept;

type RootPath<'a> = &'a mut Root;
type ShellPath<'a> = PathMut<Shell, RootPath<'a>>;
fn main() {}
