# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds will run. That set is final. Ascent runs every scheduled post leaf to root; mutation is free; each post is handed a `Context` — structure of the child field plus whether an exclusive already claimed this event (a small grab bag, not a pure "validity" flag). `#[bind]` is a post that reads the claim bit on that context.

## Paths: shared down, owned up

The path type is the normal owned `PathMut` (same as today). What changes is when the handler may hold it.

- **Descent (pre):** the framework still owns the path for the walk. Pre gets a shared borrow: `Node<&P, D>`. Read-only; no reshape.
- **Ascent (post / bind):** `into_parent` has recovered this level's path as an owned `P` again (built the usual way: `from_fn` on the way down, `into_parent` on the way up). Post and bind receive that owned path: `Node<P, D>` — same shape as today's exclusive handlers. `get_mut`, `ascend_mut`, the usual tools.

There is no parallel "`&mut Path`" API for posts. Mutation is allowed because ownership of the normal path is back at this level on the way up, not because the signature is a mutable reference.

To thread ownership through several posts at one level, each post/bind returns the path with its effects. The value carried from pre to post is whatever pre returns — a concrete type, inferred, not a type parameter of the framework.

## Developer experience

```rust
#[derive(Bind)]
#[node(parent = RootPath)]
#[binds(M)]
#[pre_post(Foo => (pre_foo, post_foo), Bar => (pre_bar, post_bar))]
#[pre(Baz => track)]                 // post is drop
#[post(Qux => guard)]                // pre is trigger → ()
#[bind(KeyA => outer_handler)]       // one function; exclusive
struct Outer {
    #[resolve_into]
    inner: Inner,
}

#[derive(Bind)]
#[node(parent = OuterPath)]
#[binds(M)]
#[bind(KeyA => inner_handler)]
struct Inner;
```

```rust
// pre: shared path. Return type is ordinary and concrete (here u32); the
// framework stores it in Option<_> inferred from this function.
fn pre_foo(ev: &FooEvent, node: Node<&OuterPath, ()>) -> u32 {
    node.parent.get().hits
}

// post: owned path in, path out; first arg is pre_foo's return (u32);
// Context carries structure + whether an exclusive already claimed.
fn post_foo(
    hits_before: u32,
    node: Node<OuterPath, ()>,
    ctx: Context,
) -> (Vec<M::Effect>, OuterPath) {
    match ctx.structure() {
        Structure::Valid => {
            // node.parent.get_mut().inner is still Inner
            let _ = (hits_before, ctx.claimed());
            (vec![], node.parent)
        }
        Structure::Invalidated => (vec![], node.parent),
    }
}

// pre_bar / post_bar likewise — whatever pre_bar returns is what post_bar receives.
// Often that is () because the pair only needs timing, not carriage.

// bind body: ordinary post shape. exclusive() only gates on ctx.claimed().
fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    v: Context,
) -> (Vec<M::Effect>, OuterPath) { ... }

fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    v: Context,
) -> (Vec<M::Effect>, InnerPath) { ... }

// sugar: act only when the child survived (no pre carriage)
fn rearm(child: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}
// #[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
```

Expression positions work as today (`#handler(…)` splice). Pinned by `crates/bind/tests/expr_handler.rs`.

## Semantics

### Schedule on the way down (final)

For each pre/post/bind on the node, if the trigger matches:

- `#[pre_post]` / `#[pre]`: call pre with `Node<&P, D>`, store `opt_i = Some(pre_return)`
- `#[post]`: store `opt_i = Some(())`
- `#[bind]`: store `opt_i = Some(())` (same as `#[post]`; event stays the dispatch `&Event`)

If the trigger misses, `opt_i = None`. Ascent never re-checks triggers and never changes which opts are `Some`.

```rust
// shape only — return type is whatever the pre function returns (concrete, inferred)
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> /* concrete type */
```

No reshape on the descent. Whether pre may also return now-effects is open; generate is return-value only (no effect batch from pre).

### Execute on the way up (all scheduled posts run)

Leaf to root. The ascent threads one monotone `claimed: bool` (false at the leaf, set true after an exclusive post runs). At each level the framework holds an owned path again.

1. Apply any reshape scheduled for this level's child field.
2. Build `Context` from the field's structure and the ascent's `claimed` bit.
3. For each scheduled post, call it with that `Context` and the owned path; take path (and optional claim) back from the return.
4. If the post was exclusive and reported a claim, set `claimed = true` for everything shallower.

A scheduled post always runs (is called). What it does with `ctx.claimed()` is its business. Structure `Invalidated` is how it learns it lost the child.

