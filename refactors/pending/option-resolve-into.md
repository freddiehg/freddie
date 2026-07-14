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

The parent must not lose its path when the child is absent. It does not, because the check borrows and the borrow ends before the path moves.

```rust
if path.parent().chrome.is_some() {                      // &, ends here
    let child = <ChromeApp as Dispatch<M>>::dispatch(
        Path::from_fn(path, |p| {                        // now the path moves
            p.parent_mut().chrome.as_mut().expect("checked above")
        }),
        event,
    )?;
    path = child.into_parent();
}
// the parent's own binds run with the path intact, present or absent
```

Compiled.

`parent_mut()` is not on `laserbeam::Path` today. `parent()` is. This projection needs the mutable one, which is three lines.

## Why not `ControlFlow`

The derived child fn once returned `ControlFlow<Child, Parent>` because it CONSUMED the parent to build the child and had to hand it back on absence. A field child is different: the derive holds the path and does the check itself, so nothing is consumed and there is nothing to hand back.

## What it buys

An enum whose only purpose is "present or absent" collapses to an `Option`, and the bindless variant's empty struct disappears.

## Status

Small. `unbox` shows the shape. Blocked on nothing.
