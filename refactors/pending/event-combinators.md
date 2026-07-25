# Event combinators: `Or` and `And`

A bind is one trigger and one handler. Two triggers that should run the same handler are two bind lines today. `Or` is one trigger that holds both arms and matches when either does. `And` is the dual: both arms must match. Both require the arms to share a source event type.

Cross-source OR (timer or Escape both go home) stays two bind lines. Those arms have different `Event`s, so they cannot sit in one `EventTrigger`, and a handler that ignores the event is already generic over it (`to_home<'a, E, ...>(_ev: &E, ...)`).

`Option` is the existing unary combinator (absent matches nothing). `OnDevice` is a source-specific combinator in `freddie_keys`. These two are binary and live in `bind`, next to `Option`.

## Not the `either` crate

The `either` crate's `Either<L, R>` is a sum type: `Left(L) | Right(R)`, one value of one type. A trigger combinator has to hold both arms at once and ask each `is_matching`. That is a product, not a sum. Reusing `either::Either` would be the wrong algebra and the wrong name in this codebase.

There is no standard library type for "pair of predicates, match if either/both". The types below are local to `bind`.

## What a bind looks like

```rust
// before: two lines, same handler
#[bind(
    Key::KeyW.down() => maximize,
    Key::UpArrow.down() => maximize,
)]

// after: one line
#[bind(
    or(Key::KeyW.down(), Key::UpArrow.down()) => maximize,
)]
```

Three or more nest:

```rust
or(or(Key::KeyH.down(), Key::LeftArrow.down()), Key::KeyA.down()) => tile_left
```

`And` is the same shape when both predicates must hold on one event:

```rust
and(some_filter, other_filter) => handler
```

Most "both of these" cases on keys are already a dedicated wrapper (`KeyPress::with`, `on_device`). `And` is for orthogonal peer triggers that share an `Event` and have no wrapper. Ship `Or` first; `And` is the same cut with `&&` instead of `||`.

## Types (`crates/bind/src/lib.rs`)

Next to the `Option` impl of `EventTrigger`:

```rust
/// Matches when either arm matches. Holds both arms; both read the same source event.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Or<A, B> {
    pub left: A,
    pub right: B,
}

/// Matches when both arms match. Holds both arms; both read the same source event.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct And<A, B> {
    pub left: A,
    pub right: B,
}

#[must_use]
pub const fn or<A, B>(left: A, right: B) -> Or<A, B> {
    Or { left, right }
}

#[must_use]
pub const fn and<A, B>(left: A, right: B) -> And<A, B> {
    And { left, right }
}

impl<A, B> EventTrigger for Or<A, B>
where
    A: EventTrigger,
    B: EventTrigger<Event = A::Event>,
{
    type Event = A::Event;

    fn is_matching(&self, event: &Self::Event) -> bool {
        self.left.is_matching(event) || self.right.is_matching(event)
    }
}

impl<A, B> EventTrigger for And<A, B>
where
    A: EventTrigger,
    B: EventTrigger<Event = A::Event>,
{
    type Event = A::Event;

    fn is_matching(&self, event: &Self::Event) -> bool {
        self.left.is_matching(event) && self.right.is_matching(event)
    }
}
```

Same `Event` is a type-system fact, not a convention. `or(Key::Escape.down(), Quit)` does not compile: `KeyEvent` is not `Quit`. Cross-source stays two lines.

No `MercuryTrigger` variant for `Or` or `And`. They do not lift through `Into` as a single claim (below).

## Handler and dispatch

Unchanged shape. The arms share `Event`, so the handler still receives `&A::Event` (or owned, after `handler-event-by-ref.md`). The derive's two-level match is the same:

```rust
// generated, unchanged structure
if let Some(ev) = TryFrom::try_from(event).ok() {
    let trigger = or(Key::KeyW.down(), Key::UpArrow.down());
    if ::bind::EventTrigger::is_matching(&trigger, ev) {
        return ControlFlow::Break(
            maximize(ev, Node { parent: path, data: () }).into_iter().collect(),
        );
    }
}
```

One bind, one handler call; the event is the same either way. No tag says which arm matched. A handler that needs to know which key fired reads `ev`, not the trigger.

Closure triggers compose: `or(|path| path.get().a.trigger(), |path| path.get().b.trigger())` when both guards share an `Event`. The macro still builds the trigger through `trigger_expr` / `call_with`; `Or` is a value expression like any other.

## THE CHECK: one bind, two claims

`accumulate` forbids the same trigger twice on the active path. `or(Key::KeyW.down(), Key::UpArrow.down())` claims both keys. A child that binds `Key::KeyW.down()` must still error.

`Into::into` on the whole `Or` cannot do that: one value, one insert. No `From<Or<_, _>> for MercuryTrigger`. Nested `Or` claims every leaf.