```rust
#[derive(Clone, Copy)]
enum Structure {
    /// Child field is still the scheduled type. Projections through the path are sound.
    Valid,
    /// Child field was replaced.
    Invalidated,
}

#[derive(Clone, Copy)]
struct Context {
    structure: Structure,
    claimed: bool,
}

impl Context {
    fn structure(self) -> Structure { self.structure }
    /// An exclusive post deeper in the tree already took this event.
    fn claimed(self) -> bool { self.claimed }
    fn valid(self) -> bool { matches!(self.structure, Structure::Valid) }
}
```

Access is flat on `Context`: `ctx.claimed()`, `ctx.structure()`, `ctx.valid()`. No nested `ctx.something.claimed()` — the bag is small enough that methods on the root are the API. Fields stay private so we can grow the bag later without breaking call sites.

`claimed()` is a snapshot at the moment this post runs. It is not "any pre/post below matched" — only exclusive posts raise it. A logging `AnyKey` pre/post never sets it.

### How `claimed` moves

```text
framework holds:  claimed: bool   (monotone, starts false)

each post call:
  ctx = Context { structure, claimed }   // snapshot
  out = post(..., ctx)
  path = out.path
  if out.claim { claimed = true }        // only exclusive sugar sets claim: true
```

Posts do not receive `&mut bool`. They read `ctx.claimed()`. The only write path is the post return's `claim` flag, which exclusive sugar sets when it actually ran the body.

```rust
struct PostOut<P> {
    effects: Vec<Effect>,
    path: P,
    /// True if this post claims the event for exclusive deepest-wins.
    claim: bool,
}
```

Plain posts always return `claim: false`. `exclusive(h)` returns `claim: true` only when it invoked `h`.

### Defaults

- `#[pre_post(trig => (pre, post))]` — user pre returns a value, user post receives it
- `#[pre(trig => pre)]` — user pre returns a value, post is `drop` of that value
- `#[post(trig => post)]` — pre is only the trigger check → `()`, user post receives `()`
- `#[bind(trig => handler)]` — a `#[post]` whose body is `exclusive(handler)`

Several pairs on one node: several independent `opt_i` (each its own concrete payload type), one `on_into_parent` closure.

### `#[bind]` is a post with no pre

There is no third handler kind and no secret `!*claimed` outside `Context`.

```rust
#[bind(KeyA => outer_handler)]
// desugars to:
#[post(KeyA => exclusive(outer_handler))]
```

```rust
// user body — same signature as any post that needs the event:
fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    ctx: Context,
) -> (Vec<M::Effect>, OuterPath);

// thin sugar — the only place that reads ctx.claimed() for deepest-wins:
fn exclusive<E, P>(
    h: impl FnOnce(&E, Node<P, ()>, Context) -> (Vec<Effect>, P),
) -> impl FnOnce(&E, Node<P, ()>, Context) -> PostOut<P> {
    move |ev, node, ctx| {
        if ctx.claimed() {
            PostOut {
                effects: Vec::new(),
                path: node.parent,
                claim: false, // already claimed deeper; do not re-claim
            }
        } else {
            let (effects, path) = h(ev, node, ctx);
            PostOut {
                effects,
                path,
                claim: true, // we ran; shallower exclusives must see claimed
            }
        }
    }
}
```

Framework when running any scheduled post:

```rust
let ctx = Context {
    structure, // Valid | Invalidated for this field
    claimed,   // ascent bit so far
};
let out = post_fn(ev, Node { parent: path, data: () }, ctx);
Extend::extend(effs, out.effects);
path = out.path;
if out.claim {
    claimed = true;
}
```

A plain `#[post]` is wrapped to `PostOut { claim: false, ... }`. A `#[bind]` uses `exclusive`. pre_post posts at the same level are not exclusive: they all still run (before the bind at that level) and always see the same `ctx` snapshot unless an earlier post at this level claimed — order is pre_post posts first, then bind, so bind sees claims from below only, not from sibling pre_posts (sibling pre_posts do not claim).

### `only_if_valid`

```rust
fn only_if_valid<P, N>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Node<P, ()>, Context) -> PostOut<P> {
    move |mut node, ctx| {
        let effects = if ctx.valid() {
            f(project(&mut node.parent))
        } else {
            Vec::new()
        };
        PostOut {
            effects,
            path: node.parent,
            claim: false,
        }
    }
}
```

## Types

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

#[derive(Clone, Copy)]
pub enum Structure {
    Valid,
    Invalidated,
}

#[derive(Clone, Copy)]
pub struct Context {
    structure: Structure,
    claimed: bool,
}

impl Context {
    pub fn structure(self) -> Structure { self.structure }
    pub fn claimed(self) -> bool { self.claimed }
    pub fn valid(self) -> bool { matches!(self.structure, Structure::Valid) }
}

pub struct PostOut<P> {
    pub effects: Vec<Effect>,
    pub path: P,
    pub claim: bool,
}

