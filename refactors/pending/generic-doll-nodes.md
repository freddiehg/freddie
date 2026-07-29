# generic doll nodes

A shell that nests a child purely for separation takes the child as a type parameter. `TypingLayer` stops hard-coding `KinesisRemaps`; the concrete stack is spelled once, at the variant that holds it:

```rust
Layer::Typing(TypingLayer<KinesisRemaps<NumberRemaps<SymbolRemaps>>>)
```

Inserting, removing, or reordering a shell is then an edit to that one spelling and the affected `parent =` attributes, never to the inner shells' structs. The parent stays an attribute: a shell is generic over what it contains, and its position in the tree is the position's business.

## Change 1: `bind_macro` accepts generic place nodes

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
impl #impl_g ::laserbeam::HasPath for #name #ty_g { ... }
impl #impl_g ::bind::EventHandler<#marker> for #name #ty_g #where_clause { ... }
impl #impl_g ::bind::Dispatch<#marker> for #name #ty_g #where_clause { ... }
```

The generated bodies do not change. A generic impl typechecks against declared bounds, and the body's three child-path operations (the identity `.into()` after `from_fn`, the `Stop` fold, `into_parent` in accumulate's recover) are inherent methods on the concrete `PathMut`/`Stop` shapes — so the one thing the derive adds, for a `#[resolve_into]` child whose type names a node parameter, is the equality that makes the opaque type concrete. Any correctly parented node satisfies it definitionally, because the child's own derive emitted exactly that `Path`:

```rust
where
    #child: 'static + ::bind::Dispatch<#marker>,   // EventHandler in accumulate's clause
    for<'q> #child: ::laserbeam::HasPath<
        Path<'q> = ::laserbeam::PathMut<#child, <Self as ::laserbeam::HasPath>::Path<'q>>,
    >,
```

With that bound the compiler normalizes `<#child as HasPath>::Path` to the `PathMut` the body already builds, every inherent method resolves, and a `T` that does not fit fails at the instantiation with an ordinary trait error. The `'static` comes from the `for<'q>` binder; every node in both trees owns its data, so it costs nothing. This shape is verified end to end in a standalone mock (a generic shell over a GAT `HasPath`, the equality bound, an inherent `into_parent` resolving through the normalization, dispatch round-tripping).

For `TypingLayer<Next>` the three impls expand to:

```rust
#[automatically_derived]
impl<Next> ::laserbeam::HasPath for TypingLayer<Next> {
    type Path<'a>
        = ::laserbeam::PathMut<Self, LayerPath<'a>>
    where
        Self: 'a;
}

#[automatically_derived]
impl<Next> ::bind::EventHandler<FigaroStruct> for TypingLayer<Next>
where
    Next: 'static + ::bind::EventHandler<FigaroStruct>,
    for<'q> Next: ::laserbeam::HasPath<
        Path<'q> = ::laserbeam::PathMut<Next, <TypingLayer<Next> as ::laserbeam::HasPath>::Path<'q>>,
    >,
{
    fn accumulate<'a>(/* body unchanged */) -> /* unchanged */
    where
        Self: 'a,
    { /* unchanged */ }
}

#[automatically_derived]
impl<Next> ::bind::Dispatch<FigaroStruct> for TypingLayer<Next>
where
    Next: 'static + ::bind::Dispatch<FigaroStruct>,
    for<'q> Next: ::laserbeam::HasPath<
        Path<'q> = ::laserbeam::PathMut<Next, <TypingLayer<Next> as ::laserbeam::HasPath>::Path<'q>>,
    >,
{
    fn dispatch<'a, 'c>(/* body unchanged */) -> /* unchanged */
    where
        Self: 'a,
        <Self as ::laserbeam::HasPath>::Path<'a>: ::laserbeam::HasStop,
    { /* unchanged */ }
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

## Change 2: figaro's dolls take their child

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

## Change 3: mercury's doll takes its child

`AndReturnHome { layers: ReturnHomeLayers, guard: TimerGuard }` is the same shape with data beside the child; the child position goes generic:

```rust
pub struct AndReturnHome<Next> {
    #[resolve_into]
    layers: Next,
    pub(crate) guard: TimerGuard,
}
```

with the `Layer` variant re-spelled as `AndReturnHome<ReturnHomeLayers>`, `pub type AndReturnHomePath<'a> = PathMut<AndReturnHome<ReturnHomeLayers>, LayerPath<'a>>`, and its `enter` constructor generic over the same parameter. Mercury's `TypingLayer` nests nothing and is not a doll; it stays as it is.
