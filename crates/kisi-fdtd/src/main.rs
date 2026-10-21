//! kisi-fdtd <case.json> [--mesh-only] [--mesh mesh.json]: JSON result on stdout.
use std::io::Write;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = a.get(1).expect("case.json");
    let case: kisi_fdtd::Case = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mesh = a.iter().position(|x| x == "--mesh").map(|i| serde_json::from_str::<kisi_fdtd::Mesh>(&std::fs::read_to_string(&a[i + 1]).unwrap()).unwrap());
    if a.iter().any(|x| x == "--mesh-only") {
        let m = mesh.unwrap_or_else(|| kisi_fdtd::make_mesh(&case));
        println!("{}", serde_json::to_string(&m).unwrap());
        return;
    }
    let r = kisi_fdtd::simulate(&case, mesh, &mut |ex, s, db| {
        if s % 2000 == 0 {
            eprint!("\rport {ex}: step {s} energy {db:.1} dB   ");
            let _ = std::io::stderr().flush();
        }
    });
    eprintln!();
    println!("{}", serde_json::to_string(&r).unwrap());
}
