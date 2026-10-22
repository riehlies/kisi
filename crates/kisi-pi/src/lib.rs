//! kisi-pi: power integrity.
//!
//! * `pi`   – DC IR drop on a rasterised multi-layer copper network, rails from fields
//! * `tree` – regulator chains and power balance
//! * `pdn`  – AC impedance of a rail (lumped model and plane grid model)
//! * `pdn_ic` – PDN seen from every IC: AC grid over all layers, capacitor loop inductances, targets, recommendations
//! * `losses` – ground return, losses per part / rail / layer, loss map, power flow

pub use kisi_board::{board, geom, load_pcb, num};

pub mod cavity_modal;
pub mod pdn;
pub mod pdn_ic;
pub mod losses;
pub mod pi;
pub mod tree;
