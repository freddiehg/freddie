# generic doll nodes

A shell that nests a child purely for separation takes the child as a type parameter. `TypingLayer` stops hard-coding `KinesisRemaps`; the concrete stack is spelled once, at the variant that holds it:

```rust
Layer::Typing(TypingLayer<KinesisRemaps<NumberRemaps<SymbolRemaps>>>)
```

Inserting, removing, or reordering a shell is then an edit to that one spelling and the affected `parent =` attributes, never to the inner shells' structs. The parent stays an attribute: a shell is generic over what it contains, and its position in the tree is the position's business.

Rust checks a generic impl at its definition against declared bounds, and the derive's generated body performs three operations on the child's path that today resolve through inherent impls on concrete types: the `.into()` building the child path, `Stop::to_maybe_invalidated` folding the child's leave into the parent's state, and `PathMut::into_parent` in accumulate's recover. A bound cannot name an inherent method, so the fold pair moves behind a laserbeam trait; the build needs only a `From` bound. With those bounds emitted, a `T` that does not fit fails at the instantiation with an ordinary trait error.

## Change 1: laserbeam — the fold behind a trait

`crates/laserbeam/src/lib.rs`. The two inherent `to_maybe_invalidated` impls keep their bodies; the trait gives them a name a bound can use, and carries the ascent beside the fold.

```rust
/// A path that folds its completed leave into its parent's state, and ascends to that parent.
/// What the derive's descent uses at a `#[resolve_into]` edge whose child is a type parameter.
pub trait FoldsInto<Parent>: Sized {
    /// The leave this path completed to, as the state it leaves behind at the parent.
    fn fold(completed: Completed<Self>) -> MaybeInvalidated<Parent>;
    /// The parent, one level up (`PathMut::into_parent` under a trait name).
    fn ascend(self) -> Parent;
}

/// A child of the root.
impl<'a, N, R> FoldsInto<&'a mut R> for PathMut<N, &'a mut R> {
    fn fold(completed: Completed<Self>) -> MaybeInvalidated<&'a mut R> {
        completed.into_inner().to_maybe_invalidated()
    }
    fn ascend(self) -> &'a mut R {
        self.into_parent()
    }
}

/// A child of a non-root.
impl<N, N2, Q: Above> FoldsInto<PathMut<N2, Q>> for PathMut<N, PathMut<N2, Q>> {
    fn fold(completed: Completed<Self>) -> MaybeInvalidated<PathMut<N2, Q>> {
        completed.into_inner().to_maybe_invalidated()
    }
    fn ascend(self) -> PathMut<N2, Q> {
        self.into_parent()
    }
}
```

## Change 2: `bind_macro` accepts generic place nodes

`crates/bind_macro/src/lib.rs`. The rejection in `expand` is deleted and the split generics thread through the three emitted impls:

```rust
fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "bind nodes may not be generic",
        ));
    }
```

becomes a set of narrower rejections — a derived level, a routed (multi-parent) child edge, and a node-level `where` clause stay non-generic, each with its own error — plus `let (impl_g, ty_g, _) = input.generics.split_for_impl();` in `place_impl`, `accumulate_impl`, and `dispatch_impl`, whose headers become:

```rust
impl #impl_g ::bind::Place for #name #ty_g { ... }
impl #impl_g ::bind::EventHandler<#marker> for #name #ty_g #where_clause { ... }
impl #impl_g ::bind::Dispatch<#marker> for #name #ty_g #where_clause { ... }
```

The child-edge emission stays one code path for concrete children. When the `#[resolve_into]` field's type names one of the node's type parameters, the edge emits through the trait instead:

- descent: unchanged construction, with the identity conversion named — the impl's where clause gains
  `for<'q> <#child as ::bind::Place>::Path<'q>: ::core::convert::From<::laserbeam::PathMut<#child, <Self as ::bind::Place>::Path<'q>>>`;
- fold: `::laserbeam::FoldsInto::fold(<#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim))` in place of `Completed::into_inner(..).to_maybe_invalidated()`, with
  `for<'q> <#child as ::bind::Place>::Path<'q>: ::laserbeam::FoldsInto<<Self as ::bind::Place>::Path<'q>>` in the where clause;
- recover (accumulate): `::laserbeam::FoldsInto::ascend(child)` in place of `child.into_parent()`, with the same `FoldsInto` bound beside the existing `#child: EventHandler<#marker>`.

The `for<'q>` form makes the parameter effectively `'static`; every node in both trees owns its data, so this costs nothing. For `TypingLayer<Next>` the three impls expand to:

