# no-clobber

Not built. `resolution.md` is what is built.

This says what the check needs, what it guarantees, and what it costs. It is written against `bind::accumulate`, which exists, so the delta is small.

## What exists

`accumulate` walks the active tree and collects every live bind's trigger into a set, erroring on a collision.

```rust
pub fn insert_or_error<T: Eq + Hash>(out: &mut HashSet<T>, t: T) -> Result<(), BindError> {
    if out.insert(t) { Ok(()) } else { Err(BindError::DuplicateTrigger) }
}
```

`duplicate_trigger_is_error` in `crates/bind/tests/accumulate.rs` asserts it: a child rebinding an ancestor's trigger fails.

So a check already runs, and it already runs over the real active tree, including derived nodes now that `accumulate` takes a path. What it cannot do is see through a trigger that stands for more than one.

## What it misses, and why

Triggers are compared AS WRITTEN. Two triggers that claim the same event but are different values do not collide.

```rust
#[bind(Key::KeyR => refresh)]        // both presses of R
#[bind(Key::KeyR.down() => other)]   // one of them
```

`Key::KeyR` and `Key::KeyR.down()` are different values, so the `HashSet` sees two entries and says nothing. One of the two binds can never fire.

`AnyKey` is worse. In mercury today it is a unit struct whose `is_matching` returns `true` unconditionally, so `TypingLayer`'s `AnyKey => passthru` claims every key, including the `Escape` its own layer binds and every key any ancestor binds. It collides with nothing, because it is one value.

So the check is real but shallow: it catches a typo, and it misses every conflict that matters.

## What it needs: one method

```rust
fn expand(self) -> Vec<M::Trigger>;   // the concrete triggers this one claims
```

`insert_or_error` then runs per concrete trigger rather than per trigger as written. Nothing else in `bind` changes: the walk, the paths, the child fns, and `dispatch` are untouched.

Triggers form a lattice, and only its bottom is concrete.

```
AnyKey(except)     -> every (key, press) for every key not in except
Key(k)             -> (k, Down) and (k, Up)
KeyPress(k, p)     -> itself

Foregrounded       -> ForegroundedApp(a) for every a
ForegroundedApp(a) -> itself
```

`AnyKey` needs `Key::ALL`, so the enum and the list come out of one declaration. A hand-written second copy of the variants drifts the first time a key is added. `freddie_keys::Key` has 95 variants and no such list today.

## The invariant it introduces

`is_matching` and `expand` describe the same relation, and nothing makes them agree. If they drift, a bind silently never fires.

`is_matching` is the specification, because it is what `dispatch` runs. `expand` is tested against it exhaustively, over every trigger and every event:

```rust
assert_eq!(t.expand().contains(&ev.trigger()), t.is_matching(ev));
```

The space is finite, which is what the next section is about.

## What this costs: keyed dimensions must be closed

`expand` requires every dimension of a trigger to be finitely enumerable. A dimension is either IN the trigger or it is DATA on the event, and never both.

```rust
struct KeyEvent {
    key: Key,               // keyed: closed enum
    press: PressType,       // keyed: closed enum
    source: Option<Board>,  // keyed: closed. Option of a closed enum is still closed.
    board_id: String,       // DATA. A handler reads it. Nothing binds on it.
}
```

An unrecognised value collapses to `Other` and its raw id travels in the payload. `App::from_bundle_id` in mercury already does exactly this, which is why the foreground source is safe today without anyone having said so.

Put an open `String` in a trigger and `expand` is impossible.

Keying a dimension also commits you. Once `source` is keyed, a bind on `Key::KeyG` claims G on every board, and a bind on "G from the ergodox" claims one of those very entries. They collide, and the check says so. So either every bind names the dimension, or one general bind locks out every specific one, or the specific one declares an override.

## Clobbering: three modes, all on the leaf

A collision is not always a bug. The declaration goes on the bind DOING the shadowing, which is the leaf. Nothing is declared on the bind being shadowed: an ancestor cannot know which descendant will one day override it.

```rust
#[bind(Key::KeyG => on_g)]                     // no-clobber, the default
#[bind(Key::KeyG => on_g, expects_clobber)]
#[bind(Key::KeyG => on_g, may_clobber)]
```

`no-clobber` must shadow nothing. A collision is an error naming the concrete trigger.

`expects_clobber` must shadow something. Colliding is fine; colliding with NOTHING is an error, because an override written to beat a bind that has since moved is stale, and nothing else in the model can report that. The bind still fires, so no behavioural test goes red.

`may_clobber` asserts nothing in either direction, and probably should not exist. The one case that might force it is a derived node with multiple parents, where the same bind shadows under one parent and not the other. Multiple parents is not built.

For any of this to work, `accumulate` has to claim ROOT FIRST, which it does: the derive inserts a node's own triggers before descending. So a leaf landing on an occupied entry knows the entry belongs to an ancestor. `dispatch` is leaf first, so the leaf's handler is the one that runs, and the two agree.

### `expects_clobber` is forced, not an escape hatch

A node UNDER a catch-all necessarily shadows it, and there is no spelling of the program in which it does not.

