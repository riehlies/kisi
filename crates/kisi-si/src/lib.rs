//! kisi-si: signal integrity.
//!
//! * `extract`  – cross-sections along every trace (references, coplanar ground, pairs)
//! * `analytic` – closed-form impedance formulas
//! * `classname` – target impedance from a net class name (`data/netclass_names.json`)
//! * `xsgeom`   – cross-section geometry text for the 2D field solver
//! * `rf`       – RLGC line models, channel S-parameters/TDR, Touchstone, crosstalk
//! * `via`      – via model (antipads, stubs, back-drill, return path)
//! * `eye`      – eye diagram and board-level channel check
//! * `accuracy` – accuracy levels and their validated uncertainty (`data/accuracy.json`)

// shared board model, re-exported so modules can use `crate::board` etc.
pub use kisi_board::{board, geom, num, package};

pub mod accuracy;
pub mod analytic;
pub mod classname;
pub mod export;
pub mod extract;
pub mod eye;
pub mod lanenet;
pub mod profile;
pub mod rf;
pub mod topology;
pub mod via;
pub mod via_cavity;
pub mod via_pbm;
pub mod xsgeom;

pub use extract::{analyze, Analysis, Params};
