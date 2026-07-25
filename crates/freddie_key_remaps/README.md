# freddie_key_remaps

Shareable, pure keyboard remaps for freddie consumers. No effects, no timers, no bind, no OS. You own a small state machine on your model, feed it `KeyEvent`s, and emit what it returns.

## Current exports

- `AloneOrModifier` — physical hold key alone → tap (key + flags); held with another key → modifier + flag.
  - `caps_esc_control()` — CapsLock → Escape / Control
  - `left_shift_open_paren()` — left shift → `(` / Shift
  - `right_shift_close_paren()` — right shift → `)` / Shift
  - `AloneOrModifier::new(hold, alone, alone_flags, modifier, flag)` for other dual-roles
- `shift_reverse` — bare ↔ shift-only on a `KeyEvent` (number-row invert, `\` ↔ `|`, …)

## What belongs here

- Dual-role / alone-vs-held keys
- Stateless pure rewrites (`KeyEvent` in → optional `KeyEvent` out)
- Anything whose whole contract is keys and flags in/out, with state only on a small owned struct

## What does not

- Ordered chords with a timeout (`jk`): `freddie::KeySequence`
- Emitting or grabbing keys: `freddie_keyboard`
- Bindings and the state tree: `bind` / `laserbeam` / the app