pub struct PathMut<N, P, F> {
    // owned parent P + projections to N (as today)
    // F also receives claimed so it can build Context for each post
    on_into_parent: F, // FnOnce(P, Structure, bool /*claimed*/) -> PostOut<P>
}

fn no_post<P>(parent: P, _: Structure, _: bool) -> PostOut<P> {
    PostOut {
        effects: Vec::new(),
        path: parent,
        claim: false,
    }
}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(P, Structure, bool) -> PostOut<P>,
{
    /// Apply reshape of the child field if any, run posts, return parent + claim.
    pub fn into_parent(self, sink: &mut Vec<Effect>, claimed: &mut bool) -> P {
        let structure = self.classify_after_reshape();
        let out = (self.on_into_parent)(self.parent, structure, *claimed);
        Extend::extend(sink, out.effects);
        if out.claim {
            *claimed = true;
        }
        out.path
    }
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claimed: &mut bool,
    ) -> Self::Path<'a>
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<M::Effect>>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    let mut effs = Vec::new();
    let mut claimed = false;
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claimed);
    if claimed || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

Handlers return effects via `PostOut`; only framework code holds the batch sink. `from_fn` is crate-private. The ascent's `claimed` bit is the same value snapshotted into every `Context`; posts never see a bare `&mut bool`.

## Generated code

### Helper the generate uses for every post

```rust
fn run_post<P>(
    claimed: &mut bool,
    structure: Structure,
    path: P,
    body: impl FnOnce(Node<P, ()>, Context) -> PostOut<P>,
) -> (P, Vec<Effect>) {
    let ctx = Context {
        structure,
        claimed: *claimed,
    };
    let out = body(
        Node {
            parent: path,
            data: (),
        },
        ctx,
    );
    if out.claim {
        *claimed = true;
    }
    (out.path, out.effects)
}
```

