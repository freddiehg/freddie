# Path 3

## Goal

- We have a nested data structure `Outer -> Middle -> Inner`. `Outer` and `Middle` are enums. We have a handler defined on `Inner`. We want to call that handler.
- That handler gets a `InnerPath<'a>` that can provide a mutable reference to `Inner`; it can also consume itself and get `MiddlePath`, etc until the root
- We iterate the data structure twice: first to create `InnerPath` and secondly to call `get_mut` at some level

## Impl

```rs
enum Outer { Middle(Middle), Inner(Inner) }
enum Middle { Vec<Inner(Inner)> }
struct Inner {}

enum ResolutionNode<'a> {
    Outer(OuterPath<'a>),
    Middle(MiddlePath<'a>),
    Inner(InnerPath<'a>),
}

struct Path<Parent, Inner> {
    parent: Path<Parent>,
    
}
```