```rust
// TypingLayer
#[bind(AnyKey.except(&[Key::Escape]) => passthru)]

// any node beneath it
#[bind(Key::KeyG => on_g, expects_clobber)]
```

The alternative is for the catch-all to except every key any descendant binds, which forces a layer to name keys only its children know about, and to keep doing so as they gain more.

This is not hypothetical. In `~/code/voicemode/src/karabiner.edn`, key `6` in layer 10 is bound twice: `R3138 [dsk_layer=10] 6 → Cmd+1` and `R3148 [dsk_layer=10:app=iTerm] 6 → Ctrl+A 1`. Karabiner takes the first matching rule and the app-conditioned one is emitted first, so iTerm wins. The intent is real and nothing states it.

### What the leaf-only rule costs

A leaf that declares `expects_clobber` may shadow ANY bind, including the root's quit key, and the root cannot refuse, because there is no mode on the bind being shadowed.

So the guarantee is "no bind is ever shadowed without the shadowing bind saying so", NOT "a bind on the root always fires". The killswitch is safe from every accident and from no deliberate act. Recovering the stronger property needs a second declaration site on the bind being shadowed.

## A catch-all is not exempt

`AnyKey` carries the keys it does not claim.

```rust
#[bind(AnyKey.except(&[Key::Escape]) => passthru)]
```

The except list names every key claimed by a node ABOVE it, and every key claimed by a sibling bind on its OWN node, because a catch-all is an ordinary bind and shadows a sibling exactly as it shadows an ancestor.

A `#[fallback]` attribute, deferred until every bind had missed, is worse for one reason that is not ergonomics: a fallback is exempt from clobbering by construction, so nothing could promise that a bind on the root survives a layer with a passthru beneath it. `except` keeps the catch-all inside the rules.

The cost is a list to maintain. The check is what makes that safe: not a list to get right by inspection, a list the tests correct you on.

## Totality is NOT part of this

Coverage falls out of the same enumerability: the concrete triggers are listable, so the ones nothing claimed are computable. It is a diagnostic. Nothing asserts it is empty and nothing should.

Mercury's layers are modal. In `NavLayer` the live binds are `KeyC`, `KeyG`, `KeyZ` down plus `Escape` down inherited from `Layer`, so 186 of the 190 concrete key triggers are claimed by nothing, deliberately, and `decide` in `freddie_keyboard/src/sys/macos.rs:157` maps that to `Decision::Drop`. Enforcing totality means writing 186 no-op binds.

The one-line dodge is worse. `AnyKey.except(&[KeyC, KeyG, KeyZ, Escape]) => swallow` empties the report by claiming everything, including the one key you forgot, which is the only key the report was ever going to tell you about.

## Where it runs

In tests, over every reachable state. That is the whole answer.

A clobber is a property of the PROGRAM, not of a run: it is there in every execution or in none. So a test sees everything a running binary would and sees it earlier. Deferring to startup only finds it later.

Because it is a test, its cost is irrelevant, which is why expansion into a map is the right implementation even though it is the naive one.

Enumerating the reachable states is the part that is not free, and it is not built.

## What is not built

`expand`, on the trigger.

`Key::ALL`, from the same declaration as the enum.

Press in the concrete keyboard trigger. Today `Key::KeyR` and `Key::KeyR.down()` silently shadow each other. This piece is shippable to master on its own, ahead of everything else here, and it closes a live hole.

`AnyKey.except(..)`. Today `AnyKey` is a unit struct that matches everything.

The three clobbering modes.

State enumeration, so the check can run over every reachable state rather than the ones a test happens to build.

## Prior art

The design is not novel, and its unusual choice is the strictness rather than the mechanism.

Pattern-match usefulness and exhaustiveness checking, in `rustc` and OCaml, computes these two things under different names: an unreachable arm is a shadowed bind, a non-exhaustive match is missing coverage. `Some(x)` before `Some(3)` is `Key(k)` before `KeyPress(k, Down)`. Maranget's algorithm does it by specializing a pattern matrix rather than enumerating, which is why Rust can tell you a `match` on `&str` is non-exhaustive without listing the strings. The closed-dimension rule above is a consequence of choosing naive expansion, not of wanting the check.

Trait coherence is the same overlap check on types: two overlapping impls is E0119, which is no-clobber. Specialization relaxes it to "overlap is allowed when one impl is strictly more specific", which is what Julia's multiple dispatch does, erroring only on incomparable overlap that `Test.detect_ambiguities` finds statically.

Firewall policy analysis shares the vocabulary exactly: Al-Shaer and Hamed classify rule relations as shadowing, correlation, generalization, and redundancy, and the detection is an offline pass over a rule set the classifier resolves by priority at runtime. That split is this one.

What differs here: specialization systems PERMIT strict domination silently. `Key(k)` strictly dominates `KeyPress(k, Down)`, and Julia would pick the more specific method without comment. Forbidding that by default, and making the override say so, is the point.

Emacs keymaps, vim mappings, CSS specificity, and Karabiner all resolve overlap by an ordering rule and check nothing at all.
