use laserbeam::Laserbeam;

#[derive(Laserbeam)]
#[laserbeam(path = P, resolved = R)]
enum BadVariant {
    Unit,
}

fn main() {}
