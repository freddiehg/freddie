//! Derive macro for the `laserbeam` crate. See `laserbeam` for usage.
//!
//! `#[derive(Laserbeam)]` reads one of two attributes and emits impls only:
//!
//! - `#[laserbeam_root(resolved = R)]` on the root node.
//! - `#[laserbeam(path = P, resolved = R)]` on a non-root node.
//!
//! A node with multiple parents declares its parent as a plain enum, one variant
//! per parent path, and each parent points at the variant to wrap into via
//! `#[resolve_into(parent = Enum::Variant)]`. That enum needs no derive: the
//! node's path is `Path<Node, ParentEnum>`, so `get_mut`/`into_parent` come from
//! `Path` itself.

use derive_support::{Edge, Via, find_resolve_into, parent_route, single_field_ty, unbox};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DataEnum, DataStruct, DeriveInput, Ident, Path, parse_macro_input};

#[proc_macro_derive(Laserbeam, attributes(laserbeam, laserbeam_root, resolve_into))]
pub fn derive_laserbeam(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

enum Role {
    Root { resolved: Path },
    Node { path: Path, resolved: Path },
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    match role_of(input)? {
        Role::Root { resolved } => node_impl(input, None, &resolved),
        Role::Node { path, resolved } => node_impl(input, Some(&path), &resolved),
    }
}

fn role_of(input: &DeriveInput) -> syn::Result<Role> {
    let mut found: Option<(Role, proc_macro2::Span)> = None;
    for attr in &input.attrs {
        let span = attr.span();
        let role = if attr.path().is_ident("laserbeam_root") {
            let mut resolved = None;
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("resolved") {
                    resolved = Some(m.value()?.parse()?);
                    Ok(())
                } else {
                    Err(m.error("expected `resolved`"))
                }
            })?;
            Role::Root {
                resolved: required(resolved, span, "laserbeam_root needs `resolved = ..`")?,
            }
        } else if attr.path().is_ident("laserbeam") {
            let mut path = None;
            let mut resolved = None;
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("path") {
                    path = Some(m.value()?.parse()?);
                    Ok(())
                } else if m.path.is_ident("resolved") {
                    resolved = Some(m.value()?.parse()?);
                    Ok(())
                } else {
                    Err(m.error("expected `path` or `resolved`"))
                }
            })?;
            Role::Node {
                path: required(path, span, "laserbeam needs `path = ..`")?,
                resolved: required(resolved, span, "laserbeam needs `resolved = ..`")?,
            }
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

fn required<T>(value: Option<T>, span: proc_macro2::Span, msg: &str) -> syn::Result<T> {
    value.ok_or_else(|| syn::Error::new(span, msg))
}

/// Emits `impl Resolve` for a node (root or not).
fn node_impl(
    input: &DeriveInput,
    path: Option<&Path>,
    resolved: &Path,
) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "laserbeam nodes may not be generic",
        ));
    }
    let name = &input.ident;
    let is_root = path.is_none();
    let path_ty = path.map_or_else(|| quote!(&'a mut Self), |p| quote!(#p<'a>));
    let body = match &input.data {
        Data::Struct(s) => struct_body(is_root, name, s, resolved)?,
        Data::Enum(e) => enum_body(is_root, name, e)?,
        Data::Union(_) => {
            return Err(syn::Error::new(
                input.span(),
                "laserbeam does not support unions",
            ));
        }
    };
    // A non-root enum inspects its variant through `path.get_mut()`, so the
    // parameter must be mutable; nothing else mutates it.
    let needs_mut = !is_root && matches!(&input.data, Data::Enum(_));
    let binding = if needs_mut {
        quote!(mut path)
    } else {
        quote!(path)
    };
    Ok(quote! {
        #[automatically_derived]
        #[allow(clippy::useless_conversion)]
        impl ::laserbeam::Resolve for #name {
            type Path<'a> = #path_ty;
            type Resolved<'a> = #resolved<'a>;
            fn resolve<'a>(#binding: #path_ty) -> #resolved<'a>
            where
                Self: 'a,
            {
                #body
            }
        }
    })
}

/// The body of `resolve` for a struct node: a leaf returns its own resolved
/// variant; otherwise it descends into its one `#[resolve_into]` field.
// The single/multi-parent `match` reads better than the `map_or_else` clippy
// wants, given the two multi-line `quote!` arms.
#[allow(clippy::option_if_let_else)]
fn struct_body(
    is_root: bool,
    name: &Ident,
    s: &DataStruct,
    resolved: &Path,
) -> syn::Result<TokenStream2> {
    match find_resolve_into(&s.fields)? {
        // POTENTIAL: emit `path.into()` instead of hardcoding the
        // `#resolved::#name(..)` variant constructor, so `resolved` could be a
        // plain struct (with a `From<LeafPath>` impl), not only an enum.
        None => Ok(quote!(#resolved::#name(path))),
        Some((field, child_ty, route)) => {
            // A `Box<Child>` field (the way a recursive data structure breaks its
            // own size) is dereferenced in the projection; the resolved child is
            // the inner `T`.
            let (child, boxed) = unbox(&child_ty);
            let edge = Edge {
                parent: name,
                is_root,
                route: route.as_ref(),
                boxed,
                via: Via::Field(&field),
            };
            let child_path = edge.child_path(&quote!(path));
            Ok(quote! {
                <#child as ::laserbeam::Resolve>::resolve(#child_path)
            })
        }
    }
}

/// The body of `resolve` for an enum node: descend into the active variant. A
/// variant may carry `#[resolve_into(parent = Route)]` to descend into a
/// multi-parent child, just like a struct field.
// The single/multi-parent `match` reads better than the `map_or_else` clippy
// wants, given the two multi-line `quote!` arms.
#[allow(clippy::option_if_let_else)]
fn enum_body(is_root: bool, name: &Ident, e: &DataEnum) -> syn::Result<TokenStream2> {
    let mut variants = Vec::new();
    for v in &e.variants {
        variants.push((
            v.ident.clone(),
            single_field_ty(&v.fields)?,
            parent_route(&v.attrs)?,
        ));
    }
    if variants.is_empty() {
        return Err(syn::Error::new(
            e.variants.span(),
            "a laserbeam enum needs at least one variant",
        ));
    }
    let arms = variants
        .iter()
        .map(|(vi, ty, route)| {
            let (child, boxed) = unbox(ty);
            let edge = Edge {
                parent: name,
                is_root,
                route: route.as_ref(),
                boxed,
                via: Via::Variant(vi),
            };
            let child_path = edge.child_path(&quote!(path));
            if is_root {
                quote! {
                    Self::#vi(_) => <#child as ::laserbeam::Resolve>::resolve(#child_path),
                }
            } else {
                quote! {
                    if ::core::matches!(path.get_mut(), Self::#vi(_)) {
                        return <#child as ::laserbeam::Resolve>::resolve(#child_path);
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    if is_root {
        Ok(quote!(match path { #(#arms)* }))
    } else {
        Ok(quote!({ #(#arms)* ::core::unreachable!() }))
    }
}

// The `#[resolve_into]`, single-field-variant, and `Box` helpers live in
// `derive_support`, shared with the bind derive.