```rust
/// Inserts every leaf trigger a bind expression claims.
///
/// A bare trigger claims itself (`Into` the marker's unified type). [`Or`] and [`And`]
/// claim both arms, so THE CHECK sees the same keys the dispatch match can fire on.
pub trait ClaimTriggers<T: Eq + Hash> {
    fn claim_triggers(self, out: &mut HashSet<T>) -> Result<(), BindError>;
}

impl<T, U> ClaimTriggers<T> for U
where
    T: Eq + Hash,
    U: Into<T>,
{
    fn claim_triggers(self, out: &mut HashSet<T>) -> Result<(), BindError> {
        insert_or_error(out, self.into())
    }
}

impl<A, B, T> ClaimTriggers<T> for Or<A, B>
where
    T: Eq + Hash,
    A: ClaimTriggers<T>,
    B: ClaimTriggers<T>,
{
    fn claim_triggers(self, out: &mut HashSet<T>) -> Result<(), BindError> {
        self.left.claim_triggers(out)?;
        self.right.claim_triggers(out)
    }
}

impl<A, B, T> ClaimTriggers<T> for And<A, B>
where
    T: Eq + Hash,
    A: ClaimTriggers<T>,
    B: ClaimTriggers<T>,
{
    fn claim_triggers(self, out: &mut HashSet<T>) -> Result<(), BindError> {
        self.left.claim_triggers(out)?;
        self.right.claim_triggers(out)
    }
}
```

Coherence: `Or` and `And` must not implement `Into<M::Trigger>`. The blanket then does not apply to them, and the arm-expanding impls do. Nested combinators recurse through `ClaimTriggers`.

`Option` stays out of this: closure triggers that produce `Option<_>` are still skipped by the check (value-from-state, not a static claim), as today.

### derive change (`bind_macro`)

`claimed_triggers` / the insert loop, before:

```rust
::bind::insert_or_error(out, ::core::convert::Into::into(#triggers))?;
```

after:

```rust
::bind::ClaimTriggers::claim_triggers(#triggers, out)?;
```

Same filter: skip `Expr::Closure` so state-read triggers still claim nothing statically. A non-closure `or(a, b)` is not a closure; both arms insert. A closure that *returns* an `Or` is still skipped (the expression is a closure). That matches today's rule for any value-from-state trigger.

Place and derived accumulate bodies both switch to `claim_triggers`. One call site shape:

```rust
// place EventHandler::accumulate
#(
    ::bind::ClaimTriggers::claim_triggers(#triggers, out)?;
)*

// derived DerivedHandler::accumulate — identical call
```

## Docs and re-exports

`crates/bind/src/lib.rs` crate docs: a trigger may be an `Or` or `And` of same-`Event` arms; THE CHECK expands them.

`or` / `and` / `Or` / `And` / `ClaimTriggers` are public. mercury can `use bind::or` at bind sites; no mercury API change is required for the combinators to exist.

## Tests (`crates/bind`)

Local trigger stubs (`struct T(u8)` implementing `EventTrigger`), no `freddie_keys` dependency.

```rust
#[test]
fn or_matches_left_or_right() {
    let t = or(T(1), T(2));
    assert!(t.is_matching(&1));
    assert!(t.is_matching(&2));
    assert!(!t.is_matching(&3));
}

#[test]
fn and_matches_only_when_both_do() { /* … */ }

#[test]
fn or_claims_both_leaves() {
    // accumulate a node bound with or(a, b); set contains both Into triggers
}

#[test]
fn or_duplicate_leaf_is_duplicate_trigger() {
    // parent binds or(a, b); child binds a -> BindError::DuplicateTrigger
}
```

## Order of changes

Each step is independently shippable.

1. `Or`, `And`, `or`, `and`, and the two `EventTrigger` impls in `bind`. Unit tests on matching. No derive change yet: a hand-built `Or` already works as a trigger value wherever a value trigger is written, if the check is off or the bind is not accumulated.

2. `ClaimTriggers` + switch `bind_macro` inserts to `claim_triggers`. Tests that accumulate expands both leaves and reports a leaf collision. This is what makes THE CHECK correct for combinator binds.

3. (optional, same PR or follow-up) adopt at a mercury site that is same-`Event` today (only if one exists; do not invent aliases). Cross-source `to_home` stays two lines.

## Out of scope

- Cross-source `Or` (`TimerFired` or `KeyEvent`). Different `Event` associated types; two bind lines.
- A tag on which arm matched. Read the event.
- Macro syntax `a | b => h`. The value is `or(a, b)`.
- Depending on the `either` crate for this. Wrong shape (sum vs product).
- Changing `OnDevice` or `Option`. They stay as they are.
