//! Shared syn helpers for the laserbeam and bind derives. They locate a node's
//! descent edges (the `#[resolve_into]` field of a struct, the single-field
//! payload of an enum variant) and unwrap `Box`.

use syn::spanned::Spanned;
use syn::{Fields, Ident, Path, Type};

/// The `#[resolve_into]` field of a struct: its name, child type, and, when the
/// child has multiple parents, the route-enum variant to wrap this node into.
pub type ResolveInto = (Ident, Type, Option<Path>);

/// Finds the single `#[resolve_into]` field of a struct, if any.
///
/// # Errors
///
/// Errors if more than one field carries `#[resolve_into]`.
pub fn find_resolve_into(fields: &Fields) -> syn::Result<Option<ResolveInto>> {
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
            let Some(ident) = f.ident.clone() else {
                continue;
            };
            found = Some((ident, f.ty.clone(), parent_route(&f.attrs)?));
        }
    }
    Ok(found)
}

/// The route enum named by `#[resolve_into(parent = Enum)]`, if present. A bare
/// `#[resolve_into]` (or no attribute) is a single-parent child.
///
/// # Errors
///
/// Errors if the attribute list contains anything other than `parent = ..`.
pub fn parent_route(attrs: &[syn::Attribute]) -> syn::Result<Option<Path>> {
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

/// The single field type of a tuple variant `Foo(Bar)`.
///
/// # Errors
///
/// Errors on any other variant shape (unit, struct, or multi-field).
pub fn single_field_ty(fields: &Fields) -> syn::Result<Type> {
    match fields {
        Fields::Unnamed(u) if u.unnamed.len() == 1 => Ok(u.unnamed[0].ty.clone()),
        _ => Err(syn::Error::new(
            fields.span(),
            "expected a single-field tuple variant `Foo(Bar)`",
        )),
    }
}

/// If `ty` is `Box<T>`, returns `(T, true)`; otherwise `(ty, false)`. A recursive
/// field or variant breaks its own size with `Box`, and a projection through it
/// has to dereference.
#[must_use]
pub fn unbox(ty: &Type) -> (&Type, bool) {
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
