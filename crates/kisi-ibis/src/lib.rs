//! kisi-ibis: parser and checks for IBIS files (I/O Buffer Information Specification).
//!
//! Reads the keywords of IBIS 1.1–7.2 that matter for a physical-channel flow: header,
//! [Component] with [Package], [Pin], [Diff Pin], [Pin Mapping], [Series Pin Mapping],
//! [Package Model]; [Model Selector]; [Model] with Model_type, C_comp, thresholds,
//! references, I-V tables, [Ramp], waveforms, [Model Spec], [Receiver Thresholds],
//! [Driver Schedule], series and terminator elements; [Submodel] and [Define Package Model]
//! as data. Keywords it does not use (IBIS-AMI, external models, interconnect models, EBD,
//! power-aware tables …) are reported, never dropped silently.
//!
//! No file system access: the input is the file's bytes (also in WebAssembly). The result is
//! data plus `Issue`s with line numbers; `derive` computes values from the tables (marked as
//! derived wherever they are shown), `check` the IBISCHK-like checks.
//!
//! The parser structure follows KiCad's KIBIS (BSD-3-Clause, see `parser.rs` and `NOTICE`).

pub mod check;
pub mod derive;
pub mod num;
mod parser;
pub mod sha256;
pub mod types;

pub use num::{Corner, Tmm};
pub use types::*;

/// Licence notice of the ported parser logic (KIBIS, BSD-3-Clause), the full text of `NOTICE`. The front
/// ends do not show it; the bundlers carry it as a comment (tools/bundle.py, tools/bundle_cli.py) and the
/// README credits it.
pub const NOTICE: &str = include_str!("../NOTICE");

/// Findings of one code at one place that are listed before the rest is summarised.
pub const KEEP_PER_CODE: usize = 5;

/// Text of an IBIS file: UTF-8, else Latin-1 (vendor files often carry a © byte); a leading
/// byte-order mark is dropped.
pub fn decode(bytes: &[u8]) -> String {
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s)
}

/// Parse IBIS text (parser findings only, no checks).
pub fn parse(text: &str) -> IbisFile {
    parser::parse_text(text)
}

/// Parse a file's bytes and run the checks. `file_name`: the name the file is stored under
/// (compared with [File Name]). Findings are sorted by line, repeated ones summarised.
pub fn read(bytes: &[u8], file_name: Option<&str>) -> IbisFile {
    let mut f = parse(&decode(bytes));
    let extra = check::check(&f, file_name);
    let mut all = std::mem::take(&mut f.issues);
    all.extend(extra);
    f.issues = check::condense(all, KEEP_PER_CODE);
    f
}
