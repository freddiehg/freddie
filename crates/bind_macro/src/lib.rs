//! Derive macro for `bind`: implements `EventHandler<M>::accumulate`.
//!
//! `#[derive(Bind)]` reads `#[binds(Marker)]` for the marker and the
//! `#[bind(trigger => handler, ..)]` pairs, and generates an `accumulate` that
//! inserts the node's triggers and recurses into its `#[resolve_into]` fields and
//! active enum variant. The `handler` is recorded for dispatch and unused here.

use derive_support::{find_resolve_into, single_field_ty, unbox};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Expr, Path, Token, Type, parse_macro_input};

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
    let triggers = trigger_exprs(&input.attrs)?;
    let (recurse, children) = body(input, &marker)?;

    let where_clause = if children.is_empty() {
        quote!()
    } else {
        quote!(where #(#children: ::bind::EventHandler<#marker>,)*)
    };

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

/// The recursion tail and the child types to bound, by node kind.
#[allow(clippy::option_if_let_else)]
fn body(input: &DeriveInput, marker: &Path) -> syn::Result<(TokenStream2, Vec<Type>)> {
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

/// One `trigger => handler` pair. The handler is parsed and dropped; accumulation
/// uses only the trigger.
struct Binding {
    trigger: Expr,
}

impl syn::parse::Parse for Binding {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let trigger = input.parse()?;
        input.parse::<Token![=>]>()?;
        input.parse::<Expr>()?; // handler, parsed and dropped
        Ok(Self { trigger })
    }
}

/// The trigger of every `trigger => handler` pair across the node's `#[bind(..)]`.
fn trigger_exprs(attrs: &[syn::Attribute]) -> syn::Result<Vec<Expr>> {
    let mut triggers = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("bind") {
            let bindings =
                attr.parse_args_with(Punctuated::<Binding, Token![,]>::parse_terminated)?;
            triggers.extend(bindings.into_iter().map(|b| b.trigger));
        }
    }
    Ok(triggers)
}
