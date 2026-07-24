# Invalidation: generated dispatch

Not done. Standalone. The design is the code the derive emits for this tree.

## The case

```rust
#[pre_post(Foo => (pre_foo, post_foo), Bar => (pre_bar, post_bar))]
#[bind(a => outer_handler)]
struct Outer {
    #[resolve_into]
    inner: Inner,
}

#[bind(a => inner_handler)]
struct Inner;
```

## Types

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

struct Valid<'n, N> {
    node: &'n mut N,
    handled: bool, // exclusive claimed at or below this field
}

enum Validity<'n, N> {
    Valid(Valid<'n, N>),
    Invalidated, // field is no longer an N after reshape applied on the way up
}

// pre_foo / pre_bar:
//   fn(&SourceEvent, Node<&mut OuterPath, ()>) -> (T, Vec<Effect>)
// post_foo / post_bar:
//   fn(T, Validity<'_, Inner>) -> Vec<Effect>
// outer_handler / inner_handler:
//   fn(&SourceEvent, Node<&mut Path, ()>) -> V  where V: Into<Vec<Effect>>
```

Exclusives and pres take `Node<&mut P, D>` (borrowed path). The framework keeps the path so every level can still `into_parent` after a deep exclusive. Deepest-wins is a threaded `claimed: &mut bool`, not path consumption and not a short-circuit past parents.

```rust
pub struct PathMut<N, P, F> {
    /* projection N, parent P */
    on_into_parent: F, // FnOnce(Validity<'_, N>) -> Vec<Effect>
}

