# Ascending phase-chain handlers

A handler for an event runs as a chain of per-level phases, from the focused leaf
up to the root. Each phase borrows exactly one level mutably, does in-place work
and/or a transition at that level, and hands an owned value (the *baton*) to the
phase above it. Nothing holds a child borrow across a parent reassignment, so the
borrow checker accepts it with no `Rc`, no `RefCell`, no `unsafe`. A derive macro
owns the topology, the walks, the dispatch, and the defaulting of omitted phases.

## The tree

```rust
#[derive(StateNode)]
enum Outer {
    Main(Middle),      // descent edge: payload is a StateNode
    Splash(Splash),    // a different subtree / leaf
}

#[derive(StateNode)]
enum Middle {
    Focused(Inner),    // descent edge
    Empty,             // leaf variant, no child
}

#[derive(StateNode)]
struct Inner {
    typed: Vec<char>,
}
```

`StateNode` is the topology marker. The macro reads each type and treats any
field/variant-payload whose type is itself a `StateNode` as a *descent edge*.
From that it derives, for every node type, its parent type and the projection
(the variant match) from parent to child:

```rust
// generated
trait Node { type Parent; }
impl Node for Inner  { type Parent = Middle; }
impl Node for Middle { type Parent = Outer; }
impl Node for Outer  { type Parent = (); }   // root: parent is ()

// generated projections (the unreachable! arms are discharged at the call site,
// see "Ascent invariant")
fn proj_outer_to_middle(o: &mut Outer) -> &mut Middle {
    match o { Outer::Main(m) => m, _ => unreachable!() }
}
fn proj_middle_to_inner(m: &mut Middle) -> &mut Inner {
    match m { Middle::Focused(i) => i, _ => unreachable!() }
}
```

An enum may have several descent edges (a branching tree). Which one is live is a
runtime fact decided by the active variant; the walk-down follows the live one.

## Events and handler keying

An event is a marker type:

```rust
struct F3Press;
```

A handler is keyed by the pair `(leaf type, event type)`. `F3Press` on `Inner` is
one handler; `F3Press` on some other leaf is a different one. At most one handler
per pair (override/stacking is an extension, below).

The handler is *defined at the leaf* because the leaf is where the event lands,
but it carries a function for each ancestor up to the root. Conceptually:

```
on_self  : Fn(&mut Inner,  ())  -> T1     // required
on_middle: Fn(&mut Middle, T1)  -> T2     // optional
on_outer : Fn(&mut Outer,  T2)  -> T3     // optional
                                   ^ parent of Outer is (), so T3 is the
                                     handler's result (dropped, or returned)
```

(note here optional means "not necessarily provided by the user" but conceptually not optional from the machinery perspective)

The seed handed to `on_self` is `()`. Each phase consumes the baton below it and
produces the baton above it.

## The baton: one rule that makes the whole thing compile

`T1`, `T2`, ... must be **owned values that do not borrow their level**. That is
the entire mechanism. `on_self` reduces the `&mut Inner` borrow to a summary
value; only because that value owns nothing tied to `inner` does the leaf borrow
die, which frees the dispatcher to re-acquire `&mut Middle` for the next phase. A
baton that borrowed `Inner` would keep `Outer` borrowed and the next phase could
not run. The baton is a relay token, not a reference.

