//! A shell node generic over its child: the doll shape, `Shell<Inner>`, where the child
//! exists only for separation and the composition is spelled at the instantiation.

mod common;

use bind::{AscendState, Bind};
use common::{Demo, Keyboard, key};
use laserbeam::{Completed, PathMut};

#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
#[bind(Keyboard("o") => outer_key)]
pub struct Shell<Next> {
    #[resolve_into]
    pub next: Next,
}

#[derive(Bind)]
#[node(parent_path = ShellPath)]
#[binds(Demo)]
#[bind(Keyboard("i") => inner_key)]
pub struct Inner;

pub type ShellPath<'a> = &'a mut Shell<Inner>;
pub type InnerPath<'a> = PathMut<Inner, ShellPath<'a>>;

// Bound on the generic shell, so it is generic over the parameter it does not inspect.
fn outer_key<'x, E, Next: 'static>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, &'x mut Shell<Next>>,
) -> (Vec<usize>, Completed<&'x mut Shell<Next>>) {
    (vec![1], st.complete())
}

fn inner_key<'x, E>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, InnerPath<'x>>,
) -> (Vec<usize>, Completed<InnerPath<'x>>) {
    (vec![2], st.complete())
}

#[test]
fn a_generic_shell_dispatches_into_its_parameter() {
    let mut s = Shell { next: Inner };
    assert_eq!(
        bind::dispatch::<Demo, Shell<Inner>, _>(&mut s, &key("i")),
        vec![2]
    );
    assert_eq!(
        bind::dispatch::<Demo, Shell<Inner>, _>(&mut s, &key("o")),
        vec![1]
    );
    assert_eq!(
        bind::dispatch::<Demo, Shell<Inner>, _>(&mut s, &key("x")),
        vec![]
    );
}
