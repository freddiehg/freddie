use rayban::Rayban;

#[derive(Rayban)]
#[rayban(path = P, resolved = R)]
#[rayban(path = P2, resolved = R)]
struct Twice {
    value: u32,
}

fn main() {}
