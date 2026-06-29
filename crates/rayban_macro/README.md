# rayban_macro

The `#[derive(Rayban)]` derive behind the `rayban` crate. You almost certainly
want `rayban` itself, which re-exports this.

The derive implements `rayban`'s `Resolve` and `Projection` traits on your state
types, so you can resolve a mutable path to the active leaf of a tree. See the
`rayban` crate for what those traits are and how to use them.

## License

MIT or Apache-2.0, at your option.
