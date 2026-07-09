//! Shared syn helpers for the laserbeam and bind derives. They locate a node's
//! descent edges (the `#[resolve_into]` field of a struct, the single-field
//! payload of an enum variant), unwrap `Box`, and build the child-`Path`
//! construction that `resolve` and `dispatch` descend through identically.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
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

/// True when a node is the tree root: it carries `#[laserbeam_root]` rather than
/// `#[laserbeam(..)]`, and its path is `&mut Self` instead of a [`Path`].
///
/// [`Path`]: laserbeam::Path
#[must_use]
pub fn is_root(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("laserbeam_root"))
}

/// How a child hangs off its parent node, for building the descent projection.
pub enum Via<'a> {
    /// A struct `#[resolve_into]` field.
    Field(&'a Ident),
    /// A single-field enum variant `Parent::Variant(Child)`.
    Variant(&'a Ident),
}

/// One descent edge from a parent node to a child.
///
/// Building the child `Path` is shared by `resolve` and `dispatch` so both
/// descend identically; `dispatch` additionally recovers the parent path on the
/// way back up.
pub struct Edge<'a> {
    /// The parent node's ident, naming `Self::` variants and the route variant.
    pub parent: &'a Ident,
    /// True when the parent is the root (its path is `&mut Self`).
    pub is_root: bool,
    /// The route enum for a multi-parent child; `None` for a single parent.
    pub route: Option<&'a Path>,
    /// True when the field or variant payload is `Box<Child>`.
    pub boxed: bool,
    /// How the child is reached on the parent.
    pub via: Via<'a>,
}

impl Edge<'_> {
    /// The child `Path` expression, given the parent-path expression `path`.
    /// `resolve` tail-calls into it and `dispatch` recurses into it, so both
    /// build the identical path.
    // The single/multi-parent `match` reads better than the `map_or_else` clippy
    // wants, given the multi-line `quote!` arms.
    #[allow(clippy::option_if_let_else)]
    #[must_use]
    pub fn child_path(&self, path: &TokenStream2) -> TokenStream2 {
        let deref = if self.boxed { quote!(*) } else { quote!() };
        match self.route {
            // Single-parent child: its path is `Path<Child, ThisPath>`, exactly
            // what `from_fn` builds, so `.into()` is identity.
            None => {
                let project = self.single_parent_projection(&deref);
                quote!(::laserbeam::Path::from_fn(#path, #project).into())
            }
            // Multi-parent child: wrap this node's path in the route variant named
            // after this node, and re-derive the child through it.
            Some(route) => {
                let parent = self.parent;
                let variant = quote!(#route::#parent);
                let project = self.multi_parent_projection(&variant, &deref);
                quote!(::laserbeam::Path::from_fn(#variant(#path.into()), #project))
            }
        }
    }

    /// The expression recovering this node's path from a child path `child` on the
    /// way back up (`dispatch` only). A single-parent child's `into_parent` is the
    /// parent path directly; a multi-parent child's is the route enum, matched
    /// back to this node's variant.
    #[allow(clippy::option_if_let_else)]
    #[must_use]
    pub fn recover_parent(&self, child: &TokenStream2) -> TokenStream2 {
        match self.route {
            None => quote!(#child.into_parent()),
            Some(route) => {
                let parent = self.parent;
                let variant = quote!(#route::#parent);
                quote!({
                    let #variant(pp) = #child.into_parent() else { ::core::unreachable!() };
                    pp
                })
            }
        }
    }

    /// The projection closure for a single-parent child.
    fn single_parent_projection(&self, deref: &TokenStream2) -> TokenStream2 {
        match &self.via {
            Via::Field(field) => {
                if self.is_root {
                    quote!(|o| &mut #deref o.#field)
                } else {
                    quote!(|np| &mut #deref np.get_mut().#field)
                }
            }
            Via::Variant(vi) => {
                let access = if self.boxed {
                    quote!(&mut **c)
                } else {
                    quote!(c)
                };
                if self.is_root {
                    quote!(|o| {
                        let Self::#vi(c) = &mut **o else { ::core::unreachable!() };
                        #access
                    })
                } else {
                    quote!(|np| {
                        let Self::#vi(c) = np.get_mut() else { ::core::unreachable!() };
                        #access
                    })
                }
            }
        }
    }

    /// The projection closure for a multi-parent child, reached through the route
    /// variant `variant`. The route type is total over every parent, but only this
    /// one is ever live, hence the `unreachable!()`.
    fn multi_parent_projection(
        &self,
        variant: &TokenStream2,
        deref: &TokenStream2,
    ) -> TokenStream2 {
        match &self.via {
            Via::Field(field) => {
                let node = if self.is_root {
                    quote!(pp.#field)
                } else {
                    quote!(pp.get_mut().#field)
                };
                quote!(|p| {
                    let #variant(pp) = p else { ::core::unreachable!() };
                    &mut #deref #node
                })
            }
            Via::Variant(vi) => {
                let inner = if self.boxed {
                    quote!(&mut **inner)
                } else {
                    quote!(inner)
                };
                let node = if self.is_root {
                    quote!(&mut **pp)
                } else {
                    quote!(pp.get_mut())
                };
                quote!(|p| {
                    let #variant(pp) = p else { ::core::unreachable!() };
                    let Self::#vi(inner) = #node else { ::core::unreachable!() };
                    #inner
                })
            }
        }
    }
}
