use laserbeam::Laserbeam;

#[derive(Laserbeam)]
#[laserbeam(path = P, resolved = R)]
#[laserbeam(path = P2, resolved = R)]
struct Twice {
    value: u32,
}

fn main() {}
