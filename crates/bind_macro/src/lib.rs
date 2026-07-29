//! Derive macro for `bind`: implements `EventHandler<M>` (accumulate) and
//! `Dispatch<M>` (dispatch).
//!
//! `#[derive(Bind)]` reads `#[binds(Marker)]` for the marker and the node's scheduled items,
//! `#[bind]` / `#[post]` / `#[pre_post]`, in source order. `accumulate` inserts the node's
//! triggers and recurses into its `#[resolve_into]` fields and active enum variant.
//!
//! `dispatch` is one linear body at every node: snap each item's trigger and pre, descend into
//! the active child, fold what the child completed to into this node's state, run every
//! scheduled item over that state, and complete. The child path is built through the shared
//! `derive_support::Edge`, so descent matches `resolve`'s.

use derive_support::{
    Edge, Route, Via, find_resolve_into, is_root, node_parent, parent_route, single_field_ty, unbox,
};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Expr, Fields, Ident, Path, Token, Type, parse_macro_input};

#[proc_macro_derive(
    Bind,
    attributes(
        binds,
        bind,
        post,
        pre_post,
        resolve_into,
        derived_child,
        derived_node,
        node
    )
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
    let items = scheduled(&input.attrs)?;

    // A DERIVED level is not a place in the tree. It has no `Resolve`, so it can have neither
    // `Dispatch` nor `EventHandler`, both of which take `Self::Path`. It implements
    // `DispatchIntoTreePath` on its `Node` instead, ascending at the place beneath it.
    if let Some(parent) = derived_node_parent(&input.attrs)? {
        return derived_node_impl(input, name, &parent, &marker, &items);
    }

    let place = place_impl(input, name)?;
    let accumulate = accumulate_impl(input, name, &marker, &items)?;
    let dispatch = dispatch_impl(input, name, &marker, &items)?;
    Ok(quote! {
        #place
        #accumulate
        #dispatch
    })
}

