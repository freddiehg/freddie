# generic doll nodes

A shell that nests a child purely for separation takes the child as a type parameter. `TypingLayer` stops hard-coding `KinesisRemaps`; the concrete stack is spelled once, at the variant that holds it:

```rust
Layer::Typing(TypingLayer<KinesisRemaps<NumberRemaps<SymbolRemaps>>>)
```

Inserting, removing, or reordering a shell is then an edit to that one spelling and the affected `parent =` attributes, never to the inner shells' structs. The parent stays an attribute: a shell is generic over what it contains, and its position in the tree is the position's business.

## Change 1: `bind_macro` accepts generic nodes

`crates/bind_macro/src/lib.rs`, the rejection deleted:

```rust
fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "bind nodes may not be generic",
        ));
    }
```

becomes:

```rust
fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let (impl_generics, ty_generics, struct_where) = input.generics.split_for_impl();
```

with `#impl_generics`, `#name #ty_generics` threaded into every emitted impl (landed as a prefactor; non-generic nodes are unaffected). The per-child `Dispatch`/`EventHandler` bounds the derive already computes emit the parameter unchanged. That alone is not enough: three pieces of the generated body resolve only for a concrete child, and laserbeam grows one trait to cover them.

### 1a. laserbeam: the child fold as a trait

The generated dispatch folds the child's leave into the parent's state through `Stop::to_maybe_invalidated`, which today exists as inherent impls on the two concrete `Stop<PathMut<..>>` shapes; the child-path construction ends in `.into()` (an identity `From` for a concrete child); and the accumulate recover calls `PathMut::into_parent` inherently. For a child that is a type parameter, all three need trait paths:

```rust
/// A child path that folds into its parent's state: what the derive's descent needs from a
/// `#[resolve_into]` child whose type is a parameter.
pub trait ChildOf<Parent: HasStop>: HasStop + Sized {
    /// Build the child's path over its parent's, `PathMut::from_fn` under a trait name.
    fn descend(parent: Parent) -> Self;
    /// The child's leave, as the state it leaves behind at the parent
    /// (`Stop::to_maybe_invalidated` under a trait name).
    fn fold(completed: Completed<Self>) -> MaybeInvalidated<Parent>;
}
```

implemented in laserbeam for the two shapes the inherent impls cover today (a child of the root, a child of a non-root), with the inherent impls delegating to it. The derive's generic-child emission uses `ChildOf` for descent, fold, and recover, and bounds it:

```rust
impl<Next> ::bind::Dispatch<M> for TypingLayer<Next>
where
    Next: ::bind::Dispatch<M>,
    for<'q> <Next as ::bind::Place>::Path<'q>:
        ::laserbeam::ChildOf<<TypingLayer<Next> as ::bind::Place>::Path<'q>>,
{ ... }
```

The `for<'q>` form makes the parameter effectively `'static`, which every node in both trees already is (they own their data). `descend` closes over the field projection, so `ChildOf` is implemented via the same `from_fn` closures `Edge` emits today; the derive passes them through. For `TypingLayer<Next>` the three impls expand to:

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
{
    fn accumulate<'a>(/* unchanged body */) -> /* unchanged */
    where
        Self: 'a,
    { /* unchanged */ }
}

#[automatically_derived]
impl<Next> ::bind::Dispatch<FigaroStruct> for TypingLayer<Next>
where
    Next: ::bind::Dispatch<FigaroStruct>,
{
    fn dispatch<'a, 'c>(/* unchanged body */) -> /* unchanged */
    where
        Self: 'a,
        <Self as ::bind::Place>::Path<'a>: ::laserbeam::HasStop,
    { /* unchanged */ }
}
```

The `Self: 'a` bounds the impls already carry cover the parameter's lifetime. Derived levels (`#[derived_node]`) reject `#[resolve_into]` already and stay non-generic; the `Node<Parent, Self>` impls they emit are untouched.

A bind test pins the feature, in `crates/bind/tests/generic_shell.rs`, a two-shell tree where the outer shell is generic:

```rust
#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[bind(Key("o") => outer_key)]
pub struct Shell<Next> {
    #[resolve_into]
    pub next: Next,
}

#[derive(Bind)]
#[node(parent = ShellPath)]
#[binds(M)]
#[bind(Key("i") => inner_key)]
pub struct Inner;

pub type ShellPath<'a> = &'a mut Shell<Inner>;

#[test]
fn a_generic_shell_dispatches_into_its_parameter() {
    let mut s = Shell { next: Inner };
    assert_eq!(dispatch::<M, Shell<Inner>, _>(&mut s, &key("i")), vec![2]);
    assert_eq!(dispatch::<M, Shell<Inner>, _>(&mut s, &key("o")), vec![1]);
}
```

## Change 2: figaro's dolls take their child

The chain today is `TypingLayer { remaps: KinesisRemaps }` → `KinesisRemaps { next: NumberRemaps }` → `NumberRemaps { next: SymbolRemaps }` → `SymbolRemaps`, plus `AlwaysOnRemaps { next: Layer }` at the top. Each shell whose child exists only for separation gains the parameter; `SymbolRemaps` is the leaf and stays as it is.

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

`Layer::Typing(TypingLayer)` becomes `Layer::Typing(TypingStack)`, and every `TypingLayer::new()` call site (`boot_layer`, `to_typing`, the wispr entries, tests) compiles unchanged since the constructor lives on the alias. Handlers already bind through the path aliases or through `HasAncestor`/`IntoAncestor` bounds, so no handler signature changes; `p.get_mut().remaps` sites rename to `p.get_mut().next`.

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

with `Layer::WithTimeout(AndReturnHome<ReturnHomeLayers>)` (today's variant, re-spelled), `pub type AndReturnHomePath<'a> = PathMut<AndReturnHome<ReturnHomeLayers>, LayerPath<'a>>`, and its `enter` constructor generic over the same parameter. Mercury's `TypingLayer` nests nothing and is not a doll; it stays as it is.
