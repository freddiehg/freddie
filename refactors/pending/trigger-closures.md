# a trigger that reads the state it is bound on

A trigger is a value: `Key::KeyR`, `AnyKey`, `Quit`. Some bindings want one that depends on the node they are bound on — "this timer's firing, not another's", "this key, but only while something is armed" — and there is no way to write that.

There nearly is, by accident. `bind_macro` parses a trigger as a `syn::Expr` and emits `let trigger = #trigger;` INSIDE the generated `dispatch`, where the node's path is in scope. So `#[bind(ArmedTimer(path.overlay_id()) => hide_overlay)]` compiles and dispatches correctly today — I ran it. But `path` is only what that function happens to call its parameter, a derived level's generated body calls it `node`, and neither is documented: a binding written that way captures a macro-internal name, and renaming that parameter would break it silently.

So the trigger may be written as a closure, and the macro calls it with the path rather than evaluating it.

```rust
#[bind(
    |root| ArmedTimer(root.jk_timer_id()) => jk_timeout,
    Quit => quit,
)]
```

The binding names its own parameter, nothing is captured invisibly, and every constant trigger is written exactly as it is today.

## what it receives

Whatever dispatch is holding for that node, by unique reference:

- a place node's path, `&mut Self::Path<'a>`. The root's path is `&mut Mercury`, so a closure there reads fields directly through auto-deref; a deeper node's is a `PathMut`, so it reads its node through `get_mut` and the level above through `parent`.
- a derived level's `&mut Node<Parent, Data>`, since a derived level has no path. Its own struct is `node.data`.

Unique rather than shared because `PathMut`'s only accessor is `get_mut`: the projection re-derives the node FROM the parent, so reading the node needs a unique borrow of the path. That is also why `get_mut` and `parent` cannot be held at once, which a trigger computing a value never needs.

The borrow ends before `path` or `node` moves into the handler, so this composes with what dispatch already does.

Nothing stops a closure mutating through `get_mut`. It should not, and no lint says so; a read-only view over the path would need laserbeam to store a second, shared projection at every level, which is a bigger change than the problem deserves.

## why the form is syntactic

The macro matches `Expr::Closure`: a call for a closure, an evaluation for anything else.

The unified alternative — one trait, blanket-implemented for `T: EventTrigger` and for `F: FnOnce(&mut P) -> T`, so the macro emits one call either way — does not compile. The two blankets overlap, and rustc cannot prove no type is both an `EventTrigger` and an `Fn`. It is the overlap rule rather than the orphan rule, since both impls would live in `bind`, and that matters because the workarounds differ: a local newtype answers an orphan problem, and this one would need the wrapper applied at every call site, which is the syntax the closure exists to avoid.

Syntactic is unambiguous where it counts, because a closure can never be a valid trigger: closures do not implement `EventTrigger`. The seam is indirection, `make_trigger() => handler` where the function returns a closure, which is treated as a value and fails with a trait error rather than being called. Nothing is written that way today.

## change 1: the macro calls a closure trigger

`crates/bind_macro/src/lib.rs`. The trigger expression is emitted at FOUR sites, not one, because THE CHECK evaluates it as well:

- `dispatch_impl`, a place node's `Dispatch::dispatch`: `let trigger = #trigger;`, with `path` in scope.
- `derived_node_impl`, a derived level's `Descend::dispatch`: the same line, with `node` in scope.
- `accumulate_impl`, a place node's `EventHandler::accumulate`: `insert_or_error(out, Into::into(#triggers))?`, with `path` in scope.
- `derived_node_impl`'s `DerivedHandler::accumulate`: the same, with `node` in scope.

Miss the accumulate pair and the `check` feature stops compiling the moment anyone writes a closure, because a closure has no `Into<Trigger>`.

One helper, used by all four:

```rust
/// The expression that produces a binding's trigger, given what dispatch is holding for this node.
///
/// A closure is CALLED with it, so a trigger can depend on the state it is bound on; anything else
/// is evaluated as the value it is. The distinction is syntactic because a trait cannot make it:
/// blanket impls for values and for closures overlap.
fn trigger_expr(trigger: &Expr, state: &TokenStream2) -> TokenStream2 {
    if matches!(trigger, Expr::Closure(_)) {
        quote!((#trigger)(#state))
    } else {
        quote!(#trigger)
    }
}
```