/// Emits `impl laserbeam::HasPath` for a place node: its path type, `PathMut<Self, Parent>` from
/// `#[node(parent_path = P)]`, or `&mut Self` for `#[node(root)]`. This is the associated type that
/// `Dispatch`, `EventHandler`, and the
/// place `DispatchIntoParent` impl all name.
fn place_impl(input: &DeriveInput, name: &Ident) -> syn::Result<TokenStream2> {
    let path_ty = if is_root(&input.attrs) {
        quote!(&'a mut Self)
    } else {
        let parent = node_parent(&input.attrs)?.ok_or_else(|| {
            syn::Error::new(
                input.ident.span(),
                "a bind node needs `#[node(parent_path = ..)]` or `#[node(root)]`",
            )
        })?;
        quote!(::laserbeam::PathMut<Self, #parent<'a>>)
    };
    Ok(quote! {
        #[automatically_derived]
        impl ::laserbeam::HasPath for #name {
            type Path<'a>
                = #path_ty
            where
                Self: 'a;
        }
    })
}

/// The parent path named by `#[derived_node(parent_path = Alias)]`, if this level is not a place.
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
                if meta.path.is_ident("parent_path") {
                    parent = Some(meta.value()?.parse::<Path>()?);
                    Ok(())
                } else {
                    Err(meta.error("expected `parent_path = Alias`"))
                }
            })?;
            found = Some(parent.ok_or_else(|| {
                syn::Error::new(attr.span(), "`#[derived_node]` needs `parent_path = Alias`")
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
///
/// Every arm ascends at the same place, so every arm returns the same `Completed` and the
/// match is total with nothing to fold between them.
fn derived_enum_node_impl(
    input: &DeriveInput,
    name: &Ident,
    parent: &Path,
    marker: &Path,
    items: &[Scheduled],
    e: &syn::DataEnum,
) -> syn::Result<TokenStream2> {
    if !items.is_empty() {
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
        reject_resolve_into(&v.fields)?;
        dispatch_arms.push(quote! {
            #name::#vi(data) => ::bind::DispatchIntoTreePath::<#marker>::dispatch_into_tree_path(
                ::bind::Node { parent, data },
                event,
                effs,
                claim,
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
        impl<'a> ::bind::DispatchIntoTreePath<#marker> for ::bind::Node<#parent<'a>, #name> {
            fn dispatch_into_tree_path(
                self,
                event: &<#marker as ::bind::Bindings>::Event,
                effs: &mut <#marker as ::bind::Bindings>::Output,
                claim: &mut ::bind::Claim<'_>,
            ) -> ::laserbeam::Completed<
                <::bind::Node<#parent<'a>, #name> as ::bind::HasTreePath>::TreePath,
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

/// Emits `DispatchIntoTreePath` (and the check's half) for a DERIVED level: the same linear body a
/// place emits, over `node` instead of `path`, ascending at the place beneath it.
///
/// It never names its own node type, and it cannot name its place either: a level whose parent
/// is another derived level knows only that parent's `Node` alias. Both are spelled through the
/// `HasTreePath` projection, which resolves because `#parent` is concrete at the impl.
fn derived_node_impl(
    input: &DeriveInput,
    name: &Ident,
    parent: &Path,
    marker: &Path,
    items: &[Scheduled],
) -> syn::Result<TokenStream2> {
    // Several possible levels: the DATA is an enum. There is no separate mechanism. The derive
    // destructures per variant and rebuilds the node, so each variant's handler gets its own
    // `Data` and the parent is shared by construction.
    if let Data::Enum(e) = &input.data {
        return derived_enum_node_impl(input, name, parent, marker, items, e);
    }
    if let Data::Struct(s) = &input.data {
        reject_resolve_into(&s.fields)?;
    }

    let node = quote!(node);
    let state = derived_state(input, marker, &node)?;
    let acc_descend = derived_accumulate_descent(input, marker)?;
    let opts = items.iter().enumerate().map(|(i, it)| opt(i, it, &node));
    let blocks = items
        .iter()
        .enumerate()
        .map(|(i, it)| scheduled_block(i, it));
    let binding = state_binding(items);
    let triggers = claimed_triggers(items);
    Ok(quote! {
        #[automatically_derived]
        impl<'a> ::bind::DispatchIntoTreePath<#marker> for ::bind::Node<#parent<'a>, #name> {
            fn dispatch_into_tree_path(
                self,
                event: &<#marker as ::bind::Bindings>::Event,
                effs: &mut <#marker as ::bind::Bindings>::Output,
                claim: &mut ::bind::Claim<'_>,
            ) -> ::laserbeam::Completed<
                <::bind::Node<#parent<'a>, #name> as ::bind::HasTreePath>::TreePath,
            > {
                let node = self;
                #(#opts)*
                let #binding = #state;
                #(#blocks)*
                ::laserbeam::MaybeInvalidated::complete(state)
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

/// The `#[derived_child]` edge's state, for dispatch. Emitted on a PLACE and on a DERIVED level
/// alike, because both reach a child the same way once they hold it.
///
/// `f` is `fn(&Parent) -> Option<Data>`: a shared reference, so what it reads decides whether
/// the level exists at all, before anything is moved. With a level, the descent consumes the
/// parent and the place comes back inside the child's leave; without one, the parent is still
/// here and flattens to that same place, which is the identity for a place.
///
/// The derive names no type it cannot see: `data`'s type comes from `f`'s return, and inference
/// resolves `DispatchIntoTreePath` from the `Node`.
fn derived_child_state(f: &Path, marker: &Path, place: &TokenStream2) -> TokenStream2 {
    quote! {
        match #f(&#place) {
            ::core::option::Option::Some(data) => ::laserbeam::Completed::to_maybe_invalidated(
                ::bind::DispatchIntoTreePath::<#marker>::dispatch_into_tree_path(
                    ::bind::Node { parent: #place, data },
                    event,
                    effs,
                    claim,
                ),
            ),
            ::core::option::Option::None => ::laserbeam::MaybeInvalidated::NotInvalidated(
                ::bind::HasTreePath::into_tree_path(#place),
            ),
        }
    }
}

/// A derived level's own `#state`: the edge above when it has a `#[derived_child]`, and the
/// node flattened to its place when it has nothing below it.
fn derived_state(
    input: &DeriveInput,
    marker: &Path,
    node: &TokenStream2,
) -> syn::Result<TokenStream2> {
    Ok(derived_child_fn(&input.attrs)?.map_or_else(
        || quote!(::laserbeam::MaybeInvalidated::NotInvalidated(::bind::HasTreePath::into_tree_path(#node))),
        |f| derived_child_state(&f, marker, node),
    ))
}

/// A derived level cannot hang a place child: its `data` is rebuilt every dispatch and dies with
/// it, so a place below it would have to fold through a `Node`, which is not a path.
fn reject_resolve_into(fields: &Fields) -> syn::Result<()> {
    for f in fields {
        if let Some(attr) = f.attrs.iter().find(|a| a.path().is_ident("resolve_into")) {
            return Err(syn::Error::new(
                attr.span(),
                "a derived level cannot have a `#[resolve_into]` child: its `data` dies with the \
                 dispatch. Persist the state in the tree at a real place the derived level reads, \
                 or hang a fresh level with `#[derived_child]`.",
            ));
        }
    }
    Ok(())
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

fn derived_accumulate_descent(input: &DeriveInput, marker: &Path) -> syn::Result<TokenStream2> {
    Ok(derived_child_fn(&input.attrs)?.map_or_else(
        || quote!(),
        |f| derived_child_accumulate(&f, marker, &quote!(node)),
    ))
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
    items: &[Scheduled],
) -> syn::Result<TokenStream2> {
    let root = is_root(&input.attrs);
    let (recurse, children, needs_mut) = accumulate_body(input, name, marker, root)?;
    let where_clause = if children.is_empty() {
        quote!()
    } else {
        quote!(where #(#children: ::bind::EventHandler<#marker>,)*)
    };
    let binding = if needs_mut {
        quote!(mut path)
    } else {
        quote!(path)
    };
    let triggers = claimed_triggers(items);
    Ok(quote! {
        ::bind::check_only! {
        #[automatically_derived]
        #[expect(clippy::useless_conversion, clippy::implicit_hasher)]
        impl ::bind::EventHandler<#marker> for #name #where_clause {
            fn accumulate<'a>(
                #binding: <Self as ::laserbeam::HasPath>::Path<'a>,
                out: &mut ::std::collections::HashSet<<#marker as ::bind::Bindings>::Trigger>,
            ) -> ::core::result::Result<
                <Self as ::laserbeam::HasPath>::Path<'a>,
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
/// `mut`. Mirrors `dispatch_body`, minus early return: accumulate never stops early.
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

/// Emits `impl Dispatch<M>`: descend into the active child, then run this node's scheduled
/// items over what the descent left of its path.
fn dispatch_impl(
    input: &DeriveInput,
    name: &Ident,
    marker: &Path,
    items: &[Scheduled],
) -> syn::Result<TokenStream2> {
    let root = is_root(&input.attrs);
    let path = quote!(path);
    let (state, children) = dispatch_state(input, name, marker, root, &path)?;
    let where_clause = if children.is_empty() {
        quote!()
    } else {
        quote!(where #(#children: ::bind::Dispatch<#marker>,)*)
    };
    let opts = items.iter().enumerate().map(|(i, it)| opt(i, it, &path));
    let blocks = items
        .iter()
        .enumerate()
        .map(|(i, it)| scheduled_block(i, it));
    let binding = state_binding(items);
    Ok(quote! {
        #[automatically_derived]
        impl ::bind::Dispatch<#marker> for #name #where_clause {
            fn dispatch<'a, 'c>(
                path: <Self as ::laserbeam::HasPath>::Path<'a>,
                event: &<#marker as ::bind::Bindings>::Event,
                effs: &mut <#marker as ::bind::Bindings>::Output,
                claim: &mut ::bind::Claim<'c>,
            ) -> ::laserbeam::Completed<<Self as ::laserbeam::HasPath>::Path<'a>>
            where
                Self: 'a,
                <Self as ::laserbeam::HasPath>::Path<'a>: ::laserbeam::HasStop,
            {
                #(#opts)*
                let #binding = #state;
                #(#blocks)*
                ::laserbeam::MaybeInvalidated::complete(state)
            }
        }
    })
}

/// The state binding: `mut` only when a scheduled item will rebind it, so a node with nothing
/// scheduled does not carry a needless `mut`.
fn state_binding(items: &[Scheduled]) -> TokenStream2 {
    if items.is_empty() {
        quote!(state)
    } else {
        quote!(mut state)
    }
}

/// This node's state after the descent, and the child types to bound.
///
/// A leaf's path is untouched, so it starts standing. Everything else descends and folds what
/// the child completed to: the fold is the child's `Stop`, read at the parent.
fn dispatch_state(
    input: &DeriveInput,
    name: &Ident,
    marker: &Path,
    root: bool,
    place: &TokenStream2,
) -> syn::Result<(TokenStream2, Vec<Type>)> {
    // `#[derived_child(f)]`: the child is not a field, so `f` produces its data and the derive
    // builds the node. Nothing here names the child's type, and nothing can: the derive has
    // only `f`'s name.
    if let Some(f) = derived_child_fn(&input.attrs)? {
        return Ok((derived_child_state(&f, marker, place), Vec::new()));
    }
    match &input.data {
        Data::Struct(s) => match find_resolve_into(&s.fields)? {
            None => Ok((
                quote!(::laserbeam::MaybeInvalidated::NotInvalidated(#place)),
                Vec::new(),
            )),
            Some((field, child_ty, route)) => {
                let (child, boxed) = unbox(&child_ty);
                let edge = Edge {
                    parent: name,
                    is_root: root,
                    route: route.as_ref(),
                    boxed,
                    via: Via::Field(&field),
                };
                Ok((
                    child_state(&edge, child, marker, place),
                    vec![child.clone()],
                ))
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
                let state = child_state(&edge, child, marker, place);
                arms.push(quote!(Self::#vi(_) => { #state }));
            }
            // The root enum matches `&mut Self` directly; a non-root enum reaches its variant
            // through the path. A SHARED read: the arms bind nothing, the discriminant is all
            // this asks for, and the arm then consumes the path to build the child's.
            let scrutinee = if root {
                quote!(#place)
            } else {
                quote!(#place.get())
            };
            Ok((quote!(match #scrutinee { #(#arms)* }), children))
        }
        Data::Union(_) => Err(syn::Error::new(
            input.span(),
            "bind does not support unions",
        )),
    }
}

/// One place edge's state: dispatch the child, unwrap its leave, and read it at this node.
///
/// A single-parent edge reads it through laserbeam's `Stop` conversions, whose two impls are
/// the root and non-root cases. A route edge cannot: its `Up` payload is the consumer's enum,
/// so the fold matches out the variant this descent constructed. The `unreachable!()`s assert
/// what the multi-parent projections already assert, that only the live route is ever built.
fn child_state(edge: &Edge<'_>, child: &Type, marker: &Path, place: &TokenStream2) -> TokenStream2 {
    let child_path = edge.child_path(place);
    let leave = quote! {
        ::laserbeam::Completed::into_inner(
            <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim),
        )
    };
    let Some(Route { parent: route, up }) = edge.route else {
        return quote!(#leave.to_maybe_invalidated());
    };
    let parent = edge.parent;
    quote! {
        match #leave {
            ::laserbeam::Stop::Here(child) => {
                let #route::#parent(recovered) = ::bind::HasParent::into_parent(child) else {
                    ::core::unreachable!()
                };
                ::laserbeam::MaybeInvalidated::NotInvalidated(recovered)
            }
            ::laserbeam::Stop::Up(above) => {
                let #up::#parent(completed) = above else { ::core::unreachable!() };
                ::laserbeam::MaybeInvalidated::Invalidated(completed)
            }
        }
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
///
/// It is handed a SHARED reference to what dispatch is holding, so a trigger reads the node it is
/// bound on, and its parent, and cannot write either.
fn trigger_expr(trigger: &Expr, state: &TokenStream2) -> TokenStream2 {
    if matches!(trigger, Expr::Closure(_)) {
        quote!(::bind::call_with(&#state, #trigger))
    } else {
        quote!(#trigger)
    }
}

/// The triggers THE CHECK collects: the ones a node CLAIMS.
///
/// Only a `#[bind]` claims, so only a `#[bind]`'s trigger is a claim to collide over. A post is
/// scheduled by its trigger and runs beside whatever claimed, which is not a clobber.
///
/// A closure trigger is skipped. Its value is read from state at dispatch, so it is not a static
/// claim, and two nodes whose state holds nothing would produce the same value and read as a
/// clobber while neither could fire at all. It is also what lets a trigger be an `Option`:
/// `insert_or_error` takes a value, `None` has none to give, and the conversion is never reached.
fn claimed_triggers(items: &[Scheduled]) -> impl Iterator<Item = &Expr> {
    items
        .iter()
        .filter(|it| it.claims && !matches!(it.trigger, Expr::Closure(_)))
        .map(|it| &it.trigger)
}

/// The local a scheduled item's opt lands in, numbered in source order across the node's
/// attributes.
fn opt_ident(i: usize) -> Ident {
    format_ident!("opt_{i}")
}

/// One scheduled item's opt, snapped BEFORE the descent: the trigger is read off the state as it
/// stands on the way down, and so is whatever the pre takes from it, while the child the descent
/// is about to run in is still there.
///
/// The pre is called rather than inlined even when it is the synthesized `|_, _| ()`, so one
/// emitted shape serves every kind of scheduled item and the `Snap` a handler receives is
/// whatever the pre returned.
fn opt(i: usize, item: &Scheduled, state: &TokenStream2) -> TokenStream2 {
    let ident = opt_ident(i);
    let trigger = trigger_expr(&item.trigger, state);
    let pre = &item.pre;
    quote! {
        let #ident = match ::core::convert::TryFrom::try_from(event) {
            ::core::result::Result::Ok(ev) => {
                let trigger = #trigger;
                if ::bind::EventTrigger::is_matching(&trigger, ev) {
                    ::core::option::Option::Some((ev, (#pre)(ev, &#state)))
                } else {
                    ::core::option::Option::None
                }
            }
            ::core::result::Result::Err(_) => ::core::option::Option::None,
        };
    }
}

/// One scheduled item's block, the same for every kind: call it with what its opt snapped and
/// the state as it stands, take its effects, and re-derive the state from the leave it returned.
///
/// Nothing here branches on the claim or on the state. The item was scheduled by its trigger and
/// runs; what each state branch means is its own business.
fn scheduled_block(i: usize, item: &Scheduled) -> TokenStream2 {
    let ident = opt_ident(i);
    let rhs = item.rhs();
    quote! {
        if let ::core::option::Option::Some((ev, snap)) = #ident {
            let (e, completed) = (#rhs)(
                ev,
                snap,
                ::bind::AscendState::new(state, ::bind::Claim::reborrow(claim)),
            );
            ::core::iter::Extend::extend(effs, e);
            state = ::laserbeam::Completed::to_maybe_invalidated(completed);
        }
    }
}

/// The pre a `#[bind]` or a `#[post]` gets: it takes nothing off the state, so the `Snap` its
/// handler is handed is `()`.
fn unit_pre() -> Expr {
    syn::parse_quote!(|_, _| ())
}

/// One item on a node's schedule: what fires it, what it snaps before the descent, what runs on
/// the way up, and whether it claims.
///
/// The three attribute kinds differ only here, at parse time. Past this point one list drives
/// one emitted shape.
struct Scheduled {
    trigger: Expr,
    /// What runs before the descent and produces the handler's `Snap`.
    pre: Expr,
    handler: Expr,
    /// `#[bind]`, and nothing else: only a bind takes the claim.
    claims: bool,
}

impl Scheduled {
    /// What dispatch calls. A bind goes through the claim gate; a post is called as written.
    /// Either way the macro looks inside no rhs.
    fn rhs(&self) -> TokenStream2 {
        let handler = &self.handler;
        if self.claims {
            quote!(::bind::exclusive(#handler))
        } else {
            quote!(#handler)
        }
    }
}

/// One `trigger => handler` pair, the form `#[bind]` and `#[post]` share.
struct Pair {
    trigger: Expr,
    handler: Expr,
}

impl syn::parse::Parse for Pair {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let trigger = input.parse()?;
        input.parse::<Token![=>]>()?;
        let handler = input.parse()?;
        Ok(Self { trigger, handler })
    }
}

/// One `trigger => (pre, post)` pair from `#[pre_post(..)]`.
struct PrePost {
    trigger: Expr,
    pre: Expr,
    post: Expr,
}

impl syn::parse::Parse for PrePost {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let trigger = input.parse()?;
        input.parse::<Token![=>]>()?;
        let content;
        syn::parenthesized!(content in input);
        let pre = content.parse()?;
        content.parse::<Token![,]>()?;
        let post = content.parse()?;
        Ok(Self { trigger, pre, post })
    }
}

/// Every scheduled item across the node's `#[bind]`, `#[post]`, and `#[pre_post]` attributes,
/// in source order: the attributes are walked once, in the order they were written, and each
/// contributes its comma-separated pairs in the order they appear inside it.
///
/// The order is what a node's schedule means, since each item sees the state the item before it
/// left. A post keyed on what the descent did is written above the binds for exactly that
/// reason.
fn scheduled(attrs: &[syn::Attribute]) -> syn::Result<Vec<Scheduled>> {
    let mut out = Vec::new();
    for attr in attrs {
        let claims = if attr.path().is_ident("bind") {
            true
        } else if attr.path().is_ident("post") {
            false
        } else {
            if attr.path().is_ident("pre_post") {
                let parsed =
                    attr.parse_args_with(Punctuated::<PrePost, Token![,]>::parse_terminated)?;
                out.extend(parsed.into_iter().map(|p| Scheduled {
                    trigger: p.trigger,
                    pre: p.pre,
                    handler: p.post,
                    claims: false,
                }));
            }
            continue;
        };
        let parsed = attr.parse_args_with(Punctuated::<Pair, Token![,]>::parse_terminated)?;
        out.extend(parsed.into_iter().map(|p| Scheduled {
            trigger: p.trigger,
            pre: unit_pre(),
            handler: p.handler,
            claims,
        }));
    }
    Ok(out)
}
