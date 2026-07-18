//! Derive macro for `bind`: implements `EventHandler<M>` (accumulate) and
//! `Dispatch<M>` (dispatch).
//!
//! `#[derive(Bind)]` reads `#[binds(Marker)]` for the marker and the
//! `#[bind(trigger => handler, ..)]` pairs. `accumulate` inserts the node's
//! triggers and recurses into its `#[resolve_into]` fields and active enum
//! variant. `dispatch` tries the active child first (leafward, so a child's
//! binding beats an ancestor's), then the node's own binds, building each node's
//! laserbeam `Path` through the shared `derive_support::Edge`.

use derive_support::{
    Edge, Via, find_resolve_into, is_root, node_parent, parent_route, single_field_ty, unbox,
};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Expr, Ident, Path, Token, Type, parse_macro_input};

#[proc_macro_derive(
    Bind,
    attributes(binds, bind, resolve_into, derived_child, derived_node, node)
)]
pub fn derive_bind(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "bind nodes may not be generic",
        ));
    }
    let name = &input.ident;
    let marker = marker_of(input)?;
    let binds = binds(&input.attrs)?;

    // A DERIVED level is not a place in the tree. It has no `Resolve`, so it can have neither
    // `Dispatch` nor `EventHandler`, both of which take `Self::Path`. It implements `Descend`
    // on its `Node` instead.
    if let Some(parent) = derived_node_parent(&input.attrs)? {
        return derived_node_impl(input, name, &parent, &marker, &binds);
    }

    let place = place_impl(input, name)?;
    let accumulate = accumulate_impl(input, name, &marker, &binds)?;
    let dispatch = dispatch_impl(input, name, &marker, &binds)?;
    let descend = descend_impl(input, name, &marker);
    Ok(quote! {
        #place
        #accumulate
        #dispatch
        #descend
    })
}

/// Emits `impl bind::Place` for a place node: its path type, `PathMut<Self, Parent>` from
/// `#[node(parent = P)]`, or `&mut Self` for `#[node(root)]`. This is the associated type that
/// `Dispatch`, `EventHandler`, and the
/// place `Descend` impl all name.
fn place_impl(input: &DeriveInput, name: &Ident) -> syn::Result<TokenStream2> {
    let path_ty = if is_root(&input.attrs) {
        quote!(&'a mut Self)
    } else {
        let parent = node_parent(&input.attrs)?.ok_or_else(|| {
            syn::Error::new(
                input.ident.span(),
                "a bind node needs `#[node(parent = ..)]` or `#[node(root)]`",
            )
        })?;
        quote!(::laserbeam::PathMut<Self, #parent<'a>>)
    };
    Ok(quote! {
        #[automatically_derived]
        impl ::bind::Place for #name {
            type Path<'a>
                = #path_ty
            where
                Self: 'a;
        }
    })
}

/// The parent path named by `#[derived_node(parent = Alias)]`, if this level is not a place.
///
/// The derive is on the level's own struct and cannot see its parent, so it has to be told.
/// With the parent and its own name it can build `Node<ParentPath<'a>, Self>` itself, which is
/// why no node alias is needed.
fn derived_node_parent(attrs: &[syn::Attribute]) -> syn::Result<Option<Path>> {
    let mut found = None;
    for attr in attrs {
        if attr.path().is_ident("derived_node") {
            if found.is_some() {
                return Err(syn::Error::new(
                    attr.span(),
                    "expected one `#[derived_node(..)]`",
                ));
            }
            let mut parent = None;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("parent") {
                    parent = Some(meta.value()?.parse::<Path>()?);
                    Ok(())
                } else {
                    Err(meta.error("expected `parent = Alias`"))
                }
            })?;
            found = Some(parent.ok_or_else(|| {
                syn::Error::new(attr.span(), "`#[derived_node]` needs `parent = Alias`")
            })?);
        }
    }
    Ok(found)
}

