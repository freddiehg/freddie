# tracking every held key at the root

Not built, punted from the passthrough work. `held-modifiers.md` (now in `past`) tracks the four MODIFIER keys at the root; this is the same idea for EVERY key, motivated by a stream-balance bug that is not worth solving on its own.

## The bug

Entering typing swallows the trigger key's down but leaks its up:

```
KeyT Down -> effects=[]                (home binds t: to_typing, swallowed, enters typing)
KeyT Up   -> effects=[Emit(KeyT Up)]   (typing is a passthrough layer, so the root re-emits it)
```

`t`-down is consumed by the `to_typing` command; `t`-up then falls to the root's `maybe_pass_through`, which is in a passthrough layer now, so it emits it. The app gets a `t`-up it never saw a `t`-down for.

More generally: mercury swallows some downs (commands, and anything unbound in a command layer) and passes others (unbound in a passthrough layer). The emitted stream is balanced only if it emits an up exactly when it emitted the down. Today the up follows the CURRENT layer's rule, not what happened to the down, so any non-modifier key held across a swallow/pass boundary desyncs:

- swallowed down, passed up: the `t` leak above, an unmatched up.
- passed down, swallowed up: hold a letter in typing, `cmd`-`escape` to home, release it. The down was passed (app saw it), the up is swallowed in home, so the app is left with the key stuck down.

Modifiers do not have this problem: no modifier is a command trigger, and the open/close sweeps re-assert and release held modifiers across every transition, so their stream stays balanced.

## The fix in principle

Emit an up iff we emitted the down. That needs a record of the non-modifier keys whose down we passed through:

- non-modifier down in a passthrough layer: emit it, remember it.
- non-modifier down in a command layer: swallow it, do not remember.
- non-modifier up: emit iff remembered, then forget; else swallow.

That is the balance, and it fixes both cases.

## Why it was punted: two unresolved questions

### Where the record lives

The exit-hold case (passed down in typing, up arrives back in home) needs the record to OUTLIVE the typing layer: the key was remembered while in typing, but its up lands after we left. So a record ON the `TypingLayer`, dropped when typing is torn down, does not fix that case, and a record on the root does.

But a record always present on the root is state that only matters while a passthrough layer is active. The cleaner shape makes it exist only then: `is_passthrough` returning `&mut Option<PassthroughState>` (Some only in a passthrough layer) rather than a bool, so the passthrough state lives exactly as long as the passthrough layer. That directly conflicts with the outlive-the-layer requirement above. The two cases want opposite lifetimes; reconciling them, or deciding the exit-hold case is not worth handling, is the open question.

### Representation

A `HashSet<Key>` is too heavy, and wrong in spirit: `Key` has a `Raw(u16)` variant, so a hash set leans on a hand-rolled `Eq`/`Hash` over a type that is otherwise a small enum. What we want is a set of keys, i.e. a bitset, one bit per key. A `u128` (or `[bool; N]`) over the named keys is the shape, except `Key::Raw(u16)` does not fit a small dense index, so the bitset has to handle or exclude raw codes.

There is probably a crate for exactly this, an enum-keyed bitset. `enumset` wants unit-only variants, which `Key::Raw(u16)` breaks; `fixedbitset`/`bitvec` over a key index would work once we define the index. Picking the representation is the second open question, and ties into whether `Key` should grow a dense-index form at all.

## Verdict

The only symptom is a stray key-up, plus a rare stuck key on exit-hold, which is low-value to chase in a vacuum. Fold it into a general "track every held key at the root" pass whenever that happens (the same `u128`-over-every-key representation), alongside deciding the `is_passthrough -> &mut Option<state>` lifetime question. Not built for now.
