# Rayban — implementation plan

Separate crates inside the phantom-kit-2 repo, depending on nothing else in it. Mutable counterpart to isograph's `resolve_position`. See `design.md` for the spec.

## Open questions (resolve before implementing)

1. Trait for `resolve`: minimal trait (no name magic, like isograph) vs no trait (accept `FooParent` magic). Recommended: trait. The path type stays trait-free either way.
2. Names: crates `rayban` + `rayban_macro`; types `Rayban<Node, Parent>`, `Root<'a, T>`; trait `Resolve`; derive `#[derive(Resolve)]`; markers `#[resolve(..)]`, `#[resolve_into]`. All TBD; avoid `Phantom` since these crates depend on nothing in phantom-kit.
3. Publish on every commit: CI auto-bumps patch; macro crate publishes before lib; needs `CARGO_REGISTRY_TOKEN` secret. Confirm scheme.
4. v1 scope: macro supports struct-field (lens) and enum-variant (prism) resolution. `Vec`/indexed `#[resolve_into]` (discriminator-driven) deferred; runtime box still supports hand-written indexed projections. Confirm defer.
5. The parent-enum variant-name convention (variant named after the parent type), as in isograph. Confirm acceptable.

## Where it lives

In the phantom-kit-2 cargo workspace, as two standalone crates that depend on nothing else in the repo:

- `rayban` (lib): runtime types `Root`, `Rayban`, the `Resolve` trait. Re-exports the derive (`pub use rayban_macro::Resolve`). No deps beyond std.
- `rayban_macro` (proc-macro): `#[derive(Resolve)]` and attributes. Deps: `syn`, `quote`, `proc-macro2`.

They are added to the workspace `members`, published to crates.io independently, and carry their own version. Dual MIT/Apache license. `#![forbid(unsafe_code)]` in `rayban`.

- Integration tests in `rayban/tests/` (need the macro + runtime together).
- `trybuild` as a dev-dep of `rayban` for compile-fail tests.

## rayban (runtime)

Exactly the types in `design.md`:

- `Root<'a, T>`: private `node`, `new`, `get_mut`, `into_parent`.
- `Rayban<Node, Parent>`: private `parent`, public `new`, `get_mut`, `into_parent`. Box field `Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>` (implicitly `'static`; projections capture only owned data).
- `Resolve<'a>` trait: assoc `Parent`, `Resolved`, method `resolve`.
- Crate-level rustdoc mirroring isograph's `resolve_position` module doc: model, example tree, a worked `Resolved` value, and the private-`parent` rationale.

## rayban_macro

Parses, per derived type: the item and its generics/lifetimes; `#[resolve(resolved = .., is_root)]` or `#[resolve(parent = .., resolved = ..)]`; `#[resolve_into]` on at most one struct field (error if more).

Generates:

- The projection box(es): enum variant -> prism `move |p| match p.get_mut() { Variant(x) => x, _ => unreachable!() }`; struct field -> lens `move |p| &mut p.get_mut().field`.
- The `Resolve` impl: `type Parent`, `type Resolved`, `resolve`.
- `resolve` body: match-then-build descent. Enum: match the active variant (no binding), wrap the current cursor as `<Child as Resolve>::Parent::<SelfName>(..)`, tail-call the child's `resolve`. Leaf struct: return `Resolved::<SelfName>(self_cursor)`.

It does NOT emit `FooPath`/`FooParent` aliases.

### Prototype first (the risky part)

Before templating the macro, hand-write the exact code it should emit for a representative tree — root enum, an intermediate struct with shared state + a `#[resolve_into]` sub-enum, a multi-parent leaf — and compile + run it. The pieces (path, `get_mut`, `into_parent`, descend walk, multi-parent) are validated in isolation; this validates the full generated shape end to end. Then write the `syn`/`quote` codegen to match it.

## Tests

`rayban/tests/`:

- `get_mut` / `into_parent` over a 3-level chain.
- Multi-parent leaf: resolve via each route; `into_parent` returns the right parent enum.
- Shared state on an intermediate, read and mutated from a leaf via `into_parent`.
- Indexed leaf via a hand-written box (`vec[selected]`) to lock that the runtime supports it.
- `resolve()` lands on the correct `Resolved` variant for several configured states.
- Round-trip: resolve, mutate, re-resolve, assert.

`trybuild` compile-fail:

- Holding the leaf and an ancestor `&mut` at once -> E0499.
- External `.parent` access -> private-field error.
- `into_parent` then `get_mut` on the moved cursor -> use-after-move.
- Two `#[resolve_into]` on one struct -> macro error.

## Private parent

`Rayban.parent` and `Root.node` are private; construction via `new`; the only way up is `into_parent` (consuming). A `trybuild` test asserts external `.parent` access fails to compile. This makes staleness a move error, not a runtime panic.

## CI

Copy isograph's GitHub Actions: `fmt --check`, `clippy -D warnings`, `build`, `test` (including `trybuild`) on stable.

Plus publish to crates.io on every commit to the default branch:

- Bump patch of both crates (scheme TBD, Q3).
- `cargo publish` `rayban_macro` first, then `rayban`.
- Gated on tests; uses `CARGO_REGISTRY_TOKEN`.

## Docs

- Crate-level rustdoc on `rayban` modeled on isograph's `resolve_position` lib.rs doc.
- `README.md` with the single end-to-end example.
- `design.md` (this folder) is the canonical spec.

## Naming

TBD; working set above. Decide before the first publish (crate names are permanent on crates.io).
