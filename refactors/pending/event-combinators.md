# Event combinators: `Either` and `And`

A bind is one trigger and one handler. Two triggers that should run the same handler are two bind lines today. `Either` is one trigger that matches when either arm does. `And` is the dual: both arms must match. Both require the arms to share a source event type.

Cross-source OR (timer or Escape both go home) stays two bind lines. Those arms have different `Event`s, so they cannot sit in one `EventTrigger`, and a handler that ignores the event is already generic over it (`to_home<'a, E, ...>(_ev: &E, ...)`).

`Option` is the existing unary combinator (absent matches nothing). `OnDevice` is a source-specific combinator in `freddie_keys`. These two are binary and live in `bind`, next to `Option`.

## What a bind looks like

```rust
// before: two lines, same handler
#[bind(
    Key::KeyW.down() => maximize,
    Key::UpArrow.down() => maximize,
)]

// after: one line
#[bind(
    either(Key::KeyW.down(), Key::UpArrow.down()) => maximize,
)]
```

Three or more nest:

```rust
either(either(Key::KeyH.down(), Key::LeftArrow.down()), Key::KeyA.down()) => tile_left
```

`And` is the same shape when both predicates must hold on one event:

```rust
and(some_filter, other_filter) => handler
```

Most "both of these" cases on keys are already a dedicated wrapper (`KeyPress::with`, `on_device`). `And` is for orthogonal peer triggers that share an `Event` and have no wrapper. Ship `Either` first; `And` is the same cut with `&&` instead of `||`.

## Types (`crates/bind/src/lib.rs`)

Next to the `Option` impl of `EventTrigger`:

```rust
/// Matches when either arm matches. Both arms read the same source event.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Either<A, B> {
    pub left: A,
    pub right: B,
}

/// Matches when both arms match. Both arms read the same source event.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct And<A, B> {
    pub left: A,
    pub right: B,
}

#[must_use]
pub const fn either<A, B>(left: A, right: B) -> Either<A, B> {
    Either { left, right }
}

#[must_use]
pub const fn and<A, B>(left: A, right: B) -> And<A, B> {
    And { left, right }
}

impl<A, B> EventTrigger for Either<A, B>
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

Same `Event` is a type-system fact, not a convention. `either(Key::Escape.down(), Quit)` does not compile: `KeyEvent` is not `Quit`. Cross-source stays two lines.

No `MercuryTrigger` variant for `Either` or `And`. They do not lift through `Into` as a single claim (below).

## Handler and dispatch

Unchanged shape. The arms share `Event`, so the handler still receives `&A::Event` (or owned, after `handler-event-by-ref.md`). The derive's two-level match is the same:

```rust
// generated, unchanged structure
if let Some(ev) = TryFrom::try_from(event).ok() {
    let trigger = either(Key::KeyW.down(), Key::UpArrow.down());
    if ::bind::EventTrigger::is_matching(&trigger, ev) {
        return ControlFlow::Break(
            maximize(ev, Node { parent: path, data: () }).into_iter().collect(),
        );
    }
}
```

One bind, one handler call, first matching arm wins only for the boolean; the event is the same either way. No tag says which arm matched. A handler that needs to know which key fired reads `ev`, not the trigger.

Closure triggers compose: `either(|path| path.get().a.trigger(), |path| path.get().b.trigger())` when both guards share an `Event`. The macro still builds the trigger through `trigger_expr` / `call_with`; `Either` is a value expression like any other.

## THE CHECK: one bind, two claims

`accumulate` forbids the same trigger twice on the active path. `either(Key::KeyW.down(), Key::UpArrow.down())` claims both keys. A child that binds `Key::KeyW.down()` must still error.

`Into::into` on the whole `Either` cannot do that: one value, one insert. No `From<Either<_, _>> for MercuryTrigger`. Nested `Either` claims every leaf.

```rust
/// Inserts every leaf trigger a bind expression claims.
///
/// A bare trigger claims itself (`Into` the marker's unified type). [`Either`] and [`And`]
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

impl<A, B, T> ClaimTriggers<T> for Either<A, B>
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

Coherence: `Either` and `And` must not implement `Into<M::Trigger>`. The blanket then does not apply to them, and the arm-expanding impls do. Nested combinators recurse through `ClaimTriggers`.

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

Same filter: skip `Expr::Closure` so state-read triggers still claim nothing statically. A non-closure `either(a, b)` is not a closure; both arms insert. A closure that *returns* an `Either` is still skipped (the expression is a closure). That matches today's rule for any value-from-state trigger.

Place and derived accumulate bodies both switch to `claim_triggers`. One call site shape:

```rust
// place EventHandler::accumulate
#(
    ::bind::ClaimTriggers::claim_triggers(#triggers, out)?;
)*

// derived DerivedHandler::accumulate — identical call
```

## Docs and re-exports

`crates/bind/src/lib.rs` crate docs: a trigger may be an `Either` or `And` of same-`Event` arms; THE CHECK expands them.

`either` / `and` / `Either` / `And` / `ClaimTriggers` are public. mercury can `use bind::either` at bind sites; no mercury API change is required for the combinators to exist.

## Tests (`crates/bind`)

```rust
#[test]
fn either_matches_left_or_right() {
    let t = either(Key::KeyW.down(), Key::UpArrow.down());
    assert!(t.is_matching(&key_event(Key::KeyW, PressType::Down)));
    assert!(t.is_matching(&key_event(Key::UpArrow, PressType::Down)));
    assert!(!t.is_matching(&key_event(Key::KeyA, PressType::Down)));
}

#[test]
fn and_matches_only_when_both_do() {
    // two orthogonal predicates on KeyEvent, or a fixture trigger pair
}

#[test]
fn either_claims_both_leaves() {
    // accumulate a node bound with either(a, b); set contains both Into triggers
}

#[test]
fn either_duplicate_leaf_is_duplicate_trigger() {
    // parent binds either(KeyW, UpArrow); child binds KeyW -> BindError::DuplicateTrigger
}
```

Key fixtures can live in bind's tests only if bind depends on `freddie_keys` for tests, or use tiny local trigger stubs in `bind` tests (preferred: local `struct T(u8)` implementing `EventTrigger`, no keys dependency).

## Order of changes

Each step is independently shippable.

1. `Either`, `And`, `either`, `and`, and the two `EventTrigger` impls in `bind`. Unit tests on matching. No derive change yet: a hand-built `Either` already works as a trigger value wherever a value trigger is written, if the check is off or the bind is not accumulated.

2. `ClaimTriggers` + switch `bind_macro` inserts to `claim_triggers`. Tests that accumulate expands both leaves and reports a leaf collision. This is what makes THE CHECK correct for combinator binds.

3. (optional, same PR or follow-up) adopt at a mercury site that is same-`Event` today (only if one exists; do not invent aliases). Cross-source `to_home` stays two lines.

## Out of scope

- Cross-source `Either` (`TimerFired` or `KeyEvent`). Different `Event` associated types; two bind lines.
- A tag on which arm matched. Read the event.
- Macro syntax `a | b => h`. The value is `either(a, b)`.
- Changing `OnDevice` or `Option`. They stay as they are.
