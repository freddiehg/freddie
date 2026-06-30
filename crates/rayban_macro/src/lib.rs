//! Derive macro for the `rayban` crate. See `rayban` for usage.
//!
//! `#[derive(Rayban)]` reads one of three attributes and emits impls only:
//!
//! - `#[rayban_root(resolved = R)]` on the root node.
//! - `#[rayban(path = P, resolved = R)]` on a non-root node.
//! - `#[rayban_path(node = N)]` on a multi-parent node's path enum.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DataEnum, DataStruct, DeriveInput, Fields, Ident, Path, Type, parse_macro_input};

#[proc_macro_derive(Rayban, attributes(rayban, rayban_root, rayban_path, resolve_into))]
pub fn derive_rayban(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

enum Role {
    Root { resolved: Path },
    Node { path: Path, resolved: Path },
    PathEnum { node: Path },
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    match role_of(input)? {
        Role::Root { resolved } => node_impl(input, None, &resolved),
        Role::Node { path, resolved } => node_impl(input, Some(&path), &resolved),
        Role::PathEnum { node } => path_enum_impl(input, &node),
    }
}

fn role_of(input: &DeriveInput) -> syn::Result<Role> {
    let mut found: Option<(Role, proc_macro2::Span)> = None;
    for attr in &input.attrs {
        let span = attr.span();
        let role = if attr.path().is_ident("rayban_root") {
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
                resolved: required(resolved, span, "rayban_root needs `resolved = ..`")?,
            }
        } else if attr.path().is_ident("rayban") {
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
                path: required(path, span, "rayban needs `path = ..`")?,
                resolved: required(resolved, span, "rayban needs `resolved = ..`")?,
            }
        } else if attr.path().is_ident("rayban_path") {
            let mut node = None;
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("node") {
                    node = Some(m.value()?.parse()?);
                    Ok(())
                } else {
                    Err(m.error("expected `node`"))
                }
            })?;
            Role::PathEnum {
                node: required(node, span, "rayban_path needs `node = ..`")?,
            }
        } else {
            continue;
        };
        if found.is_some() {
            return Err(syn::Error::new(
                span,
                "use exactly one of `rayban_root`, `rayban`, or `rayban_path`",
            ));
        }
        found = Some((role, span));
    }
    found.map(|(role, _)| role).ok_or_else(|| {
        syn::Error::new(
            input.span(),
            "missing a `rayban_root`, `rayban`, or `rayban_path` attribute",
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
            "rayban nodes may not be generic",
        ));
    }
    let name = &input.ident;
    let is_root = path.is_none();
    let path_ty = path.map_or_else(|| quote!(&'a mut Self), |p| quote!(#p<'a>));
    let body = match &input.data {
        Data::Struct(s) => struct_body(is_root, name, s, resolved)?,
        Data::Enum(e) => enum_body(is_root, e)?,
        Data::Union(_) => {
            return Err(syn::Error::new(
                input.span(),
                "rayban does not support unions",
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
        impl ::rayban::Resolve for #name {
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
        Some((field, child)) => {
            let project = if is_root {
                quote!(|o| &mut o.#field)
            } else {
                quote!(|np| &mut np.get_mut().#field)
            };
            Ok(quote! {
                <#child as ::rayban::Resolve>::resolve(
                    ::rayban::Path::from_fn(path, #project).into()
                )
            })
        }
    }
}

/// The body of `resolve` for an enum node: descend into the active variant.
fn enum_body(is_root: bool, e: &DataEnum) -> syn::Result<TokenStream2> {
    let mut variants = Vec::new();
    for v in &e.variants {
        variants.push((&v.ident, single_field_ty(&v.fields)?));
    }
    if variants.is_empty() {
        return Err(syn::Error::new(
            e.variants.span(),
            "a rayban enum needs at least one variant",
        ));
    }
    if is_root {
        let arms = variants.iter().map(|(vi, child)| {
            quote! {
                Self::#vi(_) => <#child as ::rayban::Resolve>::resolve(
                    ::rayban::Path::from_fn(path, |o| {
                        let Self::#vi(c) = &mut **o else { ::core::unreachable!() };
                        c
                    }).into()
                ),
            }
        });
        Ok(quote!(match path { #(#arms)* }))
    } else {
        let arms = variants.iter().map(|(vi, child)| {
            quote! {
                if ::core::matches!(path.get_mut(), Self::#vi(_)) {
                    return <#child as ::rayban::Resolve>::resolve(
                        ::rayban::Path::from_fn(path, |np| {
                            let Self::#vi(c) = np.get_mut() else { ::core::unreachable!() };
                            c
                        }).into()
                    );
                }
            }
        });
        Ok(quote!({ #(#arms)* ::core::unreachable!() }))
    }
}

/// Emits the `From` wraps and the `get_mut` dispatch for a multi-parent path enum.
fn path_enum_impl(input: &DeriveInput, node: &Path) -> syn::Result<TokenStream2> {
    let Data::Enum(e) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "`rayban_path` must be on an enum",
        ));
    };
    let name = &input.ident;
    let (impl_g, ty_g, where_c) = input.generics.split_for_impl();
    let mut froms = Vec::new();
    let mut arms = Vec::new();
    for v in &e.variants {
        let vi = &v.ident;
        let inner = single_field_ty(&v.fields)?;
        froms.push(quote! {
            #[automatically_derived]
            impl #impl_g ::core::convert::From<#inner> for #name #ty_g #where_c {
                fn from(p: #inner) -> Self {
                    Self::#vi(p)
                }
            }
        });
        arms.push(quote!(Self::#vi(p) => p.get_mut(),));
    }
    Ok(quote! {
        #(#froms)*
        #[automatically_derived]
        impl #impl_g #name #ty_g #where_c {
            #[must_use]
            pub fn get_mut(&mut self) -> &mut #node {
                match self { #(#arms)* }
            }
        }
    })
}

fn find_resolve_into(fields: &Fields) -> syn::Result<Option<(Ident, Type)>> {
    let mut found: Option<(Ident, Type)> = None;
    if let Fields::Named(named) = fields {
        for f in &named.named {
            if f.attrs.iter().any(|a| a.path().is_ident("resolve_into")) {
                if found.is_some() {
                    return Err(syn::Error::new(
                        f.span(),
                        "at most one `#[resolve_into]` field per struct",
                    ));
                }
                let ident = f.ident.clone().expect("named field has an ident");
                found = Some((ident, f.ty.clone()));
            }
        }
    }
    Ok(found)
}

fn single_field_ty(fields: &Fields) -> syn::Result<Type> {
    match fields {
        Fields::Unnamed(u) if u.unnamed.len() == 1 => Ok(u.unnamed[0].ty.clone()),
        _ => Err(syn::Error::new(
            fields.span(),
            "expected a single-field tuple variant `Foo(Bar)`",
        )),
    }
}