/// The fn named by `#[derived_child(f)]`, if this node's child is not a field.
///
/// `f` is `fn(&Parent) -> Option<Data>`. A shared reference, so it cannot mutate the tree and
/// cannot consume the parent.
fn derived_child_fn(attrs: &[syn::Attribute]) -> syn::Result<Option<Path>> {
    let mut found = None;
    for attr in attrs {
        if attr.path().is_ident("derived_child") {
            if found.is_some() {
                return Err(syn::Error::new(
                    attr.span(),
                    "expected one `#[derived_child(..)]`",
                ));
            }
            found = Some(attr.parse_args::<Path>()?);
        }
    }
    Ok(found)
}

/// The enum case of [`derived_node_impl`]: one dispatch/accumulate arm per variant, each
/// rebuilding the node with the variant's `Data` and the shared parent.
fn derived_enum_node_impl(
    input: &DeriveInput,
    name: &Ident,
    parent: &Path,
    marker: &Path,
    binds: &[Binding],
    e: &syn::DataEnum,
) -> syn::Result<TokenStream2> {
    if !binds.is_empty() {
        return Err(syn::Error::new(
            input.span(),
            "an enum of derived levels binds nothing itself; put the binds on its variants",
        ));
    }
    let mut dispatch_arms = Vec::new();
    let mut acc_arms = Vec::new();
    for v in &e.variants {
        let vi = &v.ident;
        single_field_ty(&v.fields)?; // one Data per variant
        dispatch_arms.push(quote! {
            #name::#vi(data) => ::bind::Descend::<#marker>::dispatch(
                ::bind::Node { parent, data },
                event,
            ),
        });
        acc_arms.push(quote! {
            #name::#vi(data) => ::bind::DerivedHandler::<#marker>::accumulate(
                ::bind::Node { parent, data },
                out,
            ),
        });
    }
    Ok(quote! {
        #[automatically_derived]
        impl<'a> ::bind::Descend<#marker> for ::bind::Node<#parent<'a>, #name> {
            fn dispatch(
                self,
                event: &<#marker as ::bind::Bindings>::Event,
            ) -> ::core::ops::ControlFlow<
                <#marker as ::bind::Bindings>::Output,
                <Self as ::bind::HasParent>::Parent,
            > {
                let ::bind::Node { parent, data } = self;
                match data { #(#dispatch_arms)* }
            }
        }

        ::bind::check_only! {
        #[automatically_derived]
        #[expect(clippy::implicit_hasher)]
        impl<'a> ::bind::DerivedHandler<#marker> for ::bind::Node<#parent<'a>, #name> {
            fn accumulate(
                self,
                out: &mut ::std::collections::HashSet<
                    <#marker as ::bind::Bindings>::Trigger,
                >,
            ) -> ::core::result::Result<
                <Self as ::bind::HasParent>::Parent,
                ::bind::BindError,
            > {
                let ::bind::Node { parent, data } = self;
                match data { #(#acc_arms)* }
            }
        }
        }
    })
}

