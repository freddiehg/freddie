//! `AlwaysEqual`: wrap a value so a type containing it derives `PartialEq`/`Eq` under `testing`
//! while treating that value as matching.

/// Wraps a value that carries a resource rather than data.
///
/// Any two `AlwaysEqual` compare equal under `testing`, so a type containing one can derive
/// `PartialEq`/`Eq` there and stay assertable in tests.
#[derive(Debug)]
pub struct AlwaysEqual<T>(pub T);

#[cfg(feature = "testing")]
impl<T> PartialEq for AlwaysEqual<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(feature = "testing")]
impl<T> Eq for AlwaysEqual<T> {}
