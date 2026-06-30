# freddie: precedence and dispatch model

The design space for how multiple bindings on the active path resolve to one behavior. v1 implements only the simplest point in it (static dispatch, non-clobberable); everything past that is a later addition. Terminology: leafward means closer to the active leaf (the more-specific node), rootward means closer to the root (the global default).

## v1: static and non-clobberable

Every binding is non-clobberable and dispatch is static. A trigger bound at two levels of the active path is a collision and an error caught during accumulation. One trigger, one handler, no overriding, no fall-through. This is what ships first.

## Precedence on conflict (later)

Once overriding is allowed, leafward clobbers rootward. The leafward binding wins over the rootward one. Walking up from the active leaf, the first binding seen for a given trigger wins, and bindings higher up do not overwrite it.

The constraint that forces this: there may be a global escape binding that applies to nav and the other layers, but in the typing layer escape should type an escape character. The typing layer's escape (leafward) clobbers the global escape (rootward).

## Clobberable vs non-clobberable

A binding can be marked non-clobberable. The behavior is one of:

- Non-clobberable: a leafward node attempting to override it is an error, caught during accumulation.
- Otherwise: the leafward binding clobbers it.

The global escape above is clobberable, which is why typing may override it. A binding we never want a layer to steal is marked non-clobberable. v1 treats every binding as non-clobberable, so the marker and the clobbering it enables come later.

## Static winner vs dynamic fall-through

Two ways to resolve which handler runs:

- Static. Accumulation picks a single winner per trigger (leafward clobbers rootward), and that one handler runs. Simple, but a leafward handler cannot conditionally decline.
- Dynamic fall-through. Accumulation keeps the chain of handlers per trigger, leafward first. When the event fires, each handler returns an `Option` saying whether it handled it; if the leafward handler declines, dispatch falls through to the next one rootward, up to the top. A leafward binding can then handle some cases and pass the rest to the global default.

The dynamic model can do everything the static one can; it costs an extra chain to keep and walk at dispatch. v1 ships the static model.

## A richer handled signal

The handled signal could be more than a two-state `Option`: a third outcome (handled-but-keep-going, or an explicit block) is conceivable. The need is unclear, so it is deferred.
