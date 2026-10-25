//! kisi – native command line (`kisi <command> <board.kicad_pcb> [options]`).
//!
//! The commands, their arguments and JSON results live in `kisi_core::commands`
//! and are the same as in the Node CLI `kisi.mjs`; this binary has no field solver
//! (formula impedances), which makes it useful for CI and development.

mod agent;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{}", agent::HELP);
        std::process::exit(2);
    }
    let code = agent::main(&args);
    if std::env::var("KISI_CAVITY_STATS").is_ok() {
        let (n, t, cells) = kisi_core::via_cavity::lu_stats();
        let (g, e) = kisi_core::via_cavity::group_stats();
        eprintln!("cavity links {:.2} s", kisi_core::via::CAV_TIME.with(|c| c.get()));
        let (na, ta) = kisi_core::via_pbm::ABCD_TIME.with(|c| c.get());
        eprintln!("cavity vias: {na} evaluations, {ta:.2} s");
        eprintln!(
            "cavity grids: {n} factorisations, {t:.2} s, {} cells on average; {g} shared groups of {e} environments",
            if n > 0 { cells / n } else { 0 }
        );
    }
    std::process::exit(code);
}
