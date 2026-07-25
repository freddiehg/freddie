//! Complete / peel on the shared App tree: the five leaves from
//! `path-peel-complete`, built with the same `PathMut::from_fn` projections
//! the derive emits.

mod common;

use common::{App, AppPath, Layer, LayerPath, Nav, NavPath};
use laserbeam::{Complete, Completed, PathMut, Stop};

const fn nav_app(nav_hits: u32, app_hits: u32) -> App {
    App {
        hits: app_hits,
        layer: Layer::Nav(Nav { hits: nav_hits }),
    }
}

fn layer_path(app: &mut App) -> LayerPath<'_> {
    PathMut::from_fn(app, |a| &mut a.layer, |a| &a.layer)
}

fn nav_path(app: &mut App) -> NavPath<'_> {
    PathMut::from_fn(
        layer_path(app),
        |lp| {
            let Layer::Nav(n) = lp.get_mut() else {
                unreachable!("test tree is Nav")
            };
            n
        },
        |lp| {
            let Layer::Nav(n) = lp.get() else {
                unreachable!("test tree is Nav")
            };
            n
        },
    )
}

#[test]
fn complete_at_nav() {
    let mut app = nav_app(7, 0);
    let out: Completed<NavPath<'_>> = nav_path(&mut app).complete();
    let Stop::Here(mut nav) = out.into_inner() else {
        panic!("expected Here");
    };
    assert_eq!(nav.get().hits, 7);
    nav.get_mut().hits = 8;
    drop(nav);
    let Layer::Nav(nav) = &app.layer else {
        unreachable!()
    };
    assert_eq!(nav.hits, 8);
}

#[test]
fn one_peel() {
    let mut app = nav_app(7, 0);
    let out: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
    let Stop::Up(rest) = out.into_inner() else {
        panic!("expected Up");
    };
    let Stop::Here(layer) = rest.into_inner() else {
        panic!("expected Up(Here(layer))");
    };
    let Layer::Nav(nav) = layer.get() else {
        unreachable!()
    };
    assert_eq!(nav.hits, 7);
}

#[test]
fn two_peels() {
    let mut app = nav_app(0, 0);
    {
        let out: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().into_parent().complete();
        let Stop::Up(rest) = out.into_inner() else {
            panic!("expected Up");
        };
        let Stop::Up(root) = rest.into_inner() else {
            panic!("expected Up(Up(app))");
        };
        root.hits = 3;
    }
    assert_eq!(app.hits, 3);
}

#[test]
fn layer_origin_bare_root() {
    let mut app = nav_app(0, 0);
    {
        let out: Completed<LayerPath<'_>> = layer_path(&mut app).into_parent().complete();
        let Stop::Up(root) = out.into_inner() else {
            panic!("expected Up(app)");
        };
        root.hits = 1;
    }
    assert_eq!(app.hits, 1);
}

#[test]
fn root_completes_bare() {
    let mut app = nav_app(0, 0);
    {
        let out: Completed<AppPath<'_>> = (&mut app).complete();
        let root = out.into_inner();
        root.hits = 5;
    }
    assert_eq!(app.hits, 5);
}
