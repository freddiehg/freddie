# Also-binds

Not done. This is the prerequisite for `timed-layer-wrapper.md`: the return-home wrapper resets its idle timer with an also-bind, so also-bind dispatch has to exist first. Every trait, generated body, and driver change is written out below, so implementing it makes no further decisions.

## What it is

Today a node's own binds are exclusive. Dispatch descends into the active child first; if the child handles the event it `Break`s and the node's own binds never run; only on a child miss does the node try its own binds, and the first match `Break`s. At most one handler runs per event, the leafward-most.

An also-bind is a second kind of bind that fires IN ADDITION to whatever else matched, not instead of it. A node with one contributes its effects on every event that reaches it, whether or not a child (or the node's own exclusive binds) also handled that event.

The motivating case is the return-home wrapper: `AndReturnHome` resets its idle timer on every key that reaches it. See `timed-layer-wrapper.md`.

## Syntax

Two attributes on a node, told apart by name. A node may carry both; `also_bind` fires first (pre-descend), `bind` after (post-descend on a child miss):

```rust
#[bind(Key::Escape.down() => to_home)]        // exclusive: fires iff nothing deeper did
#[also_bind(AnyKey => stay)]                  // also-bind: fires alongside whatever did
```

Order among the attributes never matters: two exclusive binds cannot share a trigger on the active path (the check forbids it), and the two kinds run in fixed phases (below), so declaration order changes nothing.

## Scope: place nodes only, this cut

`#[also_bind]` is accepted on PLACE nodes (a struct or enum with a `Path`, i.e. `#[node(..)]`). It is REJECTED on derived levels (`#[derived_node]`), with a compile error: `"#[also_bind] is not yet supported on derived levels"`.

The reason is the handler shape (below): an also-bind handler takes `Node<&mut Parent, Data>` — the parent borrowed. A place node's `Dispatch` holds its `path` and can lend `&mut path` for phase 1, then use it for the descent. A derived level's `Descend` is handed its `Node` by value and consumes it to descend, so lending `&mut` and then consuming needs a borrow-then-consume restructure. Nothing needs it yet — `AndReturnHome` is a place node — so it is deferred. The contract change below still touches every generated body, derived levels included (they thread `out`); only the phase-1 also-bind section is place-only.

## The dispatch contract

Every dispatch signature gains an `out: &mut M::Output` accumulator and stops carrying the output in `Break`. `Break(())` now means only "an exclusive winner fired deeper or here"; the effects live in `out`.

`M::Output` gains no bound on the `Bindings` trait itself; the bound goes on the two generic drivers that CONSTRUCT it (`dispatch` and `SimpleRunner`), because the per-node generated impls receive `&mut out` and never build one. `Output: Default` is all that is needed — the `Extend` requirement is discharged per call site against the concrete `Output` the marker names (`Vec<MercuryEffect>`), exactly as `collect` is today.

`Dispatch::dispatch`, before:

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
    ) -> ControlFlow<M::Output, Self::Path<'a>>
    where
        Self: 'a;
}
```

after:

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        out: &mut M::Output,
    ) -> ControlFlow<(), Self::Path<'a>>
    where
        Self: 'a;
}
```

`Descend::dispatch`, before:

```rust
fn dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>;
```

after:

```rust
fn dispatch(self, event: &M::Event, out: &mut M::Output) -> ControlFlow<(), Self::Parent>;
```

The top-level `dispatch`, before:

```rust
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    match <N as Dispatch<M>>::dispatch(path, event) {
        ControlFlow::Break(out) => Some(out),
        ControlFlow::Continue(_) => None,
    }
}
```

after (constructs the accumulator; `None` iff nothing fired — an exclusive winner is `Break(())`, an also-bind that fired left `out` non-empty; the one case treated as `None` is "nothing exclusive won and no also-bind produced an effect," which is exactly "nothing to do"):

```rust
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    M::Output: Default,
    N: Dispatch<M> + 'a,
{
    let mut out = M::Output::default();
    match <N as Dispatch<M>>::dispatch(path, event, &mut out) {
        ControlFlow::Break(()) => Some(out),
        ControlFlow::Continue(_) => Some(out),
    }
    .filter(|_| /* replaced below */ true)
}
```

That `filter` is a placeholder for the emptiness test, which cannot see inside `Output` generically. Resolve it by requiring the emptiness test through a tiny bound rather than peeking: the driver returns `Some(out)` whenever the walk `Break`s (an exclusive fired) and otherwise defers to whether `out` is empty. Since "empty" is not generic, the final form threads a `fired: bool` instead, set by any handler that runs:

```rust
pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    M::Output: Default,
    N: Dispatch<M> + 'a,
{
    let mut out = M::Output::default();
    match <N as Dispatch<M>>::dispatch(path, event, &mut out) {
        ControlFlow::Break(()) => Some(out), // an exclusive winner fired
        ControlFlow::Continue(()) => None,   // no exclusive winner
    }
}
```

DECISION: on a `Continue`, return `None` even if an also-bind pushed effects. Rationale: an also-bind with no exclusive winner on the path only happens for an event no exclusive bind claims, and mercury's callers `unwrap_or_default`, so `None` and `Some(out)` are indistinguishable downstream; keeping `None ⟺ no exclusive winner` preserves bind's existing tests verbatim (they assert `None` for unbound events) and needs no `fired` flag. The rearm never hits this branch: a key that reaches `AndReturnHome` is always claimed by an exclusive bind somewhere (a leaf key, or the root's `AnyKey => maybe_pass_through`), so the walk always `Break`s and `out` (carrying the `stay` reschedule) is returned. The two `Continue`-with-also-bind-effects cases — an also-bind firing on an event no exclusive bind claims — do not arise in mercury today; if one ever does, revisit with a `fired: bool`.

`SimpleRunner`'s impl gains the same `M::Output: Default` bound; its method bodies are unchanged (they call `dispatch`, which now constructs the accumulator internally):

```rust
impl<'a, M, N> SimpleRunner<'a, M, N>
where
    M: Bindings,
    M::Output: Default,
    N: Dispatch<M> + for<'b> Place<Path<'b> = &'b mut N>,
{ /* new / next / process_event unchanged */ }
```

## The generated body

`dispatch_impl` in `bind_macro` partitions a node's binds into exclusive (`#[bind]`) and also-bind (`#[also_bind]`) — a new `co_binds(attrs)` collector beside `binds(attrs)`, reading `#[also_bind(..)]` the same way `binds` reads `#[bind(..)]` — and emits three phases: also-binds, descent, exclusive. For `AndReturnHome` (`#[also_bind(AnyKey => stay)]`, `#[bind(|p| p.get().guard.trigger() => to_home)]`, `#[resolve_into] layers`), before:

```rust
impl ::bind::Dispatch<MercuryStruct> for AndReturnHome {
    fn dispatch<'a>(
        mut path: <Self as ::bind::Place>::Path<'a>,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
    ) -> ::core::ops::ControlFlow<
        <MercuryStruct as ::bind::Bindings>::Output,
        <Self as ::bind::Place>::Path<'a>,
    > {
        let child = <ReturnHomeLayers as ::bind::Dispatch<MercuryStruct>>::dispatch(
            /* child_path */,
            event,
        )?;
        path = /* recover */;
        if let Some(ev) = Result::ok(TryFrom::try_from(event)) {
            let trigger = ::bind::call_with(&path, |p| p.get().guard.trigger());
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                return ControlFlow::Break(Iterator::collect(IntoIterator::into_iter(
                    to_home(ev, ::bind::Node { parent: path, data: () }),
                )));
            }
        }
        ControlFlow::Continue(path)
    }
}
```

after:

```rust
impl ::bind::Dispatch<MercuryStruct> for AndReturnHome {
    fn dispatch<'a>(
        mut path: <Self as ::bind::Place>::Path<'a>,
        event: &<MercuryStruct as ::bind::Bindings>::Event,
        out: &mut <MercuryStruct as ::bind::Bindings>::Output,
    ) -> ::core::ops::ControlFlow<(), <Self as ::bind::Place>::Path<'a>> {
        // Phase 1: also-binds, pre-descend, `parent` borrowed so the descent keeps the path.
        if let Some(ev) = Result::ok(TryFrom::try_from(event)) {
            let trigger = AnyKey;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                Extend::extend(out, IntoIterator::into_iter(
                    stay(ev, ::bind::Node { parent: &mut path, data: () }),
                ));
            }
        }
        // Phase 2: descend; `?` bubbles a deeper exclusive winner's `Break(())`.
        let child = <ReturnHomeLayers as ::bind::Dispatch<MercuryStruct>>::dispatch(
            /* child_path */,
            event,
            out,
        )?;
        path = /* recover */;
        // Phase 3: this node's exclusive binds.
        if let Some(ev) = Result::ok(TryFrom::try_from(event)) {
            let trigger = ::bind::call_with(&path, |p| p.get().guard.trigger());
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                Extend::extend(out, IntoIterator::into_iter(
                    to_home(ev, ::bind::Node { parent: path, data: () }),
                ));
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(path)
    }
}
```

The three per-body edits the macro makes, applied everywhere it emits a dispatch body:

- The exclusive check (phase 3, `dispatch_impl`'s `checks`) swaps `return Break(collect(into_iter(handler(..))))` for `Extend::extend(out, into_iter(handler(..))); return Break(())`. The handler node is unchanged: `Node { parent: path, data: () }`.
- The child descent (phase 2, `dispatch_body` struct and enum arms) threads `out`: `Dispatch::dispatch(child_path, event, out)?`. The `?` is on `ControlFlow<(), ChildPath>`, so `Break(())` bubbles and `Continue` yields the child path to recover, exactly as before.
- Phase 1 is new: for each also-bind, the same select-then-run as an exclusive check, but the node is `Node { parent: &mut path, data: () }` and it does not `return` — it `extend`s `out` and falls through. Emitted before the descent.

`descend_impl` (the place `Descend` that delegates to `Dispatch`), before:

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event) {
    ControlFlow::Break(out) => ControlFlow::Break(out),
    ControlFlow::Continue(path) => ControlFlow::Continue(HasParent::into_parent(path)),
}
```

after (threads `out`, and `Break` now carries `()`):

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event, out) {
    ControlFlow::Break(()) => ControlFlow::Break(()),
    ControlFlow::Continue(path) => ControlFlow::Continue(HasParent::into_parent(path)),
}
```

`derived_node_impl`, `derived_enum_node_impl`, and the `derived_child_descent` take the same signature change (`out: &mut Output`, `ControlFlow<(), _>`, `?` bubbling `Break(())`), and their exclusive checks swap `collect`+`Break(out)` for `extend`+`Break(())`. They emit NO phase-1 section (also-binds are rejected on derived levels, per Scope), so their bodies are the exclusive path only, threaded.

## The check

`accumulate_impl` is unchanged in structure. Its trigger source, `claimed_triggers`, already skips closure triggers; it now also skips also-binds entirely, because an also-bind trigger is allowed to overlap anything (it fires alongside). So the collision set is built from exclusive value triggers only, and an also-bind's `AnyKey` never collides with a leaf's key. `co_binds` are not inserted into `out` in `accumulate` at all:

```rust
fn claimed_triggers(binds: &[Binding]) -> impl Iterator<Item = &Expr> {
    binds
        .iter()
        .filter(|b| !matches!(b.trigger, Expr::Closure(_)))
        .map(|b| &b.trigger)
}
```

is unchanged; it is simply passed only the exclusive `binds`, never the `co_binds`. The `#[cfg(feature = "check")]` `EventHandler`/`DerivedHandler` bodies gain no also-bind logic.

## The handler shape

An also-bind handler keeps the `Node<parent, data>` shape an exclusive handler has, but `parent` is the path by mutable reference, not by value: `Node<&mut P, Data>`. That enforces the no-consume restriction for free — `into_parent` and `ascend_mut` take `self` by value, so they are uncallable through the `&mut`; `get`, `parent`, `ascend`, and `get_mut` take `&self`/`&mut self`, so they remain.

```rust
// exclusive handler: `parent` is the owned path, so it can ascend to the root and `set_layer`
fn to_home<'a, E, P: AscendMut<MercuryPath<'a>>>(ev: &E, node: Node<P, ()>) -> Vec<Effect>;

// also-bind handler: `parent` is `&mut` the path, so it can `get_mut` its own node and `ascend` to
// READ the root, but not `ascend_mut` to mutate it, nor `into_parent` to consume it
fn stay<'a>(_ev: &KeyEvent, node: Node<&mut AndReturnHomePath<'a>, ()>) -> [MercuryEffect; 1];
```

The handler returns an `IntoIterator` like any other (here `[MercuryEffect; 1]`, wrapping the one reschedule the wrapper's `stay` method returns), which phase 1 `extend`s into `out`. The capability is exactly `Ascend` (read up, root included) plus `get_mut`, and not `AscendMut` — the split the ascend-by-ref work already draws. The one seam is the root, whose path is `&mut Mercury` rather than a `PathMut`, so a root also-bind would need `&mut &mut Mercury`; a trait (implemented for `&mut PathMut<N, P>` and `&mut &mut Root`) unifies that surface. `AndReturnHome` is never the root, so this cut does not need the trait.

## Why an also-bind runs first

The ordering is forced, not chosen: running an also-bind after the child would stop it being also-bind at all. If the child handles the event, its handler ascends to the root and consumes the path — there is nothing to hand back, which is why a handled child returns `Break` with no path. So once the child has handled the event, no ancestor holds a path. An ancestor's also-bind is therefore only guaranteed to run if it runs before the descent. Put it after, and it runs only when the child returned the path, i.e. only when the child missed — which is exactly exclusive dispatch. Pre-descend is what makes "also" mean also.

All also-binds thus run on the way DOWN, root-to-leaf, before each descent. The exclusive `Break` still short-circuits the unwind, and nothing is lost: by the time the winner fires, every ancestor's also-bind has already run. So this is a pre-descend phase plus a threaded accumulator, not a dispatch-model rewrite; the `?`-based descent and `Break` short-circuit are preserved.

## Why "also" costs the impossibility of contradiction

Exclusive dispatch buys that exactly one handler is the sole authority for an event: one set of effects from one author, so nothing can contradict it. An also-bind breaks that — its effects and the subtree's both apply — and whether they contradict is a semantic question about the state, not a trigger collision, so the check cannot see it, and states are not enumerable. An also-bind is sound only when its effect is provably independent of everything else on the path, and independence is not checkable in general, so each one is hand-verified. That is why the check merely exempts also-bind triggers rather than trying to validate them.

## The rearm is the first user

`AndReturnHome` carries one bind of each kind:

- exclusive `|p| p.get().guard.trigger() => to_home` — the firing, on a `TimerFired` event, reaching the root through `go_home`. Post-descend.
- also-bind `AnyKey => stay` — resets the timer on every key that reaches the wrapper. Pre-descend, node-local: it mutates `self.guard` and emits the reschedule, nothing more.

It is hand-verified sound: the `stay` effect's only interaction with the rest of the path is benign. On a key that stays, it resets the clock. On a key that leaves or transitions, it fires anyway — it cannot know why the ascent is happening — and arms a fresh timer the leaving handler then drops, so the schedule self-cancels. That wasted arm is the extra work, a discarded effect, not a contradiction, and the price of putting the reset where the timer lives.

## Tests

Add to `crates/bind/tests/`, a fixture with an also-bind alongside the existing exclusive tree. Reuse `common`'s marker and helpers; the also-bind handler mutates a `hits` counter so a test can see it ran, and returns `[usize; 1]` like the others.

```rust
// A place node with BOTH an also-bind and an exclusive bind. `bump` counts and returns nothing
// visible in the exclusive channel; `on_root` is the exclusive.
#[derive(Bind)]
#[node(root)]
#[binds(Demo)]
#[bind(Keyboard("esc") => on_root)]
#[also_bind(Keyboard("g") => bump)]
struct Wrap { hits: u32, #[resolve_into] inner: Leaf }

#[derive(Bind)]
#[node(parent = WrapPath)]
#[binds(Demo)]
#[bind(Keyboard("g") => on_leaf)]
struct Leaf { hits: u32 }
```

Cases the tests pin (each asserts effects AND the `hits` counters, so both channels are checked):

- `also-bind fires alongside a child's exclusive`: send `g`. The leaf's exclusive `on_leaf` fires (leafward winner) AND `Wrap`'s also-bind `bump` fires. `wrap.hits == 1`, `leaf.hits == 1`, output carries both effects. This is the case exclusive dispatch cannot express.
- `also-bind fires with no exclusive match at that node`: send `g` in a shape where the leaf does not bind `g`; the also-bind still fires and the root's exclusive (or none) resolves the winner. Confirms phase 1 is independent of the descent's outcome.
- `also-bind does not consume`: after `g`, the exclusive winner is still the leafward one, not `Wrap` — the also-bind added, it did not claim.
- `the check exempts the also-bind`: `accumulate` over `Wrap` succeeds even though `Keyboard("g")` is both an also-bind on `Wrap` and an exclusive bind on `Leaf`; no `DuplicateTrigger`.
- `None when nothing matched`: an event no exclusive bind claims and no also-bind matches returns `None`.
- `#[also_bind] on a derived node fails to compile`: a `compile_fail` case asserting the "not yet supported on derived levels" error.

## Status

Scheduled — the prerequisite for the return-home wrapper. It is one atomic change to `bind` and `bind_macro`: the contract (signatures, the `Output: Default` driver bound, the `None`-on-`Continue` return), the three-phase generated body with the `Node<&mut P>` phase-1 shape, the check exemption, and the place-only scope with a derived-level compile error. The tests above confirm the doc; the wrapper's `stay` is the first real user.
