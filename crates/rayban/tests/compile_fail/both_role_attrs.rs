use rayban::Rayban;

#[derive(Rayban)]
#[rayban_root(resolved = R)]
#[rayban(path = P, resolved = R)]
struct Both {
    value: u32,
}

fn main() {}
