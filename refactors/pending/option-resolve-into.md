# `Option<Child>` on a `#[resolve_into]` field

Not done.

## The gap

A `#[resolve_into]` child is always present. There is no way to say a place child may be absent.

```rust
#[resolve_into]
pub chrome: Option<ChromeApp>,      // not supported
```

Absence in the DERIVED half is already solved: a derived child fn returns `Option<Data>`, and `None` means no child in this state. Only the place half is missing it.

Today the workaround is an enum with a bindless variant, which is what mercury's `AppLayer::Other(OtherApp {})` is. That forces an empty struct for every "nothing here" case, which `resolution.md` deletes.

## It is a syntactic recognition, like `Box`

`derive_support::unbox` already does exactly this for `Box<Child>`:

```rust
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
```

A string comparison on the last path segment. `Option` slots into the same place.

The consequence, and it is accepted for `Box` already: `type Maybe<T> = Option<T>;` then `#[resolve_into] chrome: Maybe<ChromeApp>` is not recognized. The result is a missing `Dispatch` bound on `Maybe<ChromeApp>`, which is a confusing error rather than silent wrongness. A locally defined `struct Option<T>` would be treated as the real one.

## The descent: check, then project

The field lives on the node itself, so it is reached with the same `get_mut()` projection every other field descent uses, plus `.as_mut()`. The parent must not lose its path when the child is absent, and does not, because the check's borrow ends before the path moves.

```rust
if path.get_mut().child.is_some() {                             // &mut, ends here
    let node = <Child as Dispatch<M>>::dispatch(
        PathMut::from_fn(path, |np| {                           // now the path moves
            np.get_mut().child.as_mut().expect("checked above")
        }),
        event,
    )?;
    path = node.into_parent();
}
// the parent's own binds run with the path intact, present or absent
```

The derive builds this from the same `derive_support::Edge` projection as a plain field
(`|np| &mut np.get_mut().child`), with the check and the `.as_mut()` added. No new `PathMut`
method is needed: `get_mut` and field access already exist.

## Why not `ControlFlow`

The derived child fn once returned `ControlFlow<Child, Parent>` because it CONSUMED the parent to build the child and had to hand it back on absence. A field child is different: the derive holds the path and does the check itself, so nothing is consumed and there is nothing to hand back.

## What it buys

An enum whose only purpose is "present or absent" collapses to an `Option`, and the bindless variant's empty struct disappears.

## Status

Small. `unbox` shows the shape. Blocked on nothing.