If a phase needs the old level's data to build the new one (e.g. `Middle::Focused
-> Middle::Empty` carrying `Inner`'s buffer), it either packs that data into the
baton, or the phase above uses `mem::replace`/`mem::take` on its own `&mut` to
move the old variant out by value. Both keep the baton owned.

## Ascent invariant (why the re-walk between phases is sound)

The dispatcher holds `&mut Outer` for the whole event. It runs the phases
bottom-up, and **re-walks from the root for each phase**, because each baton
return has dropped the previous level's borrow. Concretely the walks are:

- `on_self`: walk `Outer -> Middle -> Inner` (two projections)
- `on_middle`: walk `Outer -> Middle` (one projection)
- `on_outer`: `&mut Outer` directly (no projection)

This is sound because **phase k can only mutate at level k or below**, bounded by
its borrow, while phase k+1 walks to a strictly *higher* level:

- `on_self` holds only `&mut Inner`. It cannot retag `Middle` or `Outer`. So the
  walk to `Middle` for the next phase still matches, and that projection's
  `unreachable!` arm is dead.
- `on_middle` may retag `Middle`'s own variant, but `on_outer` takes `Outer`
  whole and projects into nothing, so it does not care.

Each walk therefore lands *before* any mutation at or above its target, and no
walk passes through a level a prior phase already restructured. The ascending
order is what discharges every `unreachable!`.

## Omitted phases

Ancestor phases are optional. The open decision is what an omitted level does to
the baton. Two coherent choices:

**Passthrough (recommended).** An omitted level threads its incoming baton
unchanged and is not walked at all. Each defined phase's input type is the output
of the nearest *defined* phase below it, or `()` if none below is defined. So with
`on_self -> T1` and `on_middle` omitted, `on_outer` receives `T1`. Clean types, no
silent drops, and the bottom seed `()` falls out as the all-omitted case.

**Unit-reset (the version you sketched).** An omitted level produces `()`. With
`on_self -> T1` and `on_middle` omitted, `on_outer` receives `()` and `T1` is
discarded. Each level's types stay independent, but defining `on_self` and
`on_outer` while skipping `on_middle` silently throws away `T1`, which is a
footgun.

Pick one project-wide. The rest of this doc assumes passthrough. Under either
rule, the macro can elide the walk for an omitted level entirely; an omitted level
has no body, so there is nothing at that level to borrow.

## What you write

```rust
handler! {
    leaf  = Inner,
    event = F3Press,

    on_self(inner, _ev) -> Vec<char> {
        let snapshot = inner.typed.clone();
        inner.typed.clear();
        snapshot                       // T1 = Vec<char>
    },

    // optional; name the ancestor type, the macro checks it is on Inner's path
    on(Outer)(outer, dropped: Vec<char>) {
        // on(Middle) omitted, so under passthrough `dropped` is the T1 above
        *outer = Outer::Splash(Splash::from(dropped));
        // no explicit return -> T_outer = (); parent of Outer is () -> done
    },
}
```

You provide `on_self` (required) and any subset of ancestor phases. You name each
ancestor's type so the macro can verify it really is an ancestor of `Inner` and
order the phases by depth. Input types are dictated by the omit rule above; if you
declare an input type that disagrees with what the chain produces, the macro
errors rather than coercing.

## What the macro generates

For each registered `(leaf, event)` the macro emits a fully monomorphized dispatch
arm, and per event it emits one dispatcher that matches down to the active leaf and
runs that leaf's arm. Nothing is dynamic; whether a leaf has an `F3Press` handler
is known at compile time, so each leaf branch is either the inlined chain or a
no-op.

```rust
// generated, for the example above (on(Middle) omitted -> passthrough)
pub fn dispatch_f3_press(root: &mut Outer, ev: F3Press) {
    // 1. find the active leaf; only Inner registers F3Press here
    if !matches!(root, Outer::Main(Middle::Focused(_))) {
        return; // unhandled
    }

    // 2. phase: Inner
    let t1 = {
        let inner = match root {
            Outer::Main(Middle::Focused(i)) => i,
            _ => unreachable!(), // discharged by the matches! above
        };
        <Inner as Handle<F3Press>>::on_self(inner, ev)
    }; // leaf borrow (and the Outer/Middle reborrows it rode on) released here

    // 3. phase: Middle is omitted under passthrough -> thread t1, no walk
    let t2 = t1;

    // 4. phase: Outer
    let _t3 = <Inner as Handle<F3Press>>::on_outer(root, t2);
    // parent of Outer is (): _t3 dropped
}
```

If `on(Middle)` had been defined, step 3 becomes a re-walk:

```rust
    let t2 = {
        let middle = match root {
            Outer::Main(m) => m,
            _ => unreachable!(), // on_self held only &mut Inner; Outer's tag is intact
        };
        <Inner as Handle<F3Press>>::on_middle(middle, t1)
    };
```

The `Handle<E>` trait and its per-leaf impls are generated from the `handler!`
blocks; the ancestor methods get concrete receiver types from the topology, and
omitted methods are filled per the omit rule.

## Terminal baton

`Outer`'s parent is `()`, so the top phase's output `T3` is the handler's result.
Two uses:

- `()` and dropped, for handlers whose effect is purely the mutations they made.
- A real signal back to the dispatch loop: `enum Flow { Consumed, Bubble }`, an
  error, a redraw request. If `T3` means something, the per-event dispatcher
  should return it rather than drop it.

## Assumptions and constraints

- **Unique type path.** Ascent presumes each node type has one parent type, so a
  type's ancestors are well defined. If `Inner` appeared as a payload in two
  places, "the middle" is ambiguous. Enforce one-position-per-type (newtype per
  position if needed), or key handlers by path rather than by leaf type.
- **One handler per `(leaf, event)`.** Stacking/override is an extension.
- **Phases mutate at their level or below only.** This is structural (each phase
  holds only its own `&mut`), and it is what the ascent invariant relies on. A
  phase cannot reach up; reaching up is what the *next* phase is for.

## Extensions

- **Stop propagation.** Make the baton `ControlFlow<Stop, T>`; a phase returning
  `Break` halts the ascent. The generated dispatch checks between phases and
  returns early. This is `stopPropagation` for the tree.
- **Multiple handlers / inheritance.** Allow several `handler!` blocks per pair and
  run them in a defined order, or let an ancestor type register a fallback that
  runs when no leaf handles the event.
- **Capture phase.** A symmetric descending pass before the ascending one, if you
  want ancestors to pre-empt the leaf (root-to-leaf capture, then leaf-to-root
  bubble), mirroring DOM event flow.

## Cost

Per event: depth phases, each re-walking O(depth) projections from the root, so
O(depth^2) variant matches plus the handler bodies. Depth is a handful, so this is
noise. Everything is monomorphized and inlined; there is no dispatch table, no
boxing, no allocation beyond whatever the handler bodies themselves do.