/// Emits `Descend` (and the check's half) for a DERIVED level: its own child, then its own
/// binds, then hand the parent back.
///
/// It never names its own node type. `Node<#parent<'a>, Self>` is built from the attribute and
/// from the struct the derive sits on.
fn derived_node_impl(
    input: &DeriveInput,
    name: &Ident,
    parent: &Path,
    marker: &Path,
    binds: &[Binding],
) -> syn::Result<TokenStream2> {
    // Several possible levels: the DATA is an enum. There is no separate mechanism. The derive
    // destructures per variant and rebuilds the node, so each variant's handler gets its own
    // `Data` and the parent is shared by construction.
    if let Data::Enum(e) = &input.data {
        return derived_enum_node_impl(input, name, parent, marker, binds, e);
    }

    let descend = derived_dispatch_descent(input, marker)?;
    let acc_descend = derived_accumulate_descent(input, marker)?;
    let checks = binds.iter().map(|b| {
        let trigger = trigger_expr(&b.trigger, &quote!(node));
        let handler = &b.handler;
        quote! {
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = #trigger;
                if ::bind::EventTrigger::is_matching(&trigger, ev) {
                    return ::core::ops::ControlFlow::Break(#handler(ev, node));
                }
            }
        }
    });
    let triggers = binds
        .iter()
        .map(|b| trigger_expr(&b.trigger, &quote!(node)));
    Ok(quote! {
        #[automatically_derived]
        #[expect(clippy::useless_conversion)]
        impl<'a> ::bind::Descend<#marker> for ::bind::Node<#parent<'a>, #name> {
            fn dispatch(
                self,
                event: &<#marker as ::bind::Bindings>::Event,
            ) -> ::core::ops::ControlFlow<
                <#marker as ::bind::Bindings>::Output,
                <Self as ::bind::HasParent>::Parent,
            > {
                let node = self;
                #descend
                #(#checks)*
                ::core::ops::ControlFlow::Continue(::bind::HasParent::into_parent(node))
            }
        }

        ::bind::check_only! {
        #[automatically_derived]
        #[expect(clippy::useless_conversion, clippy::implicit_hasher)]
        impl<'a> ::bind::DerivedHandler<#marker> for ::bind::Node<#parent<'a>, #name> {
            fn accumulate(
                self,
                out: &mut ::std::collections::HashSet<<#marker as ::bind::Bindings>::Trigger>,
            ) -> ::core::result::Result<
                <Self as ::bind::HasParent>::Parent,
                ::bind::BindError,
            > {
                let node = self;
                #(
                    ::bind::insert_or_error(out, ::core::convert::Into::into(#triggers))?;
                )*
                #acc_descend
                ::core::result::Result::Ok(::bind::HasParent::into_parent(node))
            }
        }
        }
    })
}

/// The `#[derived_child]` descent, for dispatch. Emitted on a PLACE and on a DERIVED level
/// alike, because both reach a child the same way once they hold it.
///
/// `f` is `fn(&Parent) -> Option<Data>`: a shared reference, so the parent is never moved and
/// never has to be handed back. The derive builds the node, and names no type it cannot see:
/// `data`'s type comes from `f`'s return, and inference resolves `Descend` from the `Node`.
fn derived_child_descent(f: &Path, marker: &Path, place: &TokenStream2) -> TokenStream2 {
    quote! {
        let #place = match #f(&#place) {
            ::core::option::Option::Some(data) => {
                ::bind::Descend::<#marker>::dispatch(
                    ::bind::Node { parent: #place, data },
                    event,
                )?
            }
            ::core::option::Option::None => #place,
        };
    }
}

/// The same descent, for the check.
fn derived_child_accumulate(f: &Path, marker: &Path, place: &TokenStream2) -> TokenStream2 {
    quote! {
        let #place = match #f(&#place) {
            ::core::option::Option::Some(data) => {
                ::bind::DerivedHandler::<#marker>::accumulate(
                    ::bind::Node { parent: #place, data },
                    out,
                )?
            }
            ::core::option::Option::None => #place,
        };
    }
}

fn derived_dispatch_descent(input: &DeriveInput, marker: &Path) -> syn::Result<TokenStream2> {
    Ok(derived_child_fn(&input.attrs)?.map_or_else(
        || quote!(),
        |f| derived_child_descent(&f, marker, &quote!(node)),
    ))
}

fn derived_accumulate_descent(input: &DeriveInput, marker: &Path) -> syn::Result<TokenStream2> {
    Ok(derived_child_fn(&input.attrs)?.map_or_else(
        || quote!(),
        |f| derived_child_accumulate(&f, marker, &quote!(node)),
    ))
}

/// Emits `impl Descend<M>` for a PLACE: delegate to its own `Dispatch`, then hand the parent
/// back.
///
/// Per node, and not a blanket `impl<N, P> Descend<M> for PathMut<N, P>`: `Dispatch` carries
/// `Self: 'a`, and the HRTB needed to state the blanket is E0311. Here the lifetime is named,
/// so `Self: 'a` holds.
///
/// The root has no parent to hand back, so it gets none.
fn descend_impl(input: &DeriveInput, name: &Ident, marker: &Path) -> TokenStream2 {
    if is_root(&input.attrs) {
        return quote!();
    }
    quote! {
        #[automatically_derived]
        impl<'a> ::bind::Descend<#marker> for <#name as ::bind::Place>::Path<'a>
        where
            #name: 'a,
        {
            fn dispatch(
                self,
                event: &<#marker as ::bind::Bindings>::Event,
            ) -> ::core::ops::ControlFlow<
                <#marker as ::bind::Bindings>::Output,
                <Self as ::bind::HasParent>::Parent,
            > {
                match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event) {
                    ::core::ops::ControlFlow::Break(out) => {
                        ::core::ops::ControlFlow::Break(out)
                    }
                    ::core::ops::ControlFlow::Continue(path) => {
                        ::core::ops::ControlFlow::Continue(
                            ::bind::HasParent::into_parent(path),
                        )
                    }
                }
            }
        }
    }
}

