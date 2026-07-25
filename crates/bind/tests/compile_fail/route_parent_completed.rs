// A path whose parent is a route enum has no `Above` on that parent, so
// `Completed<TitlePath>` does not compile.
use laserbeam::{Completed, PathMut};

struct Title;

type MediaPath<'a> = &'a mut Media;
struct Media;
type AlbumPath<'a> = PathMut<Album, MediaPath<'a>>;
struct Album;
enum TitleParent<'a> {
    Album(AlbumPath<'a>),
}
type TitlePath<'a> = PathMut<Title, TitleParent<'a>>;

fn g(_a: Completed<TitlePath<'_>>) {}

fn main() {}
