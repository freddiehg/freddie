// Over-peeling past the root: `&mut App` has no `into_parent`.
use laserbeam::PathMut;

struct App {
    layer: Layer,
}
struct Layer {
    nav: Nav,
}
struct Nav;

type AppPath<'a> = &'a mut App;
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;

fn main() {
    let mut app = App {
        layer: Layer { nav: Nav },
    };
    let nav: NavPath<'_> = PathMut::from_fn(
        PathMut::from_fn(&mut app, |a| &mut a.layer, |a| &a.layer),
        |lp| &mut lp.get_mut().nav,
        |lp| &lp.get().nav,
    );
    let _ = nav.into_parent().into_parent().into_parent();
}