/// Emits `impl EventHandler<M>`: insert this node's triggers, then recurse.
///
/// It takes a path, exactly as `dispatch` does, and for the same reason: a level whose child
/// is produced by a function needs a path to call that function with. The two bodies descend
/// through the same `derive_support::Edge`, so they cannot drift.
fn accumulate_impl(
    input: &DeriveInput,
    name: &Ident,
    marker: &Path,
    binds: &[Binding],
) -> syn::Result<TokenStream2> {
    let root = is_root(&input.attrs);
    let (recurse, children, needs_mut) = accumulate_body(input, name, marker, root)?;
    let where_clause = if children.is_empty() {
        quote!()
    } else {
        quote!(where #(#children: ::bind::EventHandler<#marker>,)*)
    };
    // A closure trigger is called with `&mut path`, so it needs the binding to be `mut` even on a
    // node whose shape never reassigns it.
    let binding = if needs_mut || any_closure_trigger(binds) {
        quote!(mut path)
    } else {
        quote!(path)
    };
    let triggers = binds
        .iter()
        .map(|b| trigger_expr(&b.trigger, &quote!(path)));
    Ok(quote! {
        ::bind::check_only! {
        #[automatically_derived]
        #[expect(clippy::useless_conversion, clippy::implicit_hasher)]
        impl ::bind::EventHandler<#marker> for #name #where_clause {
            fn accumulate<'a>(
                #binding: <Self as ::bind::Place>::Path<'a>,
                out: &mut ::std::collections::HashSet<<#marker as ::bind::Bindings>::Trigger>,
            ) -> ::core::result::Result<
                <Self as ::bind::Place>::Path<'a>,
                ::bind::BindError,
            >
            where
                Self: 'a,
            {
                #(
                    ::bind::insert_or_error(out, ::core::convert::Into::into(#triggers))?;
                )*
                #recurse
                ::core::result::Result::Ok(path)
            }
        }
        }
    })
}

