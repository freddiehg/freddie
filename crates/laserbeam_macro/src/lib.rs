//! Derive macro for the `laserbeam` crate. See `laserbeam` for usage.
//!
//! `#[derive(Laserbeam)]` emits `impl Resolve` giving a node its path type, from one attribute:
//!
//! - `#[laserbeam_root]` on the root node; its path is `&'a mut Self`.
//! - `#[laserbeam(path = P)]` on a non-root node; its path is `P<'a>`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;
use syn::{DeriveInput, Path, parse_macro_input};

#[proc_macro_derive(Laserbeam, attributes(laserbeam, laserbeam_root, resolve_into))]
pub fn derive_laserbeam(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// A node is either the root (its path is `&mut Self`) or a node with a declared path alias.
enum Role {
    Root,
    Node(Path),
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "laserbeam nodes may not be generic",
        ));
    }
    let name = &input.ident;
    let path_ty = match role_of(input)? {
        Role::Root => quote!(&'a mut Self),
        Role::Node(p) => quote!(#p<'a>),
    };
    Ok(quote! {
        #[automatically_derived]
        impl ::laserbeam::Resolve for #name {
            type Path<'a>
                = #path_ty
            where
                Self: 'a;
        }
    })
}

fn role_of(input: &DeriveInput) -> syn::Result<Role> {
    let mut found: Option<(Role, proc_macro2::Span)> = None;
    for attr in &input.attrs {
        let span = attr.span();
        let role = if attr.path().is_ident("laserbeam_root") {
            Role::Root
        } else if attr.path().is_ident("laserbeam") {
            let mut path = None;
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("path") {
                    path = Some(m.value()?.parse()?);
                    Ok(())
                } else {
                    Err(m.error("expected `path`"))
                }
            })?;
            Role::Node(path.ok_or_else(|| syn::Error::new(span, "laserbeam needs `path = ..`"))?)
        } else {
            continue;
        };
        if found.is_some() {
            return Err(syn::Error::new(
                span,
                "use exactly one of `laserbeam_root` or `laserbeam`",
            ));
        }
        found = Some((role, span));
    }
    found.map(|(role, _)| role).ok_or_else(|| {
        syn::Error::new(
            input.span(),
            "missing a `laserbeam_root` or `laserbeam` attribute",
        )
    })
}
