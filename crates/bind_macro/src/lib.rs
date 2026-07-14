//! Derive macro for `bind`: implements `EventHandler<M>` (accumulate) and
//! `Dispatch<M>` (dispatch).
//!
//! `#[derive(Bind)]` reads `#[binds(Marker)]` for the marker and the
//! `#[bind(trigger => handler, ..)]` pairs. `accumulate` inserts the node's
//! triggers and recurses into its `#[resolve_into]` fields and active enum
//! variant. `dispatch` tries the active child first (leafward, so a child's
//! binding beats an ancestor's), then the node's own binds, building each node's
//! laserbeam `Path` through the shared `derive_support::Edge`.

use derive_support::{Edge, Via, find_resolve_into, is_root, parent_route, single_field_ty, unbox};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Expr, Ident, Path, Token, Type, parse_macro_input};

#[proc_macro_derive(Bind, attributes(binds, bind, resolve_into))]
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

    let accumulate = accumulate_impl(input, name, &marker, &binds)?;
    let dispatch = dispatch_impl(input, name, &marker, &binds)?;
    let descend = descend_impl(input, name, &marker)?;
    Ok(quote! {
        #accumulate
        #dispatch
        #descend
    })
}

/// Emits `impl Descend<M>` for a PLACE: delegate to its own `Dispatch`, then hand the parent
/// back.
///
/// Per node, and not a blanket `impl<N, P> Descend<M> for Path<N, P>`: `Dispatch` carries
/// `Self: 'a`, and the HRTB needed to state the blanket is E0311. Here the lifetime is named,
/// so `Self: 'a` holds.
///
/// The root has no parent to hand back, so it gets none.
fn descend_impl(input: &DeriveInput, name: &Ident, marker: &Path) -> syn::Result<TokenStream2> {
    if is_root(&input.attrs) {
        return Ok(quote!());
    }
    Ok(quote! {
        #[automatically_derived]
        impl<'a> ::bind::Descend<#marker> for <#name as ::laserbeam::Resolve>::Path<'a>
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
    })
}

/// Emits `impl EventHandler<M>`: insert this node's triggers, then recurse.
fn accumulate_impl(
    input: &DeriveInput,
    name: &Ident,
    marker: &Path,
    binds: &[Binding],
) -> syn::Result<TokenStream2> {
    let (recurse, children) = accumulate_body(input, marker)?;
    let where_clause = if children.is_empty() {
        quote!()
    } else {
        quote!(where #(#children: ::bind::EventHandler<#marker>,)*)
    };
    let triggers = binds.iter().map(|b| &b.trigger);
    Ok(quote! {
        #[automatically_derived]
        #[allow(clippy::useless_conversion, clippy::implicit_hasher)]
        impl ::bind::EventHandler<#marker> for #name #where_clause {
            fn accumulate(
                &self,
                out: &mut ::std::collections::HashSet<<#marker as ::bind::Bindings>::Trigger>,
            ) -> ::core::result::Result<(), ::bind::BindError> {
                #(
                    ::bind::insert_or_error(out, ::core::convert::Into::into(#triggers))?;
                )*
                #recurse
            }
        }
    })
}

/// The accumulate recursion tail and the child types to bound, by node kind.
#[allow(clippy::option_if_let_else)]
fn accumulate_body(input: &DeriveInput, marker: &Path) -> syn::Result<(TokenStream2, Vec<Type>)> {
    match &input.data {
        Data::Struct(s) => match find_resolve_into(&s.fields)? {
            None => Ok((quote!(::core::result::Result::Ok(())), Vec::new())),
            Some((field, child_ty, _route)) => {
                let (child, boxed) = unbox(&child_ty);
                let child = child.clone();
                let recv = if boxed {
                    quote!(&*self.#field)
                } else {
                    quote!(&self.#field)
                };
                Ok((
                    quote!(::bind::EventHandler::<#marker>::accumulate(#recv, out)),
                    vec![child],
                ))
            }
        },
        Data::Enum(e) => {
            let mut arms = Vec::new();
            let mut children = Vec::new();
            for v in &e.variants {
                let vi = &v.ident;
                let ty = single_field_ty(&v.fields)?;
                let (child, boxed) = unbox(&ty);
                children.push(child.clone());
                let recv = if boxed {
                    quote!(&**inner)
                } else {
                    quote!(inner)
                };
                arms.push(quote! {
                    Self::#vi(inner) => ::bind::EventHandler::<#marker>::accumulate(#recv, out),
                });
            }
            Ok((quote!(match self { #(#arms)* }), children))
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
    let binding = if needs_mut {
        quote!(mut path)
    } else {
        quote!(path)
    };
    // Each bind: extract this source's event (the type match), then `is_matching`
    // (the key match). The trigger is built once into a local; `TryFrom` and the
    // handler pin the source-event type by inference.
    let checks = binds.iter().map(|b| {
        let trigger = &b.trigger;
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
        #[allow(clippy::useless_conversion)]
        impl ::bind::Dispatch<#marker> for #name #where_clause {
            fn dispatch<'a>(
                #binding: <Self as ::laserbeam::Resolve>::Path<'a>,
                event: &<#marker as ::bind::Bindings>::Event,
            ) -> ::core::ops::ControlFlow<
                <#marker as ::bind::Bindings>::Output,
                <Self as ::laserbeam::Resolve>::Path<'a>,
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
#[allow(clippy::option_if_let_else)]
fn dispatch_body(
    input: &DeriveInput,
    name: &Ident,
    marker: &Path,
    root: bool,
) -> syn::Result<(TokenStream2, Vec<Type>, bool)> {
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
