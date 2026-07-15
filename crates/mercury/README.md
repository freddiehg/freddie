# mercury

A small, runnable demo of [freddie](../..) (`laserbeam` + `bind`): a layered keyboard remapper modeled as a state tree, with an executable and unit tests. Nothing here touches the real machine; effects are printed.

## The model

The state is an outer `Mercury { foregrounded, layer }` that resolves into a `Layer`:

- `Home` (default): `n` enters nav, `space` enters typing.
- `Nav`: `c`/`g`/`t`/`z` open Chrome/Ghostty/TTY/Zed.
- `Typing`: `a`/`s`/`d`/`f` type themselves.
- `InApp`: a per-app layer. Chrome rebinds `r` to `cmd`+`r`; the terminals rebind `d` to `cmd`+`d`; an unknown app binds nothing.

`escape` returns to `Home` from anywhere. Every node derives `Bind`, which emits its path type and its accumulate and dispatch impls.

## Events, effects, and the loop

Dispatch takes an event (a key, or an app being foregrounded) and returns the effect the active state binds for it — an inert `MercuryEffect` (type a letter, open an app, send a command). Dispatch never performs effects and never knows an effect can cause an event.

Performing effects is the job of an `EffectHandler`, driven by `drive`. Opening an app emits `Foreground(app)`; the handler is what turns that into the follow-up "app foregrounded" event, and it can fail — you might not have the app, in which case there is no follow-up and the state never enters that in-app layer. So `home → n → c → r` becomes: go to nav, open Chrome, Chrome comes up (its foreground event enters the in-app layer), `r` sends `cmd`+`r`.

## Run

```
cargo run -p mercury
```

Type one key per line; it prints the effects and shows the active layer and the keys currently bound. Or pipe a sequence:

```
printf 'n\nc\nr\n' | cargo run -p mercury
```

## Test

```
cargo test -p mercury
```

The per-event tests send one event and assert the effect; `kitchen_sink` runs a whole sequence through `drive`, and `opening_a_missing_app_does_not_enter_in_app` covers the failing-handler path.