```rust
#[automatically_derived]
impl<Next> ::bind::Place for TypingLayer<Next> {
    type Path<'a>
        = ::laserbeam::PathMut<Self, LayerPath<'a>>
    where
        Self: 'a;
}

#[automatically_derived]
impl<Next> ::bind::EventHandler<FigaroStruct> for TypingLayer<Next>
where
    Next: ::bind::EventHandler<FigaroStruct>,
    for<'q> <Next as ::bind::Place>::Path<'q>:
        ::core::convert::From<::laserbeam::PathMut<Next, <Self as ::bind::Place>::Path<'q>>>
        + ::laserbeam::FoldsInto<<Self as ::bind::Place>::Path<'q>>,
{
    fn accumulate<'a>(/* body as today, recover via FoldsInto::ascend */) -> /* unchanged */
    where
        Self: 'a,
    { /* unchanged otherwise */ }
}

#[automatically_derived]
impl<Next> ::bind::Dispatch<FigaroStruct> for TypingLayer<Next>
where
    Next: ::bind::Dispatch<FigaroStruct>,
    for<'q> <Next as ::bind::Place>::Path<'q>:
        ::core::convert::From<::laserbeam::PathMut<Next, <Self as ::bind::Place>::Path<'q>>>
        + ::laserbeam::FoldsInto<<Self as ::bind::Place>::Path<'q>>,
{
    fn dispatch<'a, 'c>(/* body as today, fold via FoldsInto::fold */) -> /* unchanged */
    where
        Self: 'a,
        <Self as ::bind::Place>::Path<'a>: ::laserbeam::HasStop,
    { /* unchanged otherwise */ }
}
```

A bind test pins the feature, `crates/bind/tests/generic_shell.rs`:

```rust
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
#[node(parent = ShellPath)]
#[binds(Demo)]
#[bind(Keyboard("i") => inner_key)]
pub struct Inner;

pub type ShellPath<'a> = &'a mut Shell<Inner>;
pub type InnerPath<'a> = PathMut<Inner, ShellPath<'a>>;

fn outer_key<'x, E>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, ShellPath<'x>>,
) -> (Vec<usize>, Completed<ShellPath<'x>>) {
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
```

## Change 3: figaro's dolls take their child

The chain is `TypingLayer { remaps: KinesisRemaps }` → `KinesisRemaps { next: NumberRemaps }` → `NumberRemaps { next: SymbolRemaps }` → `SymbolRemaps`, plus `AlwaysOnRemaps { next: Layer }` at the top; every edge is single-parent. Each shell gains the parameter; `SymbolRemaps` is the leaf and stays as it is.

`src/model/typing/mod.rs`, before:

```rust
pub struct TypingLayer {
    pub jk: DeviceSequence,
    #[resolve_into]
    pub remaps: KinesisRemaps,
}

impl TypingLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            jk: DeviceSequence::new(DeviceClass::Laptop, JK, JK_TIMEOUT),
            remaps: KinesisRemaps::new(NumberRemaps::new(SymbolRemaps::new())),
        }
    }
}
```

after:

```rust
pub struct TypingLayer<Next> {
    pub jk: DeviceSequence,
    #[resolve_into]
    pub next: Next,
}

/// The typing stack as figaro composes it. The one place the composition is spelled.
pub type TypingStack = TypingLayer<KinesisRemaps<NumberRemaps<SymbolRemaps>>>;

impl TypingStack {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            jk: DeviceSequence::new(DeviceClass::Laptop, JK, JK_TIMEOUT),
            next: KinesisRemaps::new(NumberRemaps::new(SymbolRemaps::new())),
        }
    }
}
```

`KinesisRemaps` and `NumberRemaps` follow the same shape: the struct gains `<Next>`, its `#[resolve_into]` field is named `next` and typed `Next`, and its `new` takes the child it already takes. The path aliases name the concrete compositions:

```rust
pub type TypingPath<'a> = PathMut<TypingStack, LayerPath<'a>>;
pub type KinesisRemapsPath<'a> =
    PathMut<KinesisRemaps<NumberRemaps<SymbolRemaps>>, TypingPath<'a>>;
pub type NumberRemapsPath<'a> = PathMut<NumberRemaps<SymbolRemaps>, KinesisRemapsPath<'a>>;
```

`Layer::Typing(TypingLayer)` becomes `Layer::Typing(TypingStack)`, and every `TypingLayer::new()` call site (`boot_layer`, `to_typing`, the wispr entries, tests) compiles unchanged since the constructor lives on the alias. Handlers bind through the path aliases or `HasAncestor`/`IntoAncestor` bounds, so no handler signature changes; `p.get_mut().remaps` sites rename to `p.get_mut().next`.

`src/model/always_on.rs`, the same move at the top of the tree:

```rust
pub struct AlwaysOnRemaps<Next> {
    #[resolve_into]
    pub next: Next,
}
```

with `Figaro { input: AlwaysOnRemaps<Layer> }`, `pub type AlwaysOnRemapsPath<'a> = PathMut<AlwaysOnRemaps<Layer>, FigaroPath<'a>>`, and `AlwaysOnRemaps::new(next: Next)` unchanged in body.

## Change 4: mercury's doll takes its child

`AndReturnHome { layers: ReturnHomeLayers, guard: TimerGuard }` is the same shape with data beside the child; the child position goes generic:

```rust
pub struct AndReturnHome<Next> {
    #[resolve_into]
    layers: Next,
    pub(crate) guard: TimerGuard,
}
```

with the `Layer` variant re-spelled as `AndReturnHome<ReturnHomeLayers>`, `pub type AndReturnHomePath<'a> = PathMut<AndReturnHome<ReturnHomeLayers>, LayerPath<'a>>`, and its `enter` constructor generic over the same parameter. Mercury's `TypingLayer` nests nothing and is not a doll; it stays as it is.
