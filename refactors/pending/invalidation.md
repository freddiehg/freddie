# Invalidation: descent schedules, ascent executes

Not done. Standalone. Pre, post, exclusive, and Validity are one machine. The generate for one tree is the design.

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

Two `pre_post` pairs on Outer, exclusive `a` on both levels. Deepest exclusive wins. Outer posts run on every ascent that entered Outer, including when Inner claimed `a`.

## Handlers

```rust
// pre:       fn(&Event, Node<&mut P, D>) -> (T, Vec<Effect>)
// post:      fn(T, Validity<'_, Child>) -> Vec<Effect>   // T = () if #[post] alone
// exclusive: fn(&Event, Node<&mut P, D>) -> V where V: Into<Vec<Effect>>
```

- `#[pre_post(trig => (pre, post))]` — pre down, post up with pre's `T`
- `#[pre(trig => pre)]` — post is `drop`
- `#[post(trig => post)]` — pre is the trigger check returning `()`
- `#[bind(trig => handler)]` — exclusive post: runs only if nothing deeper claimed

Descent freezes the handler set: matching pres run, push now-effects, bind `opt_i: Option<T>`. Pres do not reshape. Pre and exclusive take a borrowed path (`Node<&mut P, D>`) so the framework keeps the path for ascent.

## Validity

A post at a node guards a child field. `into_parent` applies any reshape scheduled for that field, then classifies:

```rust
struct Valid<'n, N> {
    node: &'n mut N,
    handled: bool, // exclusive claimed at or below this field
}
enum Validity<'n, N> {
    Valid(Valid<'n, N>),
    Invalidated,
}
```

```rust
fn only_if_valid<N>(
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Validity<'_, N>) -> Vec<Effect> {
    move |v| match v {
        Validity::Valid(valid) => f(valid.node),
        Validity::Invalidated => Vec::new(),
    }
}
```

## Path and batch

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub struct PathMut<N, P, F> {
    /* projection to N, parent P */
    on_into_parent: F, // FnOnce(Validity<'_, N>) -> Vec<Effect>
}

fn no_post<N>(_: Validity<'_, N>) -> Vec<Effect> {
    Vec::new()
}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(Validity<'_, N>) -> Vec<Effect>,
{
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
        handled: &mut bool,
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
    let mut handled = false;
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut handled);
    if handled || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

`from_fn` is crate-private. Handlers return effects; only framework code holds `&mut Vec<Effect>`. `pre` matched ⇒ `post` ran exactly once (`FnOnce` on the path, one ascent).

`&mut N` in `Valid` is a reborrow for the post call. `into_parent` owns the path and projects up after.

## Generated `Inner`

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        mut path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        handled: &mut bool,
    ) -> <Inner as Place>::Path<'a>
    where
        Self: 'a,
    {
        if let Some(ev) = <&AEvent as TryFrom<_>>::try_from(event).ok() {
            if a.is_matching(ev) && !*handled {
                *handled = true;
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
        handled: &mut bool,
    ) -> <Outer as Place>::Path<'a>
    where
        Self: 'a,
    {
        // down: pre_post Foo
        let opt_foo = match <&FooEvent as TryFrom<_>>::try_from(event).ok() {
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
        // down: pre_post Bar
        let opt_bar = match <&BarEvent as TryFrom<_>>::try_from(event).ok() {
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
        // down: exclusive trigger check only (body on the way up)
        let opt_a = match <&AEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if a.is_matching(ev) => Some(ev),
            _ => None,
        };

        // descend: Outer posts ride the child path
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |v| {
                let mut local = ::std::vec::Vec::new();
                match v {
                    Validity::Valid(mut valid) => {
                        if let Some(t) = opt_foo {
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
                        if let Some(t) = opt_bar {
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
                        if let Some(t) = opt_foo {
                            Extend::extend(&mut local, post_foo(t, Validity::Invalidated));
                        }
                        if let Some(t) = opt_bar {
                            Extend::extend(&mut local, post_bar(t, Validity::Invalidated));
                        }
                    }
                }
                local
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, handled);

        // up: apply reshape of .inner if any, run Outer posts, then Outer exclusive
        let mut path = inner_path.into_parent(*handled, effs);

        if let Some(ev) = opt_a {
            if !*handled {
                *handled = true;
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
        path
    }
}
```

## Walk

```text
enter Outer
  pre_foo? pre_bar?          // now-effects, bind opt_*
  opt_a = trigger check only
  build inner_path           // posts closed over opt_foo, opt_bar
  enter Inner
    exclusive inner_handler? // may set handled
  leave Inner
  into_parent(handled)       // reshape .inner?, Validity, post_foo?, post_bar?
  exclusive outer_handler?   // only if !handled
leave Outer
```

### `a` only

```text
inner_handler, handled = true
into_parent: Valid { handled: true }, posts no-op
outer_handler skip
```

Batch: `inner_handler` only.

### Foo only

```text
pre_foo, opt_foo = Some(t)
into_parent: Valid { handled: false }, post_foo(t, Valid)
```

Batch: pre_foo now-effects, post_foo.

### Foo and `a`

```text
pre_foo
inner_handler, handled = true
into_parent: Valid { handled: true }, post_foo(t, Valid { handled: true })
outer_handler skip
```

Batch: pre_foo now-effects, inner_handler, post_foo.

### Inner schedules reshape of `.inner`

```text
inner_handler claims + schedules
into_parent: apply → Invalidated
post_foo(t, Invalidated) if Foo matched
```

## Rearm

Post on the node that owns the `AndReturnHome` field:

```rust
#[post(AnyKey => only_if_valid(rearm))]
fn rearm(node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;
    vec![schedule]
}
```

`Valid`: rearm. `Invalidated` (left the set): skip; dropped guard cancels.

## Prefactor

- `Bindings::Effect` replaces `Output`
- `dispatch` threads `effs` and `handled`
- exclusives take `Node<&mut P, D>`, set `handled`, push effects, return path
- `into_parent(self, _handled, _sink)` projects up only
- top-level `Some` when `handled || !effs.is_empty()`
- `V: Into<Vec<Effect>>`; expression handlers already work (`crates/bind/tests/expr_handler.rs`)

## Open

How a deep exclusive schedules a reshape of a field it does not own, so the owner's `into_parent` applies it before that level's posts.

Product nodes: one `Validity` per live child field, join in `into_parent`.