fn no_post<N>(_: Validity<'_, N>) -> Vec<Effect> {
    Vec::new()
}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(Validity<'_, N>) -> Vec<Effect>,
{
    /// Apply any reshape scheduled for this child field, classify Validity, run the post once,
    /// push its effects, return the parent. Consuming self is the once-ness.
    pub fn into_parent(self, handled: bool, sink: &mut Vec<Effect>) -> P {
        let v = self.read_child(handled); // apply scheduled reshape, then classify
        Extend::extend(sink, (self.on_into_parent)(v));
        self.parent
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

`from_fn` is crate-private. Handlers return effects; only framework code holds `&mut Vec<Effect>`.

## Generated `Inner`

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        mut path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claimed: &mut bool,
    ) -> <Inner as Place>::Path<'a>
    where
        Self: 'a,
    {
        if !*claimed {
            if let Some(ev) = <&AEvent as TryFrom<_>>::try_from(event).ok() {
                if a.is_matching(ev) {
                    *claimed = true;
                    Extend::extend(
                        effs,
                        Into::<Vec<M::Effect>>::into(inner_handler(
                            ev,
                            ::bind::Node {
                                parent: &mut path,
                                data: (),
                            },
                        )),
                    );
                }
            }
        }
        path
    }
}
```

## Generated `Outer`

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
        // ----- down: each pre_post independently -----
        let opt_0 = match <&FooEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if Foo.is_matching(ev) => {
                let (t, now) = pre_foo(
                    ev,
                    ::bind::Node {
                        parent: &mut path,
                        data: (),
                    },
                );
                Extend::extend(effs, now);
                Some(t)
            }
            _ => None,
        };
        let opt_1 = match <&BarEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if Bar.is_matching(ev) => {
                let (t, now) = pre_bar(
                    ev,
                    ::bind::Node {
                        parent: &mut path,
                        data: (),
                    },
                );
                Extend::extend(effs, now);
                Some(t)
            }
            _ => None,
        };

        // ----- descend: one on_into_parent captures every opt_i -----
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |v| {
                let mut local = ::std::vec::Vec::new();
                match v {
                    Validity::Valid(mut valid) => {
                        if let Some(t) = opt_0 {
                            Extend::extend(
                                &mut local,
                                post_foo(
                                    t,
                                    Validity::Valid(Valid {
                                        node: &mut *valid.node,
                                        handled: valid.handled,
                                    }),
                                ),
                            );
                        }
                        if let Some(t) = opt_1 {
                            Extend::extend(
                                &mut local,
                                post_bar(
                                    t,
                                    Validity::Valid(Valid {
                                        node: &mut *valid.node,
                                        handled: valid.handled,
                                    }),
                                ),
                            );
                        }
                    }
                    Validity::Invalidated => {
                        if let Some(t) = opt_0 {
                            Extend::extend(&mut local, post_foo(t, Validity::Invalidated));
                        }
                        if let Some(t) = opt_1 {
                            Extend::extend(&mut local, post_bar(t, Validity::Invalidated));
                        }
                    }
                }
                local
            },
        );

        // ----- child -----
        let inner_path = <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, claimed);

        // ----- up: posts, then this level's exclusive -----
        let mut path = inner_path.into_parent(*claimed, effs);

        if !*claimed {
            if let Some(ev) = <&AEvent as TryFrom<_>>::try_from(event).ok() {
                if a.is_matching(ev) {
                    *claimed = true;
                    Extend::extend(
                        effs,
                        Into::<Vec<M::Effect>>::into(outer_handler(
                            ev,
                            ::bind::Node {
                                parent: &mut path,
                                data: (),
                            },
                        )),
                    );
                }
            }
        }
        path
    }
}
```

## Order of the walk

```text
enter Outer
  pre_foo?  pre_bar?                  // push now-effects, bind opt_i
  build inner_path                    // posts closed over opt_i
  enter Inner
    exclusive a => inner_handler?     // may set claimed, push effects
  leave Inner
  inner_path.into_parent(claimed)
    apply reshape of .inner if any
    Validity { handled: claimed }
    post_foo?  post_bar?              // iff opt_i was Some
  exclusive a => outer_handler?       // only if !claimed
leave Outer
```

### Event matches `a` only

```text
pre_foo skip, pre_bar skip
inner_handler runs, claimed = true
into_parent: Valid { handled: true }, post_foo skip, post_bar skip
outer_handler skip
```

Batch: `inner_handler` only. Outer posts still ran (no-op).

### Event matches Foo only

```text
pre_foo runs, opt_0 = Some(t)
inner exclusive skip
into_parent: Valid { handled: false }, post_foo(t, Valid), post_bar skip
outer exclusive skip
```

Batch: `pre_foo` now-effects, then `post_foo`.

### Event matches Foo and `a`

```text
pre_foo runs
inner_handler runs, claimed = true
into_parent: Valid { handled: true }, post_foo(t, Valid { handled: true })
outer_handler skip
```

Batch: `pre_foo` now-effects, `inner_handler`, `post_foo`.

### Reshape of `.inner` scheduled by the deep exclusive

```text
inner_handler claims and schedules reshape of Outer.inner
into_parent: apply reshape, Validity::Invalidated
post_foo(t, Invalidated) if Foo matched
```

Posts always run when their pre matched. They decide what `Invalidated` means.

## Rules

1. Several `pre_post`s on one node → several `opt_i`, one `on_into_parent` closure.
2. `pre` matched ⇒ `post` ran exactly once (`FnOnce` on the path, one ascent).
3. Posts run only inside `into_parent`. Claim does not skip them.
4. Deepest exclusive wins via `claimed`.
5. Exclusive and pre take `Node<&mut P, D>`. Path stays with the framework.
6. Reshape of a field is applied in that field's `into_parent` before `Validity` is built.

## Prefactor (no pre/post)

Behavior-identical exclusive dispatch:

- `Bindings::Effect` replaces `Output`
- `dispatch` threads `effs` and `claimed`
- exclusives take `Node<&mut P, D>`, set `claimed`, push effects
- `into_parent(self, _handled, _sink)` is a no-op project-up
- top-level returns `Some(effs)` when `claimed || !effs.is_empty()`, else `None`
- `V: Into<Vec<Effect>>`; expression handlers already work (`crates/bind/tests/expr_handler.rs`)

## Open

- How a deep exclusive schedules a reshape of an ancestor field (carrier on the descent; apply in the owner's `into_parent`). Until then, exclusives only mutate through the borrowed path they are given.
- Product nodes: one `Validity` per live child field, join before parent exclusives.
