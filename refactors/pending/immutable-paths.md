# immutable paths in laserbeam

Placeholder. Not designed.

laserbeam has one path type, `PathMut`, a mutable cursor (`get_mut`). Add an isograph-style
immutable path alongside it, `Path`, a declarative read into the tree.

And a way to convert a `PathMut` into a `Path`, so a mutable cursor can be downgraded to a
read.
