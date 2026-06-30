# laserbeam_macro

The `#[derive(Laserbeam)]` derive behind the `laserbeam` crate. You almost certainly
want `laserbeam` itself, which re-exports this.

The derive implements `laserbeam`'s `Resolve` and `Projection` traits on your state
types, so you can resolve a mutable path to the active leaf of a tree. See the
`laserbeam` crate for what those traits are and how to use them.

## License

MIT or Apache-2.0, at your option.
