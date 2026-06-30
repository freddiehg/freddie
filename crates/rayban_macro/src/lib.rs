//! Derive macro for the `rayban` crate. See `rayban` for usage.
//!
//! `#[derive(Rayban)]` reads one of two attributes and emits impls only:
//!
//! - `#[rayban_root(resolved = R)]` on the root node.
//! - `#[rayban(path = P, resolved = R)]` on a non-root node.
//!
//! A node with multiple parents declares its parent as a plain enum, one variant
//! per parent path, and each parent points at the variant to wrap into via
//! `#[resolve_into(parent = Enum::Variant)]`. That enum needs no derive: the
//! node's path is `Path<Node, ParentEnum>`, so `get_mut`/`into_parent` come from
//! `Path` itself.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DataEnum, DataStruct, DeriveInput, Fields, Ident, Path, Type, parse_macro_input};

#[proc_macro_derive(Rayban, attributes(rayban, rayban_root, resolve_into))]
pub fn derive_rayban(input: TokenStream) -> TokenStream {
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
        } else {
            continue;
        };
        if found.is_some() {
            return Err(syn::Error::new(
                span,
                "use exactly one of `rayban_root` or `rayban`",
            ));
        }
        found = Some((role, span));
    }
    found.map(|(role, _)| role).ok_or_else(|| {
        syn::Error::new(
            input.span(),
            "missing a `rayban_root` or `rayban` attribute",
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
        Data::Enum(e) => enum_body(is_root, name, e)?,
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
            let deref = if boxed { quote!(*) } else { quote!() };
            match route {
                // Single-parent child: its path is `Path<Child, ThisPath>`,
                // exactly what `from_fn` builds, so `.into()` is identity.
                None => {
                    let project = if is_root {
                        quote!(|o| &mut #deref o.#field)
                    } else {
                        quote!(|np| &mut #deref np.get_mut().#field)
                    };
                    Ok(quote! {
                        <#child as ::rayban::Resolve>::resolve(
                            ::rayban::Path::from_fn(path, #project).into()
                        )
                    })
                }
                // Multi-parent child: wrap this node's path in the route enum and
                // re-derive the child through that variant. The variant is named
                // after this node (the parent), the way isograph uses
                // `<Child>::Parent::#name`. The projection is total over the enum;
                // only this variant is ever live, hence the `unreachable!()`.
                Some(parent_enum) => {
                    let variant = quote!(#parent_enum::#name);
                    let project = if is_root {
                        quote!(|p| {
                            let #variant(pp) = p else { ::core::unreachable!() };
                            &mut #deref pp.#field
                        })
                    } else {
                        quote!(|p| {
                            let #variant(pp) = p else { ::core::unreachable!() };
                            &mut #deref pp.get_mut().#field
                        })
                    };
                    // `path.into()` is identity for an unboxed variant and
                    // `From<T> for Box<T>` for a boxed one, so a recursive parent
                    // chain breaks its own size with `Album(Box<AlbumPath>)`, the
                    // way isograph does.
                    Ok(quote! {
                        <#child as ::rayban::Resolve>::resolve(
                            ::rayban::Path::from_fn(#variant(path.into()), #project)
                        )
                    })
                }
            }
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
            &v.ident,
            single_field_ty(&v.fields)?,
            parent_route(&v.attrs)?,
        ));
    }
    if variants.is_empty() {
        return Err(syn::Error::new(
            e.variants.span(),
            "a rayban enum needs at least one variant",
        ));
    }
    // A `Box<Child>` variant (recursive data) is dereferenced; the resolved
    // child is the inner `T`.
    if is_root {
        let arms = variants.iter().map(|(vi, ty, route)| {
            let (child, boxed) = unbox(ty);
            match route {
                // Single-parent child: descend straight into the variant payload.
                None => {
                    let access = if boxed { quote!(&mut **c) } else { quote!(c) };
                    quote! {
                        Self::#vi(_) => <#child as ::rayban::Resolve>::resolve(
                            ::rayban::Path::from_fn(path, |o| {
                                let Self::#vi(c) = &mut **o else { ::core::unreachable!() };
                                #access
                            }).into()
                        ),
                    }
                }
                // Multi-parent child: wrap the root path (`&mut Self`) in the
                // route variant named after this enum. The root has no `get_mut`,
                // so the projection dereferences instead.
                Some(route) => {
                    let inner = if boxed { quote!(&mut **inner) } else { quote!(inner) };
                    quote! {
                        Self::#vi(_) => <#child as ::rayban::Resolve>::resolve(
                            ::rayban::Path::from_fn(#route::#name(path.into()), |p| {
                                let #route::#name(pp) = p else { ::core::unreachable!() };
                                let Self::#vi(inner) = &mut **pp else { ::core::unreachable!() };
                                #inner
                            })
                        ),
                    }
                }
            }
        });
        Ok(quote!(match path { #(#arms)* }))
    } else {
        let arms = variants.iter().map(|(vi, ty, route)| {
            let (child, boxed) = unbox(ty);
            match route {
                // Single-parent child: descend straight into the variant payload.
                None => {
                    let access = if boxed { quote!(&mut **c) } else { quote!(c) };
                    quote! {
                        if ::core::matches!(path.get_mut(), Self::#vi(_)) {
                            return <#child as ::rayban::Resolve>::resolve(
                                ::rayban::Path::from_fn(path, |np| {
                                    let Self::#vi(c) = np.get_mut() else { ::core::unreachable!() };
                                    #access
                                }).into()
                            );
                        }
                    }
                }
                // Multi-parent child reached via this variant: wrap this enum's
                // path in the route variant named after this enum, then re-derive
                // the child by matching its way back out.
                Some(route) => {
                    let inner = if boxed { quote!(&mut **inner) } else { quote!(inner) };
                    quote! {
                        if ::core::matches!(path.get_mut(), Self::#vi(_)) {
                            return <#child as ::rayban::Resolve>::resolve(
                                ::rayban::Path::from_fn(#route::#name(path.into()), |p| {
                                    let #route::#name(pp) = p else { ::core::unreachable!() };
                                    let Self::#vi(inner) = pp.get_mut() else { ::core::unreachable!() };
                                    #inner
                                })
                            );
                        }
                    }
                }
            }
        });
        Ok(quote!({ #(#arms)* ::core::unreachable!() }))
    }
}

/// The `#[resolve_into]` field of a struct: its name, child type, and, when the
/// child has multiple parents, the route-enum variant to wrap this node into.
type ResolveInto = (Ident, Type, Option<Path>);

fn find_resolve_into(fields: &Fields) -> syn::Result<Option<ResolveInto>> {
    let mut found: Option<ResolveInto> = None;
    if let Fields::Named(named) = fields {
        for f in &named.named {
            if !f.attrs.iter().any(|a| a.path().is_ident("resolve_into")) {
                continue;
            }
            if found.is_some() {
                return Err(syn::Error::new(
                    f.span(),
                    "at most one `#[resolve_into]` field per struct",
                ));
            }
            let ident = f.ident.clone().expect("named field has an ident");
            found = Some((ident, f.ty.clone(), parent_route(&f.attrs)?));
        }
    }
    Ok(found)
}

/// The route enum named by `#[resolve_into(parent = Enum)]`, if present. A bare
/// `#[resolve_into]` (or no attribute) is a single-parent child; the `parent`
/// form marks a child reached through a multi-parent route enum.
fn parent_route(attrs: &[syn::Attribute]) -> syn::Result<Option<Path>> {
    let Some(attr) = attrs.iter().find(|a| a.path().is_ident("resolve_into")) else {
        return Ok(None);
    };
    let mut parent = None;
    if matches!(attr.meta, syn::Meta::List(_)) {
        attr.parse_nested_meta(|m| {
            if m.path.is_ident("parent") {
                parent = Some(m.value()?.parse()?);
                Ok(())
            } else {
                Err(m.error("expected `parent`"))
            }
        })?;
    }
    Ok(parent)
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

/// If `ty` is `Box<T>`, returns `(T, true)`; otherwise `(ty, false)`. Recursive
/// data structures break their own size with `Box`, and the projection through
/// such a field or variant has to dereference it.
fn unbox(ty: &Type) -> (&Type, bool) {
    if let Type::Path(tp) = ty
        && let Some(seg) = tp.path.segments.last()
        && seg.ident == "Box"
        && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return (inner, true);
    }
    (ty, false)
}
