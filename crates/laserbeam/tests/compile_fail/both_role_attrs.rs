use laserbeam::Laserbeam;

#[derive(Laserbeam)]
#[laserbeam_root(resolved = R)]
#[laserbeam(path = P, resolved = R)]
struct Both {
    value: u32,
}

fn main() {}