### Inner (leaf, one bind)

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claimed: &mut bool,
    ) -> <Inner as Place>::Path<'a>
    where
        Self: 'a,
    {
        // schedule: Option<()> if KeyA matches (same as any #[post])
        let opt_a = /* trigger check → Some(()) or None */;

        if let ::core::option::Option::Some(()) = opt_a {
            let ev = /* &KeyEvent from event */;
            // exclusive(inner_handler) is the post body
            let (path, out_effs) = run_post(
                claimed,
                Structure::Valid, // leaf: no child field to invalidate
                path,
                |node, v| exclusive(inner_handler)(ev, node, v),
            );
            ::core::iter::Extend::extend(effs, out_effs);
            return path;
        }
        path
    }
}
```

### Outer (two pre_posts + one bind)

```rust
impl Dispatch<M> for Outer {
    fn dispatch<'a>(
        mut path: <Outer as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claimed: &mut bool,
    ) -> <Outer as Place>::Path<'a>
    where
        Self: 'a,
    {
        // ----- descent: schedule -----
        let opt_foo = /* Foo match → Some(pre_foo(...)) */;
        let opt_bar = /* Bar match → Some(pre_bar(...)) */;
        let opt_a = /* KeyA match → Some(()) */;

        // into_parent will pass (structure, claimed) into this closure.
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, structure, claimed_now| {
                let mut claimed = claimed_now;
                let mut local = ::std::vec::Vec::new();
                let mut path = parent;
                if let ::core::option::Option::Some(t) = opt_foo {
                    let (p, e) = run_post(&mut claimed, structure, path, |node, v| {
                        let (effects, path) = post_foo(t, node, v);
                        PostOut {
                            effects,
                            path,
                            claim: false,
                        }
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                if let ::core::option::Option::Some(t) = opt_bar {
                    let (p, e) = run_post(&mut claimed, structure, path, |node, v| {
                        let (effects, path) = post_bar(t, node, v);
                        PostOut {
                            effects,
                            path,
                            claim: false,
                        }
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                PostOut {
                    effects: local,
                    path,
                    claim: claimed, // may have been raised by an exclusive among these posts
                }
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, claimed);

        // ----- ascent: reshape + pre_post posts (Context built inside run_post) -----
        let mut path = inner_path.into_parent(effs, claimed);

        // ----- bind: exclusive post, same run_post path -----
        if opt_a.is_some() {
            let structure = path.structure_of_inner(); // Valid | Invalidated after reshape
            let ev = /* &KeyEvent */;
            let (p, e) = run_post(claimed, structure, path, |node, v| {
                exclusive(outer_handler)(ev, node, v)
            });
            path = p;
            ::core::iter::Extend::extend(effs, e);
        }
        path
    }
}
```

Every post goes through `run_post`. That is where `Context` is built (`structure` + current `claimed`) and where `out.claim` updates the ascent bit. `exclusive` only reads `ctx.claimed()` and sets `PostOut.claim`; it does not touch a parallel bool.

### `#[pre]` / `#[post]` alone

```rust
// #[pre(Baz => track)]
if let Some(t) = opt_baz {
    drop(t);
}

// #[post(Qux => guard)] — plain post, claim: false
if let Some(()) = opt_qux {
    let (p, e) = run_post(claimed, structure, path, |node, v| {
        let (effects, path) = guard((), node, v);
        PostOut {
            effects,
            path,
            claim: false,
        }
    });
    path = p;
    // extend local/effs with e
}
```

## Walk

```text
DESCENT
  Outer owns path
  pre_foo? pre_bar? with &path → opt_*
  opt_a? (bind scheduled as Option<()>)
  move path into inner_path

ASCENT  claimed starts false
  Inner bind scheduled:
    ctx = Context { structure: Valid, claimed: false }
    exclusive(inner_handler) sees !ctx.claimed(), runs body, PostOut.claim = true
    claimed = true
  Outer into_parent:
    structure = Valid | Invalidated after reshape
    post_foo gets Context { structure, claimed: true }   // ctx.claimed() == true
    post_bar same
  Outer bind scheduled:
    ctx = Context { structure, claimed: true }
    exclusive(outer_handler) sees ctx.claimed(), skips body, claim: false
```

### `KeyA` only

```text
Inner exclusive runs, claim true
Outer bind sees ctx.claimed(), skips
```

### `Foo` only

```text
opt_foo = Some(hits_before)
Inner: no bind
Outer post_foo(hits_before, path, ctx) with !ctx.claimed() && ctx.valid()
Outer bind not scheduled
```

### `Foo` and `KeyA`

```text
Inner exclusive may reshape, claim true
Outer post_foo still runs — ctx.claimed() == true
Outer exclusive skips
```

### Logging `AnyKey` pre_post also present

```text
also schedules, also runs with claim: false on its PostOut
does not raise claimed
does not change structure unless it mutates the child
```

## Rearm

```rust
#[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
fn rearm(node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;
    vec![schedule]
}
```

`structure: Valid`: rearm. `Invalidated`: skip; `Drop` of the guard cancels the timer. Does not care about `claimed`.

## Prefactor

Shippable alone, before pre/post attributes exist.

Before: `type Output`, `ControlFlow<Output, Path>`, exclusive takes path by value and `Break`s.

After:

- `type Effect`
- `dispatch` threads `effs` and `claimed`, always returns `Path`
- exclusive is `#[post(exclusive(h))]`; claim flows through `Context` / `PostOut`
- `into_parent(self, sink, claimed)` exists; no post yet in the prefactor
- top-level `Some` when `claimed || !effs.is_empty()`
- `V: Into<Vec<Effect>>`; expression handlers already work

Mercury binds that today `ascend_mut` + `set_layer` wait on the reshape carrier (Open): under full ascent they should schedule the layer replace for the owner's `into_parent`, then return the path, rather than consuming the path mid-walk. Prefactor may keep today's by-value exclusive + `Break` for a behavior-identical cut; the feature wants full ascent + path returned.

## Rules

1. Descent schedules; that set is final.
2. Ascent runs every scheduled post; mutation does not cancel them.
3. Pre: shared path. Post: owned path in/out via `PostOut`.
4. Every post receives `Context`; reads are `ctx.claimed()`, `ctx.structure()`, `ctx.valid()` (fields private).
5. Ascent holds a monotone `claimed` bit; each post sees a snapshot via `ctx.claimed()`; exclusive sugar sets `PostOut.claim` when it runs the body; framework ORs that into the bit.
6. Logging never sets `claim`. Only `exclusive(...)` does.
7. no pre → `()`. no post → `drop`.
8. `#[bind]` = `#[post(exclusive(handler))]` — thin sugar over `ctx.claimed()` / `PostOut.claim`.
9. Reshape of a field applied in that field's `into_parent` before posts at that level.

## Tests

- scheduled post runs after deep bind, including `Invalidated`
- logging `AnyKey` pre_post does not set `claimed`
- deepest bind wins; parent bind runs when leaf misses
- path is returned through two posts at one level
- drop counter on a pre return value: consumed once by post (or drop)
- pre miss: no post
- `#[pre]` alone: drop; `#[post]` alone: receives `()`
- expression post: `only_if_valid(project, rearm)`

## Open

- Whether `pre` may also push now-effects on the way down.
- How a deep bind schedules a reshape of a field it does not own (carrier applied in owner's `into_parent`). Until then, binds that `ascend_mut` + `set_layer` and path return do not compose cleanly.
- Sugar so user posts can write `-> Vec<Effect>` while the derive still threads path.
- Product nodes: one Context (structure bit) per live child field.
- Fallbacks that must not run if something claimed the event (e.g. root `AnyKey` passthrough). Closer to `claimed` than to `Context`. Deferred.
- Third structure state: deferred.
