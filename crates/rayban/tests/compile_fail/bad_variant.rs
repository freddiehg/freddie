use rayban::Rayban;

#[derive(Rayban)]
#[rayban(path = P, resolved = R)]
enum BadVariant {
    Unit,
}

fn main() {}