/// The accumulate recursion, the child types to bound, and whether the path binding needs
/// `mut`. Mirrors `dispatch_body`, minus the `ControlFlow`: accumulate never stops early.
fn accumulate_body(
    input: &DeriveInput,
    name: &Ident,
    marker: &Path,
    root: bool,
) -> syn::Result<(TokenStream2, Vec<Type>, bool)> {
    if let Some(f) = derived_child_fn(&input.attrs)? {
        return Ok((
            derived_child_accumulate(&f, marker, &quote!(path)),
            Vec::new(),
            false,
        ));
    }
    match &input.data {
        Data::Struct(s) => match find_resolve_into(&s.fields)? {
            None => Ok((quote!(), Vec::new(), false)),
            Some((field, child_ty, route)) => {
                let (child, boxed) = unbox(&child_ty);
                let edge = Edge {
                    parent: name,
                    is_root: root,
                    route: route.as_ref(),
                    boxed,
                    via: Via::Field(&field),
                };
                let child_path = edge.child_path(&quote!(path));
                let recover = edge.recover_parent(&quote!(child));
                let recurse = quote! {
                    let child =
                        <#child as ::bind::EventHandler<#marker>>::accumulate(#child_path, out)?;
                    path = #recover;
                };
                Ok((recurse, vec![child.clone()], true))
            }
        },
        Data::Enum(e) => {
            let mut arms = Vec::new();
            let mut children = Vec::new();
            for v in &e.variants {
                let vi = &v.ident;
                let ty = single_field_ty(&v.fields)?;
                let route = parent_route(&v.attrs)?;
                let (child, boxed) = unbox(&ty);
                children.push(child.clone());
                let edge = Edge {
                    parent: name,
                    is_root: root,
                    route: route.as_ref(),
                    boxed,
                    via: Via::Variant(vi),
                };
                let child_path = edge.child_path(&quote!(path));
                let recover = edge.recover_parent(&quote!(child));
                arms.push(quote! {
                    Self::#vi(_) => {
                        let child = <#child as ::bind::EventHandler<#marker>>::accumulate(
                            #child_path,
                            out,
                        )?;
                        path = #recover;
                    }
                });
            }
            let scrutinee = if root {
                quote!(path)
            } else {
                quote!(path.get_mut())
            };
            Ok((quote!(match #scrutinee { #(#arms)* }), children, true))
        }
        Data::Union(_) => Err(syn::Error::new(
            input.span(),
            "bind does not support unions",
        )),
    }
}

/// Emits `impl Dispatch<M>`: try the active child first, then this node's binds.
fn dispatch_impl(
    input: &DeriveInput,
    name: &Ident,
    marker: &Path,
    binds: &[Binding],
) -> syn::Result<TokenStream2> {
    let root = is_root(&input.attrs);
    let (recurse, children, needs_mut) = dispatch_body(input, name, marker, root)?;
    let where_clause = if children.is_empty() {
        quote!()
    } else {
        quote!(where #(#children: ::bind::Dispatch<#marker>,)*)
    };
    // A closure trigger is called with `&mut path`, so it needs the binding to be `mut` even on a
    // node whose shape never reassigns it.
    let binding = if needs_mut || any_closure_trigger(binds) {
        quote!(mut path)
    } else {
        quote!(path)
    };
    // Each bind: extract this source's event (the type match), then `is_matching`
    // (the key match). The trigger is built once into a local; `TryFrom` and the
    // handler pin the source-event type by inference.
    let checks = binds.iter().map(|b| {
        let trigger = trigger_expr(&b.trigger, &quote!(path));
        let handler = &b.handler;
        quote! {
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = #trigger;
                if ::bind::EventTrigger::is_matching(&trigger, ev) {
                    return ::core::ops::ControlFlow::Break(#handler(
                        ev,
                        ::bind::Node { parent: path, data: () },
                    ));
                }
            }
        }
    });
    Ok(quote! {
        #[automatically_derived]
        #[expect(clippy::useless_conversion)]
        impl ::bind::Dispatch<#marker> for #name #where_clause {
            fn dispatch<'a>(
                #binding: <Self as ::bind::Place>::Path<'a>,
                event: &<#marker as ::bind::Bindings>::Event,
            ) -> ::core::ops::ControlFlow<
                <#marker as ::bind::Bindings>::Output,
                <Self as ::bind::Place>::Path<'a>,
            >
            where
                Self: 'a,
            {
                #recurse
                #(#checks)*
                ::core::ops::ControlFlow::Continue(path)
            }
        }
    })
}

