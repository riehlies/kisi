//! kisi-pi: power integrity.
//!
//! * `pi`   – DC IR drop on a rasterised multi-layer copper network, rails from fields
//! * `tree` – regulator chains and power balance
//! * `pdn`  – AC impedance of a rail (lumped model and plane grid model)
//! * `pdn_ic` – PDN seen from every IC: AC grid over all layers, capacitor loop inductances, targets, recommendations
//! * `losses` – ground return, losses per part / rail / layer, loss map, power flow
//! * `field_syntax` – net names in the power fields (quoting of names with spaces)

pub use kisi_board::{board, geom, load_pcb, num, package};

pub mod cavity_modal;
pub mod field_syntax;
pub mod pdn;
pub mod pdn_ic;
pub mod losses;
pub mod pi;
pub mod tree;
