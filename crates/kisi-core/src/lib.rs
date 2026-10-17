//! kisi-core: KiCad signal-integrity extraction.
//!
//! Milestone M1: board parser, stackup, cross-section extraction and
//! closed-form impedance evaluation.

pub mod analytic;
pub mod board;
pub mod extract;
pub mod geom;
pub mod pi;
pub mod sexpr;
pub mod view;
pub mod xsgeom;

pub use board::{apply_project, load_pcb, Board};
pub use extract::{analyze, Analysis, Params};
