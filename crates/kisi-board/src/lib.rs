//! kisi-board: the board model every analysis builds on.
//!
//! * `board` – parser for `.kicad_pcb` / `.kicad_pro` (KiCad 8/9/10), stackup, tracks, vias, pads, zones
//! * `sexpr` – S-expression reader
//! * `geom`  – 2D geometry helpers
//! * `package` – case size of two-terminal chip parts (footprint name, pad geometry)
//! * `view`  – compact scene for the browser viewer
//! * `num`   – complex numbers and physical constants

pub mod board;
pub mod geom;
pub mod num;
pub mod package;
pub mod sexpr;
pub mod view;

pub use board::{apply_project, load_pcb, Board};
