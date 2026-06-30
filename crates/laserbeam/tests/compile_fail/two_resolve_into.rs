use laserbeam::Laserbeam;

#[derive(Laserbeam)]
#[laserbeam(path = P, resolved = R)]
struct TwoFields {
    #[resolve_into]
    a: u32,
    #[resolve_into]
    b: u32,
}

fn main() {}
