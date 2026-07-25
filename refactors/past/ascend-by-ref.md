# ascend by shared reference

`laserbeam` walks up a path to an ancestor one way: by consuming it. A handler that wants to READ an ancestor and keep using its own node has no reach, because ascending consumes the whole chain and the leaf is gone.

The by-reference read already exists one level at a time. `PathMut::parent(&self) -> &Parent` and `PathMut::get(&self) -> &Node` walk up and read through a shared borrow. What is missing is the generic-depth version: chaining `parent().parent()...` to a named ancestor.

We add it as a second trait, mirroring `get`/`get_mut`:

```rust
pub trait Ascend<Target> {
    fn ascend(&self) -> &Target;   // borrows, leaves the path alive
}

pub trait AscendMut<Target> {
    fn ascend_mut(self) -> Target;  // consumes, walks up by value
}
```

`ascend` takes `&self` and returns `&Target`. For a `PathMut` ancestor, `Target` is that ancestor's path type and the caller reads through it with `.get()`. For the root, whose path type is `MercuryPath<'a> = &'a mut Mercury`, `&Target` is `&&mut Mercury`.

Every mercury handler consumes the path (it walks up to mutate the root, or hands the root to `and_go_home_from`, which needs `&mut Mercury`), so their bounds are `AscendMut<MercuryPath<'a>>`. The by-reference `Ascend` is for a future handler that reads an ancestor and then keeps using its own node; no current handler does that, so a leaf handler stays on `ascend_mut`. (Converting one to `ascend` trips `clippy::needless_pass_by_value`: dispatch hands a handler its `Node` by value, and only consuming it clears the lint.)

## Change 1: rename the consuming walk to `ascend_mut`

Prefactor. No new behavior; it frees the name `ascend`. Atomic: the laserbeam rename and every mercury callsite move in one commit, because the old name disappears. `Ascend::ascend(self) -> Target` becomes `Ascend::ascend_mut(self) -> Target`, the `ascend_to` sugar becomes `ascend_to_mut`, and every `.ascend()` in `crates/mercury/src/handlers/` becomes `.ascend_mut()`. The generic bounds `P: Ascend<MercuryPath<'a>>` are untouched.

## Change 2: add the by-reference trait

Split the single `Ascend` into two traits: `Ascend<Target>` carries `ascend(&self) -> &Target`, and a new `AscendMut<Target>` carries `ascend_mut(self) -> Target`. The reflexive impl and the per-depth macro each emit both impls; the ref side walks up with a `parent()` chain (`ascend_up_ref!`) where the mut side uses `into_parent()` (`ascend_up!`).

Both traits, after:

```rust
pub trait Ascend<Target> {
    fn ascend(&self) -> &Target;
}

pub trait AscendMut<Target> {
    fn ascend_mut(self) -> Target;
}

/// Every path is its own ancestor, at depth zero.
impl<T> Ascend<T> for T {
    fn ascend(&self) -> &T {
        self
    }
}

impl<T> AscendMut<T> for T {
    fn ascend_mut(self) -> T {
        self
    }
}
```

The by-reference companion to `ascend_up!`:

```rust
/// One `parent()` per type parameter, the shared-borrow mirror of `ascend_up!`.
macro_rules! ascend_up_ref {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        ascend_up_ref!($e.parent() $(, $rest)*)
    };
}
```

The per-depth macro emits one impl of each trait:

```rust
macro_rules! ascend_impls {
    ([$($acc:ident),*]) => {};
    ([$($acc:ident),*], $head:ident $(, $rest:ident)*) => {
        impl<T, $($acc,)* $head> Ascend<T> for ascend_nest!(T $(, $acc)*, $head) {
            fn ascend(&self) -> &T {
                ascend_up_ref!(self $(, $acc)*, $head)
            }
        }
        impl<T, $($acc,)* $head> AscendMut<T> for ascend_nest!(T $(, $acc)*, $head) {
            fn ascend_mut(self) -> T {
                ascend_up!(self $(, $acc)*, $head)
            }
        }
        ascend_impls!([$($acc,)* $head] $(, $rest)*);
    };
}
```

The sugar splits: `ascend_to<Target>(&self) -> &Target where Self: Ascend<Target>` and `ascend_to_mut<Target>(self) -> Target where Self: AscendMut<Target>`.

Every mercury handler's bound moves from `Ascend<MercuryPath<'a>>` to `AscendMut<MercuryPath<'a>>`, and `use laserbeam::Ascend;` becomes `use laserbeam::AscendMut;`.

### tests

`ascend_tests` bounds its reachability helpers on both traits (`P: Ascend<T> + AscendMut<T>`), so the twelve-level check asserts both reaches at once. A value-level test in the `tests` module reads an ancestor through `ascend` and confirms the leaf survives:

```rust
    #[test]
    fn ascend_reads_an_ancestor_by_shared_ref() {
        type Outer<'a> = PathMut<Attack, &'a mut Sheer>;
        let mut album = Sheer {
            heart: Attack { length: 7 },
        };
        let outer: Outer = PathMut::from_fn(&mut album, |a| &mut a.heart, |a| &a.heart);
        let mut inner: PathMut<u32, Outer> =
            PathMut::from_fn(outer, |p| &mut p.get_mut().length, |p| &p.get().length);

        let attack: &Outer = inner.ascend_to::<Outer>();
        assert_eq!(attack.get().length, 7);

        *inner.get_mut() += 1;
        assert_eq!(*inner.get(), 8);
    }
```

## Change 3: make `AscendMut` a subtrait of `Ascend`

A consuming ascender can always also borrow-ascend, so `AscendMut<Target>: Ascend<Target>`. This lets a handler that holds one `AscendMut` bound also read an ancestor by reference from the same bound.

```rust
pub trait AscendMut<Target>: Ascend<Target> {
    fn ascend_mut(self) -> Target;
}
```

Every `AscendMut` impl already has a matching `Ascend` impl (the macro and the reflexive impl emit both), so the supertrait bound is satisfied without new impls.
