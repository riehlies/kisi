//! kisi-pi: power integrity.
//!
//! * `pi`   – DC IR drop on a rasterised multi-layer copper network, rails from fields / KiPIDA
//! * `tree` – regulator chains and power balance
//! * `pdn`  – AC impedance of a rail (lumped model and plane grid model)

pub use kisi_board::{board, geom, load_pcb, num};

pub mod pdn;
pub mod pi;
pub mod tree;
