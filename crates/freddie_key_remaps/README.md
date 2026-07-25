# freddie_key_remaps

Shareable, pure keyboard remaps for freddie consumers (mercury, figaro, anything else on `freddie_keys`). No effects, no timers, no bind, no OS. You own a small state machine on your model, feed it `KeyEvent`s, and emit what it returns.

## What belongs here

- Dual-role / alone-vs-held keys: tap as one key, hold as a modifier (`AloneOrModifier`)
- Stateless pure rewrites (`KeyEvent` in → optional `KeyEvent` out)
- Anything whose whole contract is "keys and flags in, keys and flags out," with state only on a small owned struct

## What does not

- Ordered chords with a timeout (`jk`): `freddie::KeySequence` (needs `TimerGuard`)
- Emitting, grabbing, or posting keys: `freddie_keyboard`
- Bindings and the state tree: `bind` / `laserbeam` / the app

## Current exports

- `AloneOrModifier` — physical hold key alone → tap `alone`; held with another key → `modifier` + flag. `AloneOrModifier::caps_esc_control()` is CapsLock → Escape / Control.

## Candidates

- Shift-alone → `(` / `)`
- Number-row shift invert
- Bracket ↔ cmd+bracket
