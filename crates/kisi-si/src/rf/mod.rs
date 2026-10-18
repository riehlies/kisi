//! High-frequency models (milestone M3): frequency-dependent RLGC per
//! cross-section, the channel of a net (cascade → S-parameters, TDR) and
//! crosstalk to neighbouring traces.
//!
//! Units inside this module are SI (m, Ω, H, S, F, Hz, s) unless a name says
//! otherwise; board geometry (mm) is converted on entry.


mod channel;
mod crosstalk;
mod line;
#[cfg(test)]
mod tests;

pub use crate::num::{Cx, C0, MU0};
pub use channel::*;
pub use crosstalk::*;
pub use line::*;