/// The dispatch recursion into the active child, the child types to bound, and
/// whether the path binding needs `mut` (any node that descends reassigns it).
fn dispatch_body(
    input: &DeriveInput,
    name: &Ident,
    marker: &Path,
    root: bool,
) -> syn::Result<(TokenStream2, Vec<Type>, bool)> {
    // `#[derived_child(f)]`: the child is not a field, so `f` produces its data and the derive
    // builds the node. Nothing here names the child's type, and nothing can: the derive has
    // only `f`'s name.
    if let Some(f) = derived_child_fn(&input.attrs)? {
        return Ok((
            derived_child_descent(&f, marker, &quote!(path)),
            Vec::new(),
            false,
        ));
    }
    match &input.data {
        Data::Struct(s) => match find_resolve_into(&s.fields)? {
            // A leaf descends into nothing; its path is never reassigned.
            None => Ok((quote!(), Vec::new(), false)),
            Some((field, child_ty, route)) => {
                let (child, boxed) = unbox(&child_ty);
                let edge = Edge {
                    parent: name,
                    is_root: root,
                    route: route.as_ref(),
                    boxed,
                    via: Via::Field(&field),
                };
                let child_path = edge.child_path(&quote!(path));
                let recover = edge.recover_parent(&quote!(child));
                let recurse = quote! {
                    let child = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event)?;
                    path = #recover;
                };
                Ok((recurse, vec![child.clone()], true))
            }
        },
        Data::Enum(e) => {
            let mut arms = Vec::new();
            let mut children = Vec::new();
            for v in &e.variants {
                let vi = &v.ident;
                let ty = single_field_ty(&v.fields)?;
                let route = parent_route(&v.attrs)?;
                let (child, boxed) = unbox(&ty);
                children.push(child.clone());
                let edge = Edge {
                    parent: name,
                    is_root: root,
                    route: route.as_ref(),
                    boxed,
                    via: Via::Variant(vi),
                };
                let child_path = edge.child_path(&quote!(path));
                let recover = edge.recover_parent(&quote!(child));
                arms.push(quote! {
                    Self::#vi(_) => {
                        let child = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event)?;
                        path = #recover;
                    }
                });
            }
            // The root enum matches `&mut Self` directly; a non-root enum reaches
            // its variant through the path's `get_mut`.
            let scrutinee = if root {
                quote!(path)
            } else {
                quote!(path.get_mut())
            };
            Ok((quote!(match #scrutinee { #(#arms)* }), children, true))
        }
        Data::Union(_) => Err(syn::Error::new(
            input.span(),
            "bind does not support unions",
        )),
    }
}

/// The marker named by the one required `#[binds(Marker)]`.
fn marker_of(input: &DeriveInput) -> syn::Result<Path> {
    let mut found = None;
    for attr in &input.attrs {
        if attr.path().is_ident("binds") {
            if found.is_some() {
                return Err(syn::Error::new(attr.span(), "expected one `#[binds(..)]`"));
            }
            found = Some(attr.parse_args::<Path>()?);
        }
    }
    found.ok_or_else(|| syn::Error::new(input.span(), "missing `#[binds(Marker)]`"))
}

/// The expression that produces a binding's trigger, given what dispatch is holding for this node.
///
/// A closure is CALLED with it, so a trigger can depend on the state it is bound on; anything else
/// is evaluated as the value it is. The distinction is syntactic because a trait cannot make it:
/// blanket impls for values and for closures overlap, and rustc cannot prove no type is both an
/// `EventTrigger` and an `Fn`.
///
/// A closure goes through [`bind::call_with`](::bind::call_with) rather than being called here: a
/// closure parameter takes its type from an expected type, not from an immediate call, and that
/// function's signature is what supplies one. Calling it directly would make every state-reading
/// binding annotate its own parameter with a path type it should not have to name.
fn trigger_expr(trigger: &Expr, state: &TokenStream2) -> TokenStream2 {
    if matches!(trigger, Expr::Closure(_)) {
        quote!(::bind::call_with(&mut #state, #trigger))
    } else {
        quote!(#trigger)
    }
}

/// Whether any of these bindings reads the node it is bound on, which is what makes the path
/// binding need `mut`: the closure is called with a unique reference to it.
fn any_closure_trigger(binds: &[Binding]) -> bool {
    binds.iter().any(|b| matches!(b.trigger, Expr::Closure(_)))
}

/// One `trigger => handler` pair. `accumulate` uses the trigger; `dispatch` uses
/// both.
struct Binding {
    trigger: Expr,
    handler: Expr,
}

impl syn::parse::Parse for Binding {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let trigger = input.parse()?;
        input.parse::<Token![=>]>()?;
        let handler = input.parse()?;
        Ok(Self { trigger, handler })
    }
}

/// Every `trigger => handler` pair across the node's `#[bind(..)]` attributes.
fn binds(attrs: &[syn::Attribute]) -> syn::Result<Vec<Binding>> {
    let mut out = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("bind") {
            let parsed =
                attr.parse_args_with(Punctuated::<Binding, Token![,]>::parse_terminated)?;
            out.extend(parsed);
        }
    }
    Ok(out)
}
