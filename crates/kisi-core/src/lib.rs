//! kisi-core: facade over the analysis crates.
//!
//! Front ends (CLI, WASM) depend on this crate only, so
//! the paths `kisi_core::rf`, `kisi_core::pi`, … stay stable while the
//! implementation lives in separately versioned crates:
//!
//! * `kisi-board` – board model and parser
//! * `kisi-si`    – signal integrity
//! * `kisi-pi`    – power integrity
//! * `kisi-sparse` – sparse solver (shared with the field-solver WASM module)

pub use kisi_board::{apply_project, board, geom, load_pcb, num, sexpr, view, Board};
pub use kisi_pi::{cavity_modal, losses, pdn, pdn_ic, pi, thermal, tree};
pub use kisi_si::{analytic, analyze, extract, eye, profile, rf, topology, via, via_pbm, xsgeom, Analysis, Params};

pub mod commands;
pub mod config;
pub mod gaps;
pub mod service;
pub mod variant;