`Expr` is already imported by `bind_macro`.

`dispatch_impl`'s check, before:

```rust
    let checks = binds.iter().map(|b| {
        let trigger = &b.trigger;
        let handler = &b.handler;
        quote! {
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = #trigger;
```

after:

```rust
    let checks = binds.iter().map(|b| {
        let trigger = trigger_expr(&b.trigger, &quote!(&mut path));
        let handler = &b.handler;
        quote! {
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = #trigger;
```

`derived_node_impl`'s check is the same edit with `&quote!(&mut node)`.

The two accumulate sites take the trigger through the same helper. `accumulate_impl`, before:

```rust
    let triggers = binds.iter().map(|b| &b.trigger);
```

after:

```rust
    let triggers = binds
        .iter()
        .map(|b| trigger_expr(&b.trigger, &quote!(&mut path)));
```

and `derived_node_impl`'s with `&quote!(&mut node)`.

## change 2: a closure trigger forces `mut`

The path is passed by unique reference, so a node with a closure trigger needs its path binding to be `mut`. `needs_mut` comes out of `dispatch_body` and `accumulate_body` today, computed from the node's SHAPE (whether it descends into a child), and neither of them sees the binds.

So the flag is OR-ed at the two call sites that have both. `dispatch_impl`, before:

```rust
    let (recurse, children, needs_mut) = dispatch_body(input, name, marker, root)?;
```

after:

```rust
    let (recurse, children, needs_mut) = dispatch_body(input, name, marker, root)?;
    // A closure trigger reads the path, so the binding has to be `mut` even for a leaf that
    // never reassigns it.
    let needs_mut = needs_mut || binds.iter().any(|b| matches!(b.trigger, Expr::Closure(_)));
```

`accumulate_impl` gains the same two lines. A derived level needs nothing: its `let node = self;` is already owned and rebindable.

Watch for `unused_mut` on the way through: a node whose only reason for `mut` is a closure trigger does use it, through the `&mut path` the closure is called with, so the lint stays quiet. Verify that rather than assume it, since `-D unused` is on.

## change 3: the derive says so

`crates/bind/src/lib.rs`, the `Bind` derive's docs and the `EventTrigger` docs both describe a trigger as a value. They gain the closure form: what it is called with per node kind, that it must not mutate, and that `node` and `parent` cannot be held at once.

Without this the feature is undiscoverable, which is the whole complaint about the accidental version.

## change 4: tests

`crates/bind/tests/`, over a small tree built for it: a root with `armed: Option<u64>`, a child place node with a field of its own, and a trigger type `Armed(Option<u64>)` whose `is_matching` compares against the event's id.

- a closure trigger matching on state: armed with an id, the matching event dispatches to the handler.
- the same trigger not matching: a different id, and `dispatch` returns `None` because no binding matched, rather than the handler running and declining.
- a node holding nothing: the trigger produces a value that matches no event.
- a constant trigger beside a closure one on the same node, both dispatching correctly, so the two forms coexist.
- a closure reading `parent()` on a deeper node, so the upward direction is exercised.

I verified the first three against mercury as a scratch experiment (a root field `armed_id: Option<u64>` bound as `Armed(path.armed_id) => on_fired`, which is the accidental form this change replaces); they belong in `bind`'s own tests, written against a tree that exists for testing.

One case has to run under the `check` feature, since two of the four emit sites are only compiled there: a node with a closure trigger, put through `accumulate`, asserting it collects the trigger the closure produced. Without it, the accumulate half of change 1 is untested and breaks silently for anyone who turns the feature on.

## what this does not do

It does not make the trigger set static. `accumulate` collects trigger values by walking the tree, so a trigger read from state answers "no duplicates in this state" rather than "no duplicates ever". Nothing calls `accumulate` in mercury today, and `refactors/pending/no-clobber.md` is where that question lives.
