//! kisi-fdtd <case.json> [--mesh-only] [--mesh mesh.json] [--volume out.bin]: JSON result on stdout.
//! `--volume` (research prototype): one excitation of the case's `volume_port` (default 0) with the
//! volume field output of its `volume` key (`VolumeSpec`), written as a binary blob.
use std::io::Write;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = a.get(1).expect("case.json");
    let text = std::fs::read_to_string(path).unwrap();
    let case: kisi_fdtd::Case = serde_json::from_str(&text).unwrap();
    let mesh = a
        .iter()
        .position(|x| x == "--mesh")
        .map(|i| serde_json::from_str::<kisi_fdtd::Mesh>(&std::fs::read_to_string(&a[i + 1]).unwrap()).unwrap());
    if a.iter().any(|x| x == "--mesh-only") {
        let m = mesh.unwrap_or_else(|| kisi_fdtd::make_mesh(&case));
        println!("{}", serde_json::to_string(&m).unwrap());
        return;
    }
    let mut progress = |ex: usize, s: usize, db: f64| {
        if s.is_multiple_of(2000) {
            eprint!("\rport {ex}: step {s} energy {db:.1} dB   ");
            let _ = std::io::stderr().flush();
        }
    };
    if let Some(i) = a.iter().position(|x| x == "--volume") {
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let spec: kisi_fdtd::VolumeSpec = serde_json::from_value(v["volume"].clone()).expect("case key `volume`");
        let port = v["volume_port"].as_u64().unwrap_or(0) as usize;
        let (r, vf) = kisi_fdtd::simulate_volume(&case, mesh, port, &spec, &mut progress);
        eprintln!();
        std::fs::write(&a[i + 1], vf.to_bytes(kisi_fdtd::volume_header(&r))).unwrap();
        println!("{}", serde_json::to_string(&kisi_fdtd::volume_header(&r)).unwrap());
        return;
    }
    let r = kisi_fdtd::simulate(&case, mesh, &mut progress);
    eprintln!();
    println!("{}", serde_json::to_string(&r).unwrap());
}
