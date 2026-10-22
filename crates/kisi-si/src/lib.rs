//! kisi-si: signal integrity.
//!
//! * `extract`  – cross-sections along every trace (references, coplanar ground, pairs)
//! * `analytic` – closed-form impedance formulas
//! * `xsgeom`   – cross-section geometry text for the 2D field solver
//! * `rf`       – RLGC line models, channel S-parameters/TDR, Touchstone, crosstalk
//! * `via`      – via model (antipads, stubs, back-drill, return path)
//! * `eye`      – eye diagram and board-level channel check

// shared board model, re-exported so modules can use `crate::board` etc.
pub use kisi_board::{board, geom, num, package};

pub mod analytic;
pub mod extract;
pub mod eye;
pub mod profile;
pub mod rf;
pub mod topology;
pub mod via;
pub mod via_pbm;
pub mod xsgeom;

pub use extract::{analyze, Analysis, Params};
