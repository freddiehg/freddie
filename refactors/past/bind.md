# bind: accumulation

bind is a crate within freddie, split into `bind` (the traits, the free fn, and the entry point) and `bind_macro` (the `#[derive(Bind)]`). It builds on laserbeam, because the edges accumulation recurses are laserbeam's `#[resolve_into]` fields and enum variants. Dispatch lives in `freddie-keys-plan.md`.

Accumulation forbids clobbering: a trigger is bound at most once on any active path. If a child rebinds a trigger an ancestor already bound, accumulation returns an error.

## What the app defines

The app defines its source types, one unified `MercuryTrigger` enum over them, and a marker that names that enum.

```rust
use std::collections::HashSet;
use std::hash::Hash;

// each source has one struct; they serve as set keys, so they derive Hash and Eq.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Keyboard(String);
impl Keyboard { fn new(k: &str) -> Self { Keyboard(k.to_owned()) } }

#[derive(Clone, PartialEq, Eq, Hash)]
struct Foreground(String);
impl Foreground { fn new(app: &str) -> Self { Foreground(app.to_owned()) } }

// the unified trigger enum has one variant per source. derive_more::From gives
// Keyboard -> MercuryTrigger and Foreground -> MercuryTrigger, so `.into()` lifts either.
#[derive(Clone, PartialEq, Eq, Hash, derive_more::From)]
enum MercuryTrigger {
    Keyboard(Keyboard),
    Foreground(Foreground),
}

// the marker names the MercuryTrigger enum for the whole app.
struct MercuryStruct;
impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
}
```

`Event` and `Output` (the handler's argument and return) belong to dispatch, so `Bindings` omits them here. Accumulation reads triggers and ignores handlers.

## What bind provides

```rust
// the app implements this marker trait on MercuryStruct.
trait Bindings {
    type Trigger: Clone + Eq + Hash;
}

// #[derive(Bind)] generates this on every bindable node.
trait EventHandler<M: Bindings> {
    fn accumulate(&self, out: &mut HashSet<M::Trigger>) -> Result<(), BindError>;
}

// accumulation fails only on a duplicate trigger.
#[derive(Debug)]
enum BindError {
    DuplicateTrigger,   // raised when a trigger is bound twice on the active path
}

// insert returns false when the key is already present (std HashSet has no
// fallible insert), so this reports a duplicate as an error.
fn insert_or_error<T: Clone + Eq + Hash>(out: &mut HashSet<T>, t: T) -> Result<(), BindError> {
    if out.insert(t) { Ok(()) } else { Err(BindError::DuplicateTrigger) }
}

// the entry point allocates the set, runs the root's accumulate, and returns it.
fn accumulate<M, N>(root: &N) -> Result<HashSet<M::Trigger>, BindError>
where
    M: Bindings,
    N: EventHandler<M>,
{
    let mut out = HashSet::new();
    root.accumulate(&mut out)?;
    Ok(out)
}
```

The app calls `bind::accumulate::<MercuryStruct, _>(&root)` to get the `HashSet<MercuryTrigger>` of triggers to register.

## The attributes

```rust
#[derive(Laserbeam, Bind)]
#[laserbeam(path = NavPath, resolved = Resolved)]   // laserbeam reads these
#[binds(MercuryStruct)]                              // names the marker M
#[bind(Keyboard::new("g") => open_chrome)]          // binds a trigger to a handler
struct Nav {}
```

`#[binds(MercuryStruct)]` names the marker, so the derive emits `impl EventHandler<MercuryStruct> for Nav`. Because `M` is the concrete `MercuryStruct`, `M::Trigger` resolves to the concrete `MercuryTrigger`, `out` is `&mut HashSet<MercuryTrigger>`, and `.into()` lifts each trigger into `MercuryTrigger`.

`#[bind(trigger => handler, ..)]` lists the node's bindings in one attribute. Each `trigger` is any expression that lifts into `MercuryTrigger` via `Into`, and accumulation uses it. Each `handler` is recorded for dispatch and stays unused here.

## What is bindable

Accumulation recurses only laserbeam's descent edges.

- A struct's bindable children are its `#[resolve_into]` fields. Plain fields (`String`, counters, buffers) hold state; accumulation skips them, and they never implement `EventHandler`.
- An enum's bindable child is the active variant's inner type. Every variant's inner type implements `EventHandler<M>`, because it derives `Bind`.

So `EventHandler<M>` exists only on the node types that derive `Bind`, and accumulation walks only `#[resolve_into]` fields and variant inners.

## What the derive generates

For each node the derive emits one `accumulate`. It inserts the node's own binds with `insert_or_error(..)?`, then recurses into the node's bindable children and returns their result. Each node inserts its own binds before it recurses, so a child that rebinds an ancestor's trigger hits the already-present key and errors.

## Worked example

The state tree is a root struct holding the active layer, an enum of two layers, and two leaf layers, with binds at every level.

```rust
#[derive(Laserbeam, Bind)]
#[laserbeam_root(resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Keyboard::new("esc") => to_nav)]      // esc returns to nav from anywhere
struct Mercury {
    #[resolve_into] layer: Layer,            // accumulation descends into this field
}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = LayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Keyboard::new("f1") => show_help)]    // f1 fires in any layer
enum Layer {
    Nav(Nav),
    Typing(Typing),
}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = NavPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Keyboard::new("g") => open_chrome, Keyboard::new("j") => focus_down, Foreground::new("Slack") => on_slack)]
struct Nav {}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = TypingPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Keyboard::new("backspace") => delete_char)]
struct Typing {}
```

The derive generates these four impls.

```rust
impl EventHandler<MercuryStruct> for Mercury {
    fn accumulate(&self, out: &mut HashSet<MercuryTrigger>) -> Result<(), BindError> {
        insert_or_error(out, ::core::convert::Into::into(Keyboard::new("esc")))?;
        self.layer.accumulate(out)                            // recurses into the layer
    }
}

impl EventHandler<MercuryStruct> for Layer {
    fn accumulate(&self, out: &mut HashSet<MercuryTrigger>) -> Result<(), BindError> {
        insert_or_error(out, ::core::convert::Into::into(Keyboard::new("f1")))?;
        match self {                                          // recurses into the active variant
            Layer::Nav(inner)    => inner.accumulate(out),
            Layer::Typing(inner) => inner.accumulate(out),
        }
    }
}

impl EventHandler<MercuryStruct> for Nav {
    fn accumulate(&self, out: &mut HashSet<MercuryTrigger>) -> Result<(), BindError> {
        insert_or_error(out, ::core::convert::Into::into(Keyboard::new("g")))?;
        insert_or_error(out, ::core::convert::Into::into(Keyboard::new("j")))?;
        insert_or_error(out, ::core::convert::Into::into(Foreground::new("Slack")))?;
        Ok(())                                                // a leaf ends the recursion
    }
}

impl EventHandler<MercuryStruct> for Typing {
    fn accumulate(&self, out: &mut HashSet<MercuryTrigger>) -> Result<(), BindError> {
        insert_or_error(out, ::core::convert::Into::into(Keyboard::new("backspace")))?;
        Ok(())
    }
}
```

### State A: the nav layer

The state is `Mercury { layer: Layer::Nav(Nav {}) }`, and the app calls `bind::accumulate::<MercuryStruct, _>(&root)`.

1. The set starts empty.
2. `Mercury::accumulate` inserts `Keyboard("esc")`, giving `{esc}`, then recurses into `self.layer`.
3. `Layer::accumulate` inserts `Keyboard("f1")`, giving `{esc, f1}`, and the active variant `Nav` recurses.
4. `Nav::accumulate` inserts `Keyboard("g")`, `Keyboard("j")`, and `Foreground("Slack")`, giving `{esc, f1, g, j, Slack}`, then returns `Ok(())`.

The call returns:

```
Ok({ Keyboard("esc"), Keyboard("f1"), Keyboard("g"), Keyboard("j"), Foreground("Slack") })
```

### State B: the typing layer

The state is `Mercury { layer: Layer::Typing(Typing {}) }`.

1. The set starts empty.
2. `Mercury::accumulate` inserts `esc`, giving `{esc}`, then recurses.
3. `Layer::accumulate` inserts `f1`, giving `{esc, f1}`, and the active variant `Typing` recurses.
4. `Typing::accumulate` inserts `backspace`, giving `{esc, f1, backspace}`, then returns `Ok(())`.

The call returns:

```
Ok({ Keyboard("esc"), Keyboard("f1"), Keyboard("backspace") })
```

### The collision case

Suppose `Typing` instead binds `esc`.

```rust
#[bind(Keyboard::new("esc") => stop_typing)]
struct Typing {}
```

Accumulating state B now runs:

1. `Mercury::accumulate` inserts `esc`, giving `{esc}`, then recurses.
2. `Layer::accumulate` inserts `f1`, giving `{esc, f1}`, then recurses into `Typing`.
3. `Typing::accumulate` calls `out.insert(Keyboard("esc").into())`, which returns `false` because `Mercury` already inserted `esc`. `insert_or_error` returns `Err(BindError::DuplicateTrigger)`, and `?` propagates it out through `Layer::accumulate` and `Mercury::accumulate`.

The call returns:

```
Err(BindError::DuplicateTrigger)
```

`Typing` cannot rebind `esc` because `Mercury` bound it first. Children cannot clobber parents.
