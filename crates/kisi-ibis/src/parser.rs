//! Line-oriented IBIS parser.
//!
//! The structure – a section context ([Component], [Model], [Model Selector], …) plus a
//! "continuation" state that says what the data lines after a keyword mean – and the rules for
//! keywords, the comment character, [Pin] columns, [Package] rows, [Ramp] dV/dt values,
//! waveform fixtures and package-model matrices follow the KIBIS parser of KiCad
//! (`eeschema/sim/kibis/ibis_parser.cpp`, read at commit 874060a). Unlike KIBIS this parser
//! never stops at a problem: every finding becomes an `Issue` with its line, and keywords it
//! does not use are reported, never dropped silently.
//!
//! Ported from KIBIS under the following licence:
//!
//! Copyright (C) 2022 Fabien Corona f.corona<at>laposte.net
//! Copyright The KiCad Developers, see AUTHORS.txt for contributors.
//!
//! Redistribution and use in source and binary forms, with or without modification,
//! are permitted provided that the following conditions are met:
//!
//! 1. Redistributions of source code must retain the above copyright notice,
//!    this list of conditions and the following disclaimer.
//!
//! 2. Redistributions in binary form must reproduce the above copyright notice,
//!    this list of conditions and the following disclaimer in the documentation
//!    and/or other materials provided with the distribution.
//!
//! 3. Neither the name of the copyright holder nor the names of its contributors may be used
//!    to endorse or promote products derived from this software without specific
//!    prior written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS
//! OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY
//! AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
//! CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
//! CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//! SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
//! THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
//! OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
//! POSSIBILITY OF SUCH DAMAGE.

use crate::num::{self, Num, Tmm};
use crate::types::*;
use std::collections::BTreeMap;

/// Longest line the parser accepts without a note (KIBIS: "official limit is 1024").
const MAX_LINE: usize = 1024;
/// Characters allowed as comment character ([Comment Char]).
const COMMENT_CHARS: &str = "!\"#$%&'()*,:;<>?@\\^`{|}~";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Ctx {
    /// before the first keyword
    Start,
    Header,
    Component(usize),
    Selector(usize),
    Model(usize),
    Submodel(usize),
    Pkg(usize),
    PkgData(usize),
    /// after a top-level block kisi skips
    Top,
    End,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Text {
    Source,
    Notes,
    Disclaimer,
    Copyright,
    Manufacturer(usize),
    SiLocation(usize),
    TimingLocation(usize),
    PkgManufacturer(usize),
    PkgOem(usize),
    PkgDescription(usize),
}

/// Which table data lines go to.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tab {
    Pulldown,
    Pullup,
    GndClamp,
    PowerClamp,
    Extra(usize),
    SeriesCurrent,
    Mosfet(usize),
    Sub(usize),
}

/// What the data lines after the last keyword mean.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Cont {
    None,
    Ignore,
    Text(Text),
    Package,
    Pin,
    PinMapping,
    DiffPin,
    SeriesPin,
    Selector,
    ModelParams,
    Table(Tab),
    Waveform,
    Composite,
    Ramp,
    ModelSpec,
    RxThresholds,
    DriverSchedule,
    AddSubmodel,
    SubmodelParams,
    SubmodelSpec,
    PkgPins,
    Matrix,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Series {
    Main,
    On,
    Off,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mat {
    R,
    L,
    C,
}

struct Unsup {
    count: usize,
    line: usize,
    display: String,
    why: &'static str,
}

struct Parser {
    f: IbisFile,
    ctx: Ctx,
    cont: Cont,
    line: usize,
    comment: char,
    /// skipping an unsupported block: until this keyword (None: until the next top-level one)
    skip: Option<Option<&'static str>>,
    series: Series,
    /// [Pin]: row columns of R_pin, L_pin, C_pin
    pin_cols: Option<[usize; 3]>,
    pin_assumed_noted: bool,
    /// package model matrix being read: which, row, next column
    matrix: Option<(Mat, Option<usize>, usize)>,
    last_component: Option<usize>,
    unsup: BTreeMap<String, Unsup>,
    after_end_noted: bool,
    /// line of the last [Comment Char] that changed the character
    comment_line: usize,
    /// lines starting with '|' after the comment character was changed (read as comments)
    old_comment_lines: usize,
}

/// Keywords the parser knows (normalised: lower case, spaces as `_`).
const KNOWN: &[&str] = &[
    "ibis_ver", "comment_char", "file_name", "file_rev", "date", "source", "notes", "disclaimer", "copyright", "component", "manufacturer", "package", "pin", "package_model",
    "pin_mapping", "diff_pin", "series_pin_mapping", "series_switch_groups", "si_location", "timing_location", "die_supply_pads", "bus_label", "alternate_package_models",
    "end_alternate_package_models", "node_declarations", "end_node_declarations", "circuit_call", "end_circuit_call", "model_selector", "model", "model_spec", "receiver_thresholds",
    "voltage_range", "temperature_range", "pullup_reference", "pulldown_reference", "power_clamp_reference", "gnd_clamp_reference", "external_reference", "pulldown", "pullup",
    "gnd_clamp", "power_clamp", "isso_pu", "isso_pd", "composite_current", "rgnd", "rpower", "rac", "cac", "ttgnd", "ttpower", "ramp", "rising_waveform", "falling_waveform",
    "driver_schedule", "add_submodel", "on", "off", "r_series", "l_series", "c_series", "rl_series", "rc_series", "lc_series", "series_current", "series_mosfet",
    "algorithmic_model", "end_algorithmic_model", "external_model", "end_external_model", "external_circuit", "end_external_circuit", "submodel", "submodel_spec",
    "gnd_pulse_table", "power_pulse_table", "define_package_model", "end_package_model", "oem", "description", "number_of_pins", "pin_numbers", "model_data", "end_model_data",
    "resistance_matrix", "capacitance_matrix", "inductance_matrix", "bandwidth", "row", "merged_pins", "number_of_sections", "test_data", "test_load", "interconnect_model_set",
    "end_interconnect_model_set", "interconnect_model_group", "end_interconnect_model_group", "begin_board_description", "end_board_description", "end",
];

/// Keywords that start a new top-level section.
fn top_level(kw: &str) -> bool {
    matches!(
        kw,
        "component" | "model_selector" | "model" | "submodel" | "define_package_model" | "test_data" | "test_load" | "interconnect_model_set" | "interconnect_model_group" | "begin_board_description" | "external_circuit" | "end"
    )
}

/// `[Model Selector]` → `model_selector`.
fn norm(kw: &str) -> String {
    let mut out = String::new();
    for w in kw.trim().split(|c: char| c == ' ' || c == '_' || c == '\t').filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push('_');
        }
        out.push_str(&w.to_ascii_lowercase());
    }
    out
}

/// `name = value`, `name value …` → (name, values).
fn param(text: &str) -> (&str, Vec<&str>) {
    let t = text.trim();
    let end = t.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(t.len());
    let mut rest = t[end..].trim_start();
    if let Some(r) = rest.strip_prefix('=') {
        rest = r.trim_start();
    }
    (&t[..end], rest.split_whitespace().collect())
}

fn canonical_type(t: &str) -> Option<&'static str> {
    MODEL_TYPES.iter().find(|m| m.eq_ignore_ascii_case(t)).copied()
}

/// Model sub-parameters with one value besides Vinl … Cref.
const SINGLE_PARAMS: &[&str] = &["Cref_rising", "Cref_falling", "Rref_rising", "Rref_falling", "Vref_rising", "Vref_falling", "Vmeas_rising", "Vmeas_falling", "Rref_diff", "Cref_diff"];

pub fn parse_text(text: &str) -> IbisFile {
    let mut p = Parser {
        f: IbisFile::default(),
        ctx: Ctx::Start,
        cont: Cont::None,
        line: 0,
        comment: '|',
        skip: None,
        series: Series::Main,
        pin_cols: None,
        pin_assumed_noted: false,
        matrix: None,
        last_component: None,
        unsup: BTreeMap::new(),
        after_end_noted: false,
        comment_line: 0,
        old_comment_lines: 0,
    };
    for raw in text.split('\n') {
        p.line += 1;
        p.feed(raw.strip_suffix('\r').unwrap_or(raw));
    }
    p.finish(text.lines().count());
    p.f
}

impl Parser {
    fn at(&self) -> Option<String> {
        match self.ctx {
            Ctx::Start | Ctx::Header => Some("header".into()),
            Ctx::Component(i) => Some(format!("component {}", self.f.components[i].name)),
            Ctx::Selector(i) => Some(format!("model selector {}", self.f.model_selectors[i].name)),
            Ctx::Model(i) => Some(format!("model {}", self.f.models[i].name)),
            Ctx::Submodel(i) => Some(format!("submodel {}", self.f.submodels[i].name)),
            Ctx::Pkg(i) | Ctx::PkgData(i) => Some(format!("package model {}", self.f.package_models[i].name)),
            Ctx::Top | Ctx::End => None,
        }
    }

    fn issue(&mut self, severity: Severity, code: &'static str, message: impl Into<String>) {
        let at = self.at();
        self.f.issues.push(Issue { severity, line: Some(self.line), code, at, message: message.into() });
    }

    fn unsupported(&mut self, kw: &str, display: &str, why: &'static str) {
        let line = self.line;
        let e = self.unsup.entry(kw.to_string()).or_insert(Unsup { count: 0, line, display: format!("[{}]", display.trim()), why });
        e.count += 1;
    }

    fn feed(&mut self, raw: &str) {
        if self.ctx == Ctx::End {
            let t = raw.split(self.comment).next().unwrap_or("");
            if !self.after_end_noted && !t.trim().is_empty() {
                self.after_end_noted = true;
                self.issue(Severity::Note, "after-end", "text after [END] is ignored");
            }
            return;
        }
        if raw.chars().count() > MAX_LINE {
            self.issue(Severity::Warning, "line-length", format!("line longer than {MAX_LINE} characters"));
        }
        // [Comment Char] is read before comments are cut off: its argument is often the
        // comment character itself
        let lead = raw.trim_start();
        if let Some(rest) = lead.strip_prefix('[') {
            if let Some(j) = rest.find(']') {
                if norm(&rest[..j]) == "comment_char" {
                    if self.skip.is_none() {
                        self.comment_char(&rest[j + 1..]);
                    }
                    return;
                }
            }
        }
        let text = match raw.find(self.comment) {
            Some(i) => &raw[..i],
            None => raw,
        };
        if text.trim().is_empty() {
            return;
        }
        let t = text.trim_start();
        // files that change the comment character and keep using '|' at the start of a line:
        // no data starts with '|', so read it as a comment (reported once)
        if self.comment != '|' && t.starts_with('|') {
            if self.old_comment_lines == 0 && self.skip.is_none() {
                self.issue(
                    Severity::Warning,
                    "comment-char",
                    format!("line starts with '|' although [Comment Char] (line {}) made '{}' the comment character: read as a comment, like the further such lines", self.comment_line, self.comment),
                );
            }
            self.old_comment_lines += 1;
            return;
        }
        if let Some(rest) = t.strip_prefix('[') {
            match rest.find(']') {
                Some(j) => {
                    let col1 = t.len() == text.len();
                    self.keyword(&rest[..j], &rest[j + 1..], col1);
                }
                None => {
                    if self.skip.is_none() {
                        self.issue(Severity::Error, "bad-keyword", format!("keyword without closing ']': '{}' (line ignored)", t.trim()));
                    }
                }
            }
        } else if self.skip.is_none() {
            self.data(text);
        }
    }

    fn comment_char(&mut self, arg: &str) {
        let a = arg.trim();
        let mut chars = a.chars();
        let Some(c) = chars.next() else {
            self.issue(Severity::Error, "comment-char", "[Comment Char] without a character");
            return;
        };
        if !chars.as_str().starts_with("_char") {
            self.issue(Severity::Error, "comment-char", format!("[Comment Char] '{a}': write it as {c}_char (unchanged: '{}')", self.comment));
            return;
        }
        if !COMMENT_CHARS.contains(c) {
            self.issue(Severity::Error, "comment-char", format!("[Comment Char] '{c}' is not an allowed comment character (unchanged: '{}')", self.comment));
            return;
        }
        self.comment = c;
        self.comment_line = self.line;
        self.cont = Cont::None;
    }

    fn keyword(&mut self, name: &str, rest: &str, col1: bool) {
        let kw = norm(name);
        if let Some(until) = self.skip {
            match until {
                Some(end) => {
                    if kw == end {
                        self.skip = None;
                    }
                    return;
                }
                None if top_level(&kw) => self.skip = None,
                None => return,
            }
        }
        self.cont = Cont::None;
        if self.ctx == Ctx::Start {
            if kw != "ibis_ver" {
                self.issue(Severity::Error, "first-keyword", format!("the first keyword must be [IBIS Ver], found [{}]", name.trim()));
            }
            self.ctx = Ctx::Header;
        }
        if !col1 {
            self.issue(Severity::Warning, "keyword-column", format!("[{}] does not start in column 1", name.trim()));
        }
        let rest_t = rest.trim();
        let first = rest.split_whitespace().next().unwrap_or("").to_string();
        match kw.as_str() {
            "ibis_ver" => {
                if self.f.header.ibis_ver.is_some() {
                    self.issue(Severity::Warning, "duplicate-keyword", "[IBIS Ver] given twice");
                }
                if first.is_empty() {
                    self.issue(Severity::Error, "missing-value", "[IBIS Ver] without a version");
                }
                self.f.header.ibis_ver = (!first.is_empty()).then_some(first);
                self.f.header.ibis_ver_line = Some(self.line);
            }
            "file_name" | "file_rev" | "date" => {
                let v = (!rest_t.is_empty()).then(|| rest_t.to_string());
                if v.is_none() {
                    self.issue(Severity::Error, "missing-value", format!("[{}] without a value", name.trim()));
                }
                let h = &mut self.f.header;
                match kw.as_str() {
                    "file_name" => h.file_name = v,
                    "file_rev" => h.file_rev = v,
                    _ => h.date = v,
                }
            }
            "source" | "notes" | "disclaimer" | "copyright" => {
                let t = match kw.as_str() {
                    "source" => Text::Source,
                    "notes" => Text::Notes,
                    "disclaimer" => Text::Disclaimer,
                    _ => Text::Copyright,
                };
                self.text(t, rest_t);
                self.cont = Cont::Text(t);
            }
            "end" => {
                self.f.ended = true;
                self.ctx = Ctx::End;
            }
            "component" => {
                if rest_t.is_empty() {
                    self.issue(Severity::Error, "missing-value", "[Component] without a name");
                }
                self.f.components.push(Component { name: rest_t.to_string(), line: self.line, ..Default::default() });
                let i = self.f.components.len() - 1;
                self.ctx = Ctx::Component(i);
                self.last_component = Some(i);
                self.pin_cols = None;
            }
            "model_selector" => {
                self.f.model_selectors.push(ModelSelector { name: first.clone(), line: self.line, entries: vec![] });
                self.ctx = Ctx::Selector(self.f.model_selectors.len() - 1);
                self.name_check("[Model Selector]", rest_t);
                self.cont = Cont::Selector;
            }
            "model" => {
                self.f.models.push(Model { name: first.clone(), line: self.line, ..Default::default() });
                self.ctx = Ctx::Model(self.f.models.len() - 1);
                self.name_check("[Model]", rest_t);
                self.series = Series::Main;
                self.cont = Cont::ModelParams;
            }
            "submodel" => {
                self.f.submodels.push(Submodel { name: first.clone(), line: self.line, ..Default::default() });
                self.ctx = Ctx::Submodel(self.f.submodels.len() - 1);
                self.name_check("[Submodel]", rest_t);
                self.unsupported("submodel", name, "submodels (dynamic clamp, bus hold) are parsed but not used by kisi");
                self.cont = Cont::SubmodelParams;
            }
            "define_package_model" => {
                self.f.package_models.push(PackageModel { name: first.clone(), line: self.line, ..Default::default() });
                self.ctx = Ctx::Pkg(self.f.package_models.len() - 1);
                self.name_check("[Define Package Model]", rest_t);
                self.unsupported("define_package_model", name, "package model (coupled R/L/C matrices) parsed and stored; kisi uses the uncoupled [Pin]/[Package] R/L/C so far");
                self.matrix = None;
            }
            "test_data" | "test_load" => {
                self.unsupported(&kw, name, "test data and loads are not used by kisi (section skipped)");
                self.skip = Some(None);
                self.ctx = Ctx::Top;
            }
            "interconnect_model_set" | "interconnect_model_group" => {
                self.unsupported(&kw, name, "IBIS 7 interconnect models (IBIS-ISS / Touchstone) are not supported (block skipped)");
                self.skip = Some(Some(if kw == "interconnect_model_set" { "end_interconnect_model_set" } else { "end_interconnect_model_group" }));
                self.ctx = Ctx::Top;
            }
            "begin_board_description" => {
                self.unsupported(&kw, name, "electrical board descriptions (EBD) are not supported (block skipped)");
                self.skip = Some(Some("end_board_description"));
                self.ctx = Ctx::Top;
            }
            "external_circuit" => {
                self.unsupported(&kw, name, "external circuits (SPICE, IBIS-ISS, Verilog-A) are not supported (block skipped)");
                self.skip = Some(Some("end_external_circuit"));
                self.ctx = Ctx::Top;
            }
            _ => self.section_keyword(&kw, name, rest),
        }
    }

    /// A section name must be one word.
    fn name_check(&mut self, what: &str, rest: &str) {
        let n = rest.split_whitespace().count();
        if n == 0 {
            self.issue(Severity::Error, "missing-value", format!("{what} without a name"));
        } else if n > 1 {
            self.issue(Severity::Warning, "name", format!("{what} name '{rest}' has spaces: only '{}' is used", rest.split_whitespace().next().unwrap_or("")));
        }
    }

    fn misplaced(&mut self, kw: &str, name: &str) {
        let where_ = self.at().unwrap_or_else(|| "this place".into());
        if KNOWN.contains(&kw) {
            self.issue(Severity::Warning, "misplaced-keyword", format!("[{}] is not expected in {where_}: ignored", name.trim()));
        } else {
            self.issue(Severity::Warning, "unknown-keyword", format!("unknown keyword [{}]: ignored", name.trim()));
        }
        self.cont = Cont::Ignore;
    }

    fn section_keyword(&mut self, kw: &str, name: &str, rest: &str) {
        let rest_t = rest.trim();
        match self.ctx {
            Ctx::Component(i) => match kw {
                "manufacturer" => {
                    self.text(Text::Manufacturer(i), rest_t);
                    self.cont = Cont::Text(Text::Manufacturer(i));
                }
                "si_location" | "timing_location" => {
                    let t = if kw == "si_location" { Text::SiLocation(i) } else { Text::TimingLocation(i) };
                    self.text(t, rest_t);
                }
                "package" => {
                    self.f.components[i].package.line = Some(self.line);
                    self.cont = Cont::Package;
                    if !rest_t.is_empty() {
                        self.data(rest);
                    }
                }
                "pin" => {
                    self.pin_header(rest_t);
                    self.cont = Cont::Pin;
                }
                "package_model" => {
                    let n = rest.split_whitespace().next().map(str::to_string);
                    if n.is_none() {
                        self.issue(Severity::Error, "missing-value", "[Package Model] without a name");
                    }
                    self.f.components[i].package_model = n;
                }
                "pin_mapping" => self.cont = Cont::PinMapping,
                "diff_pin" => self.cont = Cont::DiffPin,
                "series_pin_mapping" => self.cont = Cont::SeriesPin,
                "series_switch_groups" | "die_supply_pads" | "bus_label" => {
                    self.unsupported(kw, name, "not used by kisi (rows skipped)");
                    self.cont = Cont::Ignore;
                }
                "alternate_package_models" | "node_declarations" | "circuit_call" => {
                    self.unsupported(kw, name, "not supported (block skipped)");
                    self.skip = Some(Some(match kw {
                        "alternate_package_models" => "end_alternate_package_models",
                        "node_declarations" => "end_node_declarations",
                        _ => "end_circuit_call",
                    }));
                }
                _ => self.misplaced(kw, name),
            },
            Ctx::Model(i) => {
                self.f.models[i].seen.push((kw.to_string(), self.line));
                self.model_keyword(i, kw, name, rest)
            }
            Ctx::Submodel(i) => match kw {
                "submodel_spec" => self.cont = Cont::SubmodelSpec,
                "pulldown" | "pullup" | "gnd_clamp" | "power_clamp" | "gnd_pulse_table" | "power_pulse_table" => {
                    let s = &mut self.f.submodels[i];
                    s.tables.push((name.trim().to_string(), Table { line: self.line, rows: vec![] }));
                    self.cont = Cont::Table(Tab::Sub(s.tables.len() - 1));
                }
                "ramp" => {
                    self.f.submodels[i].ramp = Some(Ramp { line: self.line, ..Default::default() });
                    self.cont = Cont::Ramp;
                }
                "rising_waveform" | "falling_waveform" => {
                    self.f.submodels[i].waveforms.push(Waveform { line: self.line, rising: kw == "rising_waveform", ..Default::default() });
                    self.cont = Cont::Waveform;
                }
                _ => self.misplaced(kw, name),
            },
            Ctx::Pkg(i) | Ctx::PkgData(i) => match kw {
                "manufacturer" | "oem" | "description" => {
                    let t = match kw {
                        "manufacturer" => Text::PkgManufacturer(i),
                        "oem" => Text::PkgOem(i),
                        _ => Text::PkgDescription(i),
                    };
                    self.text(t, rest_t);
                    self.cont = Cont::Text(t);
                }
                "number_of_pins" => match rest_t.split_whitespace().next().and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) => self.f.package_models[i].number_of_pins = Some(n),
                    None => self.issue(Severity::Error, "bad-number", format!("[Number Of Pins] '{rest_t}' is not a pin count")),
                },
                "pin_numbers" => self.cont = Cont::PkgPins,
                "model_data" => self.ctx = Ctx::PkgData(i),
                "end_model_data" => {
                    self.ctx = Ctx::Pkg(i);
                    self.matrix = None;
                }
                "resistance_matrix" | "capacitance_matrix" | "inductance_matrix" => {
                    let which = match kw {
                        "resistance_matrix" => Mat::R,
                        "capacitance_matrix" => Mat::C,
                        _ => Mat::L,
                    };
                    let kind = rest.split_whitespace().next().unwrap_or("").to_string();
                    let nk = norm(&kind);
                    if !matches!(nk.as_str(), "full_matrix" | "banded_matrix" | "sparse_matrix") {
                        self.issue(Severity::Error, "matrix", format!("[{}] type '{kind}' unknown (Full_matrix, Banded_matrix or Sparse_matrix)", name.trim()));
                    }
                    let m = Matrix { line: self.line, kind: nk, bandwidth: None, entries: vec![] };
                    let pm = &mut self.f.package_models[i];
                    let slot = match which {
                        Mat::R => &mut pm.r,
                        Mat::L => &mut pm.l,
                        Mat::C => &mut pm.c,
                    };
                    let dup = slot.is_some();
                    *slot = Some(m);
                    if dup {
                        self.issue(Severity::Warning, "duplicate-keyword", format!("[{}] given twice (the last one is kept)", name.trim()));
                    }
                    self.matrix = Some((which, None, 0));
                }
                "bandwidth" => match (rest_t.split_whitespace().next().and_then(|s| s.parse::<usize>().ok()), self.matrix) {
                    (Some(b), Some((w, _, _))) => {
                        if let Some(m) = self.matrix_mut(i, w) {
                            m.bandwidth = Some(b);
                        }
                    }
                    (None, _) => self.issue(Severity::Error, "bad-number", format!("[Bandwidth] '{rest_t}' is not a number")),
                    (_, None) => self.issue(Severity::Warning, "matrix", "[Bandwidth] outside a matrix"),
                },
                "row" => {
                    let pin = rest.split_whitespace().next().unwrap_or("");
                    let idx = self.pkg_pin(i, pin);
                    match (idx, self.matrix) {
                        (Some(r), Some((w, _, _))) => {
                            let start = match self.matrix_mut(i, w).map(|m| m.kind.clone()).as_deref() {
                                Some("full_matrix") => r,
                                _ => 0,
                            };
                            self.matrix = Some((w, Some(r), start));
                            self.cont = Cont::Matrix;
                        }
                        (None, _) => self.issue(Severity::Warning, "matrix", format!("[Row] '{pin}' is not in [Pin Numbers]")),
                        (_, None) => self.issue(Severity::Warning, "matrix", "[Row] outside a matrix"),
                    }
                }
                "end_package_model" => {
                    self.ctx = match self.last_component {
                        Some(c) => Ctx::Component(c),
                        None => Ctx::Header,
                    };
                    self.matrix = None;
                }
                "merged_pins" | "number_of_sections" => {
                    self.unsupported(kw, name, "package model sections are not parsed");
                    self.cont = Cont::Ignore;
                }
                _ => self.misplaced(kw, name),
            },
            _ => self.misplaced(kw, name),
        }
    }

    fn model_keyword(&mut self, i: usize, kw: &str, name: &str, rest: &str) {
        let toks: Vec<&str> = rest.split_whitespace().collect();
        let disp = format!("[{}]", name.trim());
        match kw {
            "voltage_range" | "temperature_range" | "pullup_reference" | "pulldown_reference" | "power_clamp_reference" | "gnd_clamp_reference" | "external_reference" | "rgnd"
            | "rpower" | "rac" | "cac" => {
                let v = self.tmm(&toks, &disp, true);
                let m = &mut self.f.models[i];
                let slot = match kw {
                    "voltage_range" => &mut m.voltage_range,
                    "temperature_range" => &mut m.temperature_range,
                    "pullup_reference" => &mut m.pullup_ref,
                    "pulldown_reference" => &mut m.pulldown_ref,
                    "power_clamp_reference" => &mut m.power_clamp_ref,
                    "gnd_clamp_reference" => &mut m.gnd_clamp_ref,
                    "external_reference" => &mut m.ext_ref,
                    "rgnd" => &mut m.rgnd,
                    "rpower" => &mut m.rpower,
                    "rac" => &mut m.rac,
                    _ => &mut m.cac,
                };
                let dup = slot.is_some();
                *slot = v;
                if dup {
                    self.issue(Severity::Warning, "duplicate-keyword", format!("{disp} given twice (the last one is kept)"));
                }
            }
            "ttgnd" | "ttpower" => {
                let v = self.tmm(&toks, &disp, true);
                if let Some(v) = v {
                    self.f.models[i].extra_values.push(NamedTmm { name: name.trim().to_string(), value: v, line: self.line });
                }
                self.unsupported(kw, name, "parsed, not used by kisi");
            }
            "pulldown" | "pullup" | "gnd_clamp" | "power_clamp" => {
                let t = Some(Table { line: self.line, rows: vec![] });
                let m = &mut self.f.models[i];
                let (slot, tab) = match kw {
                    "pulldown" => (&mut m.pulldown, Tab::Pulldown),
                    "pullup" => (&mut m.pullup, Tab::Pullup),
                    "gnd_clamp" => (&mut m.gnd_clamp, Tab::GndClamp),
                    _ => (&mut m.power_clamp, Tab::PowerClamp),
                };
                let dup = slot.is_some();
                *slot = t;
                if dup {
                    self.issue(Severity::Warning, "duplicate-keyword", format!("{disp} given twice (the last one is kept)"));
                }
                self.cont = Cont::Table(tab);
                self.table_rest(&toks, &disp);
            }
            "isso_pu" | "isso_pd" => {
                let m = &mut self.f.models[i];
                m.extra_tables.push((name.trim().to_string(), Table { line: self.line, rows: vec![] }));
                self.cont = Cont::Table(Tab::Extra(m.extra_tables.len() - 1));
                self.unsupported(kw, name, "power-aware data: parsed, not used by kisi");
            }
            "composite_current" => {
                self.unsupported(kw, name, "power-aware data: parsed, not used by kisi");
                match self.f.models[i].waveforms.last_mut() {
                    Some(w) => {
                        w.composite_current = Some(Table { line: self.line, rows: vec![] });
                        self.cont = Cont::Composite;
                    }
                    None => {
                        self.issue(Severity::Warning, "misplaced-keyword", "[Composite Current] without a waveform before it: ignored");
                        self.cont = Cont::Ignore;
                    }
                }
            }
            "ramp" => {
                if self.f.models[i].ramp.is_some() {
                    self.issue(Severity::Warning, "duplicate-keyword", "[Ramp] given twice (the last one is kept)");
                }
                self.f.models[i].ramp = Some(Ramp { line: self.line, ..Default::default() });
                self.cont = Cont::Ramp;
            }
            "rising_waveform" | "falling_waveform" => {
                self.f.models[i].waveforms.push(Waveform { line: self.line, rising: kw == "rising_waveform", ..Default::default() });
                self.cont = Cont::Waveform;
            }
            "model_spec" => self.cont = Cont::ModelSpec,
            "receiver_thresholds" => self.cont = Cont::RxThresholds,
            "driver_schedule" => {
                self.unsupported(kw, name, "multi-stage drivers (pre-emphasis): rows kept as data, not used by kisi");
                self.cont = Cont::DriverSchedule;
            }
            "add_submodel" => {
                self.cont = Cont::AddSubmodel;
                if toks.len() >= 2 {
                    self.data(rest);
                }
            }
            "on" | "off" => {
                let m = &mut self.f.models[i];
                let slot = if kw == "on" { &mut m.on } else { &mut m.off };
                let dup = slot.is_some();
                if !dup {
                    *slot = Some(SeriesData::default());
                }
                self.series = if kw == "on" { Series::On } else { Series::Off };
                if dup {
                    self.issue(Severity::Warning, "duplicate-keyword", format!("{disp} given twice"));
                }
            }
            "r_series" | "l_series" | "c_series" | "rl_series" | "rc_series" | "lc_series" => {
                let v = self.tmm(&toks, &disp, true);
                let s = series_mut(&mut self.f.models[i], self.series);
                let slot = match kw {
                    "r_series" => &mut s.r,
                    "l_series" => &mut s.l,
                    "c_series" => &mut s.c,
                    "rl_series" => &mut s.rl,
                    "rc_series" => &mut s.rc,
                    _ => &mut s.lc,
                };
                *slot = v;
            }
            "series_current" => {
                series_mut(&mut self.f.models[i], self.series).current = Some(Table { line: self.line, rows: vec![] });
                self.cont = Cont::Table(Tab::SeriesCurrent);
            }
            "series_mosfet" => {
                let s = series_mut(&mut self.f.models[i], self.series);
                s.mosfet.push((None, Table { line: self.line, rows: vec![] }));
                self.cont = Cont::Table(Tab::Mosfet(s.mosfet.len() - 1));
            }
            "algorithmic_model" => {
                self.unsupported(kw, name, "IBIS-AMI (equalisation) is out of scope (block skipped)");
                self.skip = Some(Some("end_algorithmic_model"));
            }
            "external_model" => {
                self.unsupported(kw, name, "external models (SPICE, IBIS-ISS, Verilog-A) are not supported (block skipped)");
                self.skip = Some(Some("end_external_model"));
            }
            _ => self.misplaced(kw, name),
        }
    }

    /// Text after a table keyword: a first row, or ignored.
    fn table_rest(&mut self, toks: &[&str], disp: &str) {
        if toks.is_empty() {
            return;
        }
        if num::parse(toks[0]).is_ok() {
            self.data(&toks.join(" "));
        } else {
            self.issue(Severity::Warning, "stray-text", format!("text after {disp} ignored: '{}'", toks.join(" ")));
        }
    }

    /// Set a text value or append a line to it (the value may start on the next line).
    fn text(&mut self, t: Text, s: &str) {
        let h = &mut self.f.header;
        let slot: &mut Option<String> = match t {
            Text::Source => &mut h.source,
            Text::Notes => &mut h.notes,
            Text::Disclaimer => &mut h.disclaimer,
            Text::Copyright => &mut h.copyright,
            Text::Manufacturer(i) => &mut self.f.components[i].manufacturer,
            Text::SiLocation(i) => &mut self.f.components[i].si_location,
            Text::TimingLocation(i) => &mut self.f.components[i].timing_location,
            Text::PkgManufacturer(i) => &mut self.f.package_models[i].manufacturer,
            Text::PkgOem(i) => &mut self.f.package_models[i].oem,
            Text::PkgDescription(i) => &mut self.f.package_models[i].description,
        };
        match slot {
            Some(old) if old.is_empty() => *old = s.to_string(),
            Some(old) if !s.is_empty() => {
                old.push('\n');
                old.push_str(s);
            }
            Some(_) => {}
            None => *slot = Some(s.to_string()),
        }
    }

    fn pin_header(&mut self, rest: &str) {
        let toks: Vec<&str> = rest.split_whitespace().collect();
        let col = |name: &str| toks.iter().position(|t| t.eq_ignore_ascii_case(name)).map(|k| k + 1);
        let (r, l, c) = (col("R_pin"), col("L_pin"), col("C_pin"));
        self.pin_cols = match (r, l, c) {
            (Some(r), Some(l), Some(c)) => Some([r, l, c]),
            (None, None, None) => None,
            _ => {
                self.issue(Severity::Error, "pin-columns", "[Pin] names some but not all of R_pin, L_pin, C_pin");
                None
            }
        };
        self.pin_assumed_noted = false;
        if let Ctx::Component(i) = self.ctx {
            self.f.components[i].pin_rlc_columns = self.pin_cols.is_some();
        }
    }

    /// typ/min/max from tokens; `cols`: warn when not exactly three columns; `row`: a table row
    /// (NA in typ leaves the row out of the typ curve – a warning, not an error).
    fn tmm_row(&mut self, toks: &[&str], what: &str, cols: bool, row: bool) -> Option<Tmm> {
        if toks.is_empty() {
            self.issue(Severity::Error, "missing-value", format!("{what}: value missing"));
            return None;
        }
        let mut out = Tmm::default();
        let mut bad = false;
        for (k, t) in toks.iter().take(3).enumerate() {
            match num::parse(t) {
                Ok(Num::Na) => {}
                Ok(Num::Val { v, suspect }) => {
                    if let Some(c) = suspect {
                        self.suspect(t, c, what);
                    }
                    match k {
                        0 => out.typ = Some(v),
                        1 => out.min = Some(v),
                        _ => out.max = Some(v),
                    }
                }
                Err(e) => {
                    bad = true;
                    self.issue(Severity::Error, "bad-number", format!("{what}: {e}"));
                }
            }
        }
        if out.typ.is_none() && !bad {
            if row {
                self.issue(Severity::Warning, "na-typ", format!("{what}: NA in the typ column (the row is left out of the typ curve)"));
            } else {
                self.issue(Severity::Error, "na-typ", format!("{what}: the typ value is NA (IBIS requires typ)"));
            }
        }
        if cols && toks.len() != 3 {
            self.issue(Severity::Warning, "columns", format!("{what}: {} value(s), expected typ, min and max (NA allowed)", toks.len()));
        }
        Some(out)
    }

    fn tmm(&mut self, toks: &[&str], what: &str, cols: bool) -> Option<Tmm> {
        self.tmm_row(toks, what, cols, false)
    }

    fn suspect(&mut self, tok: &str, c: char, what: &str) {
        self.issue(
            Severity::Warning,
            "scale-suffix",
            format!("{what}: '{tok}' – '{c}' is not an IBIS scaling factor (case matters: k, u, n, p) and is ignored"),
        );
    }

    fn value(&mut self, tok: Option<&&str>, what: &str) -> Option<f64> {
        let Some(t) = tok else {
            self.issue(Severity::Error, "missing-value", format!("{what}: value missing"));
            return None;
        };
        match num::parse(t) {
            Ok(Num::Na) => None,
            Ok(Num::Val { v, suspect }) => {
                if let Some(c) = suspect {
                    self.suspect(t, c, what);
                }
                Some(v)
            }
            Err(e) => {
                self.issue(Severity::Error, "bad-number", format!("{what}: {e}"));
                None
            }
        }
    }

    fn data(&mut self, text: &str) {
        let toks: Vec<&str> = text.split_whitespace().collect();
        if toks.is_empty() {
            return;
        }
        match self.cont {
            Cont::Ignore => {}
            Cont::None => {
                if self.ctx == Ctx::Start {
                    self.issue(Severity::Warning, "stray-data", "text before [IBIS Ver] ignored");
                    self.cont = Cont::Ignore;
                } else {
                    self.issue(Severity::Warning, "stray-data", format!("data line without a keyword that takes it: '{}' (ignored)", text.trim()));
                }
            }
            Cont::Text(t) => self.text(t, text.trim()),
            Cont::Package => self.package_row(&toks),
            Cont::Pin => self.pin_row(&toks),
            Cont::PinMapping => self.pin_mapping_row(&toks),
            Cont::DiffPin => self.diff_pin_row(&toks),
            Cont::SeriesPin => {
                let Ctx::Component(i) = self.ctx else { return };
                if toks.len() < 3 {
                    self.issue(Severity::Warning, "columns", "[Series Pin Mapping] row needs pin_1, pin_2 and model_name");
                    return;
                }
                let line = self.line;
                self.f.components[i].series_pins.push(SeriesPin { pin1: toks[0].into(), pin2: toks[1].into(), model: toks[2].into(), group: toks.get(3).map(|s| s.to_string()), line });
            }
            Cont::Selector => {
                let Ctx::Selector(i) = self.ctx else { return };
                let desc = text.trim()[toks[0].len()..].trim().to_string();
                let line = self.line;
                self.f.model_selectors[i].entries.push(SelectorEntry { model: toks[0].into(), description: desc, line });
            }
            Cont::ModelParams => self.model_param(text),
            Cont::Table(t) => self.table_row(t, &toks),
            Cont::Waveform => self.waveform_line(text, &toks),
            Cont::Composite => {
                if let Some(row) = self.row(&toks, "[Composite Current]") {
                    if let Some(t) = self.waveform_mut().and_then(|w| w.composite_current.as_mut()) {
                        t.rows.push(row);
                    }
                }
            }
            Cont::Ramp => self.ramp_line(text),
            Cont::ModelSpec | Cont::SubmodelSpec => {
                let (name, vals) = param(text);
                let what = format!("{} {name}", if self.cont == Cont::ModelSpec { "[Model Spec]" } else { "[Submodel Spec]" });
                let v = self.tmm(&vals, &what, false);
                let line = self.line;
                let e = v.map(|value| NamedTmm { name: name.to_string(), value, line });
                match (self.ctx, e) {
                    (Ctx::Model(i), Some(e)) => self.f.models[i].model_spec.push(e),
                    (Ctx::Submodel(i), Some(e)) => self.f.submodels[i].spec.push(e),
                    _ => {}
                }
            }
            Cont::RxThresholds => {
                let Ctx::Model(i) = self.ctx else { return };
                let (name, vals) = param(text);
                let value = vals.first().and_then(|t| num::value(t).ok());
                let line = self.line;
                self.f.models[i].receiver_thresholds.push(RxParam { name: name.to_string(), value, text: vals.join(" "), line });
            }
            Cont::DriverSchedule => {
                let Ctx::Model(i) = self.ctx else { return };
                if toks.len() != 5 {
                    self.issue(Severity::Warning, "columns", format!("[Driver Schedule] row has {} columns, expected 5 (model, Rise_on_dly, Rise_off_dly, Fall_on_dly, Fall_off_dly)", toks.len()));
                }
                let mut v = [None; 4];
                for (k, slot) in v.iter_mut().enumerate() {
                    if let Some(t) = toks.get(k + 1) {
                        *slot = self.value(Some(t), "[Driver Schedule]");
                    }
                }
                let line = self.line;
                self.f.models[i].driver_schedule.push(ScheduleRow { model: toks[0].into(), rise_on: v[0], rise_off: v[1], fall_on: v[2], fall_off: v[3], line });
            }
            Cont::AddSubmodel => {
                let Ctx::Model(i) = self.ctx else { return };
                let mode = toks.get(1).copied().unwrap_or("");
                if !["All", "Driving", "Non-Driving"].iter().any(|m| m.eq_ignore_ascii_case(mode)) {
                    self.issue(Severity::Warning, "add-submodel", format!("[Add Submodel] mode '{mode}' unknown (All, Driving, Non-Driving)"));
                }
                self.unsupported("add_submodel", "Add Submodel", "submodels (dynamic clamp, bus hold) are parsed but not used by kisi");
                let line = self.line;
                self.f.models[i].add_submodel.push(SubmodelRef { name: toks[0].into(), mode: mode.into(), line });
            }
            Cont::SubmodelParams => {
                let Ctx::Submodel(i) = self.ctx else { return };
                let (name, vals) = param(text);
                if name.eq_ignore_ascii_case("Submodel_type") {
                    let t = vals.first().map(|s| s.to_string());
                    if !t.as_deref().is_some_and(|t| ["Dynamic_clamp", "Bus_hold", "Fall_back"].iter().any(|k| k.eq_ignore_ascii_case(t))) {
                        self.issue(Severity::Warning, "submodel", format!("Submodel_type '{}' unknown (Dynamic_clamp, Bus_hold, Fall_back)", t.clone().unwrap_or_default()));
                    }
                    self.f.submodels[i].submodel_type = t;
                } else {
                    self.issue(Severity::Warning, "unknown-parameter", format!("[Submodel] sub-parameter '{name}' unknown: ignored"));
                }
            }
            Cont::PkgPins => {
                let (Ctx::Pkg(i) | Ctx::PkgData(i)) = self.ctx else { return };
                let pm = &mut self.f.package_models[i];
                pm.pins.push(toks[0].into());
                if toks.len() > 1 && !pm.sections {
                    pm.sections = true;
                    self.unsupported("number_of_sections", "Pin Numbers sections", "package model sections are not parsed");
                }
            }
            Cont::Matrix => self.matrix_line(&toks),
        }
    }

    fn package_row(&mut self, toks: &[&str]) {
        let Ctx::Component(i) = self.ctx else { return };
        let what = format!("[Package] {}", toks[0]);
        let key = toks[0].to_ascii_lowercase();
        if !matches!(key.as_str(), "r_pkg" | "l_pkg" | "c_pkg") {
            self.issue(Severity::Warning, "package", format!("[Package] row '{}' unknown (R_pkg, L_pkg, C_pkg): ignored", toks[0]));
            return;
        }
        let v = self.tmm(&toks[1..], &what, true);
        let p = &mut self.f.components[i].package;
        let slot = match key.as_str() {
            "r_pkg" => &mut p.r,
            "l_pkg" => &mut p.l,
            _ => &mut p.c,
        };
        let dup = slot.is_some();
        *slot = v;
        if dup {
            self.issue(Severity::Warning, "duplicate-keyword", format!("{what} given twice (the last one is kept)"));
        }
    }

    fn pin_row(&mut self, toks: &[&str]) {
        let Ctx::Component(i) = self.ctx else { return };
        if toks.len() < 3 {
            self.issue(Severity::Error, "columns", format!("[Pin] row '{}' needs pin name, signal name and model name: ignored", toks.join(" ")));
            return;
        }
        let mut rlc = [None; 3];
        if toks.len() > 3 {
            let cols = match self.pin_cols {
                Some(c) => Some(c),
                None if toks.len() >= 6 => {
                    if !self.pin_assumed_noted {
                        self.pin_assumed_noted = true;
                        self.issue(Severity::Warning, "pin-columns", "[Pin] rows have 6 columns but the keyword line names no R_pin/L_pin/C_pin: read as R, L, C");
                    }
                    Some([3, 4, 5])
                }
                None => None,
            };
            match cols {
                Some(c) => {
                    for (k, col) in c.iter().enumerate() {
                        let what = ["R_pin", "L_pin", "C_pin"][k];
                        if let Some(t) = toks.get(*col) {
                            rlc[k] = self.value(Some(t), &format!("[Pin] {} {what}", toks[0]));
                        }
                    }
                    if toks.len() != 6 {
                        self.issue(Severity::Warning, "columns", format!("[Pin] row '{}' has {} columns, expected 3 or 6", toks[0], toks.len()));
                    }
                }
                None => self.issue(Severity::Warning, "columns", format!("[Pin] row '{}' has {} columns, expected 3 (the keyword line names no R_pin/L_pin/C_pin)", toks[0], toks.len())),
            }
        }
        let line = self.line;
        self.f.components[i].pins.push(Pin { name: toks[0].into(), signal: toks[1].into(), model: toks[2].into(), r: rlc[0], l: rlc[1], c: rlc[2], line });
    }

    fn pin_mapping_row(&mut self, toks: &[&str]) {
        let Ctx::Component(i) = self.ctx else { return };
        if !(3..=6).contains(&toks.len()) {
            self.issue(Severity::Warning, "columns", format!("[Pin Mapping] row has {} columns, expected 3 to 6", toks.len()));
            if toks.len() < 3 {
                return;
            }
        }
        let s = |k: usize| toks.get(k).map(|s| s.to_string());
        let line = self.line;
        self.f.components[i].pin_mapping.push(PinMapping { pin: toks[0].into(), pulldown_ref: toks[1].into(), pullup_ref: toks[2].into(), gnd_clamp_ref: s(3), power_clamp_ref: s(4), ext_ref: s(5), line });
    }

    fn diff_pin_row(&mut self, toks: &[&str]) {
        let Ctx::Component(i) = self.ctx else { return };
        if toks.len() < 2 {
            self.issue(Severity::Error, "columns", "[Diff Pin] row needs at least pin and inv_pin");
            return;
        }
        if toks.len() != 6 {
            self.issue(Severity::Warning, "columns", format!("[Diff Pin] row {} has {} columns, expected 6 (pin inv_pin vdiff tdelay_typ tdelay_min tdelay_max, NA allowed)", toks[0], toks.len()));
        }
        let vdiff = toks.get(2).and_then(|t| self.value(Some(t), "[Diff Pin] vdiff"));
        // tdelay: NA is common (generators write it for "no delay given"), so no finding for it
        let mut tdelay = Tmm::default();
        for (k, t) in toks.iter().skip(3).take(3).enumerate() {
            let v = self.value(Some(t), "[Diff Pin] tdelay");
            match k {
                0 => tdelay.typ = v,
                1 => tdelay.min = v,
                _ => tdelay.max = v,
            }
        }
        let line = self.line;
        self.f.components[i].diff_pins.push(DiffPin { pin: toks[0].into(), inv_pin: toks[1].into(), vdiff, tdelay, line });
    }

    fn model_param(&mut self, text: &str) {
        let Ctx::Model(i) = self.ctx else { return };
        let (name, vals) = param(text);
        let key = name.to_ascii_lowercase();
        self.f.models[i].seen.push((key.clone(), self.line));
        match key.as_str() {
            "model_type" => {
                let t = vals.first().copied().unwrap_or("");
                match canonical_type(t) {
                    Some(c) => self.f.models[i].model_type = Some(c.to_string()),
                    None => self.issue(Severity::Error, "model-type", format!("Model_type '{t}' unknown")),
                }
            }
            "polarity" | "enable" => {
                let t = vals.first().copied().unwrap_or("");
                let ok: &[&str] = if key == "polarity" { &["Inverting", "Non-Inverting"] } else { &["Active-High", "Active-Low"] };
                match ok.iter().find(|o| o.eq_ignore_ascii_case(t)) {
                    Some(o) => {
                        let m = &mut self.f.models[i];
                        if key == "polarity" {
                            m.polarity = Some(o.to_string());
                        } else {
                            m.enable = Some(o.to_string());
                        }
                    }
                    None => self.issue(Severity::Warning, "model-param", format!("{name} '{t}' unknown ({})", ok.join(", "))),
                }
            }
            "vinl" | "vinh" | "vmeas" | "vref" | "rref" | "cref" => {
                if vals.len() > 1 {
                    self.issue(Severity::Warning, "columns", format!("{name} takes one value (typ/min/max belong in [Model Spec])"));
                }
                let v = self.value(vals.first(), name);
                let m = &mut self.f.models[i];
                let slot = match key.as_str() {
                    "vinl" => &mut m.vinl,
                    "vinh" => &mut m.vinh,
                    "vmeas" => &mut m.vmeas,
                    "vref" => &mut m.vref,
                    "rref" => &mut m.rref,
                    _ => &mut m.cref,
                };
                *slot = v;
            }
            "c_comp" | "c_comp_pullup" | "c_comp_pulldown" | "c_comp_power_clamp" | "c_comp_gnd_clamp" => {
                let v = self.tmm(&vals, name, true);
                let m = &mut self.f.models[i];
                let slot = match key.as_str() {
                    "c_comp" => &mut m.c_comp,
                    "c_comp_pullup" => &mut m.c_comp_pullup,
                    "c_comp_pulldown" => &mut m.c_comp_pulldown,
                    "c_comp_power_clamp" => &mut m.c_comp_power_clamp,
                    _ => &mut m.c_comp_gnd_clamp,
                };
                *slot = v;
            }
            _ => match SINGLE_PARAMS.iter().find(|p| p.eq_ignore_ascii_case(name)) {
                Some(p) => {
                    if let Some(v) = self.value(vals.first(), name) {
                        self.f.models[i].params.push((p.to_string(), v));
                    }
                }
                None => self.issue(Severity::Warning, "unknown-parameter", format!("[Model] sub-parameter '{name}' unknown: ignored")),
            },
        }
    }

    fn row(&mut self, toks: &[&str], what: &str) -> Option<Row> {
        let x = match num::parse(toks[0]) {
            Ok(Num::Val { v, .. }) => v,
            Ok(Num::Na) => {
                self.issue(Severity::Error, "bad-row", format!("{what}: NA in the first column (row ignored)"));
                return None;
            }
            Err(e) => {
                self.issue(Severity::Error, "bad-row", format!("{what}: {e} (row ignored)"));
                return None;
            }
        };
        if toks.len() != 4 {
            self.issue(Severity::Warning, "columns", format!("{what}: row has {} columns, expected 4 (x, typ, min, max)", toks.len()));
        }
        if toks.len() < 2 {
            return None;
        }
        let y = self.tmm_row(&toks[1..], what, false, true)?;
        Some(Row { x, y, line: self.line })
    }

    fn table_name(&self, t: Tab) -> String {
        match t {
            Tab::Pulldown => "[Pulldown]".into(),
            Tab::Pullup => "[Pullup]".into(),
            Tab::GndClamp => "[GND Clamp]".into(),
            Tab::PowerClamp => "[POWER Clamp]".into(),
            Tab::Extra(k) => match self.ctx {
                Ctx::Model(i) => format!("[{}]", self.f.models[i].extra_tables.get(k).map(|x| x.0.as_str()).unwrap_or("?")),
                _ => "[?]".into(),
            },
            Tab::SeriesCurrent => "[Series Current]".into(),
            Tab::Mosfet(_) => "[Series MOSFET]".into(),
            Tab::Sub(k) => match self.ctx {
                Ctx::Submodel(i) => format!("[{}]", self.f.submodels[i].tables.get(k).map(|x| x.0.as_str()).unwrap_or("?")),
                _ => "[?]".into(),
            },
        }
    }

    fn table_row(&mut self, t: Tab, toks: &[&str]) {
        if let Tab::Mosfet(k) = t {
            if toks[0].to_ascii_lowercase().starts_with("vds") {
                let joined = toks.join(" ");
                let (_, vals) = param(&joined);
                let v = self.value(vals.first(), "[Series MOSFET] Vds");
                if let Ctx::Model(i) = self.ctx {
                    if let Some(e) = series_mut(&mut self.f.models[i], self.series).mosfet.get_mut(k) {
                        e.0 = v;
                    }
                }
                return;
            }
        }
        let what = self.table_name(t);
        let Some(row) = self.row(toks, &what) else { return };
        if let Some(tab) = self.table_mut(t) {
            tab.rows.push(row);
        }
    }

    fn table_mut(&mut self, t: Tab) -> Option<&mut Table> {
        match self.ctx {
            Ctx::Model(i) => {
                let series = self.series;
                let m = &mut self.f.models[i];
                match t {
                    Tab::Pulldown => m.pulldown.as_mut(),
                    Tab::Pullup => m.pullup.as_mut(),
                    Tab::GndClamp => m.gnd_clamp.as_mut(),
                    Tab::PowerClamp => m.power_clamp.as_mut(),
                    Tab::Extra(k) => m.extra_tables.get_mut(k).map(|x| &mut x.1),
                    Tab::SeriesCurrent => series_mut(m, series).current.as_mut(),
                    Tab::Mosfet(k) => series_mut(m, series).mosfet.get_mut(k).map(|x| &mut x.1),
                    Tab::Sub(_) => None,
                }
            }
            Ctx::Submodel(i) => match t {
                Tab::Sub(k) => self.f.submodels[i].tables.get_mut(k).map(|x| &mut x.1),
                _ => None,
            },
            _ => None,
        }
    }

    fn waveform_mut(&mut self) -> Option<&mut Waveform> {
        match self.ctx {
            Ctx::Model(i) => self.f.models[i].waveforms.last_mut(),
            Ctx::Submodel(i) => self.f.submodels[i].waveforms.last_mut(),
            _ => None,
        }
    }

    fn waveform_line(&mut self, text: &str, toks: &[&str]) {
        let starts_alpha = toks[0].chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        if !starts_alpha {
            let what = if self.waveform_mut().is_some_and(|w| w.rising) { "[Rising Waveform]" } else { "[Falling Waveform]" };
            if let Some(row) = self.row(toks, what) {
                if let Some(w) = self.waveform_mut() {
                    w.table.rows.push(row);
                }
            }
            return;
        }
        let (name, vals) = param(text);
        let key = name.to_ascii_lowercase();
        let known = ["r_fixture", "v_fixture", "v_fixture_min", "v_fixture_max", "l_fixture", "c_fixture", "r_dut", "l_dut", "c_dut"];
        if !known.contains(&key.as_str()) {
            self.issue(Severity::Warning, "unknown-parameter", format!("waveform sub-parameter '{name}' unknown: ignored"));
            return;
        }
        let v = self.value(vals.first(), name);
        if let Some(w) = self.waveform_mut() {
            let slot = match key.as_str() {
                "r_fixture" => &mut w.r_fixture,
                "v_fixture" => &mut w.v_fixture,
                "v_fixture_min" => &mut w.v_fixture_min,
                "v_fixture_max" => &mut w.v_fixture_max,
                "l_fixture" => &mut w.l_fixture,
                "c_fixture" => &mut w.c_fixture,
                "r_dut" => &mut w.r_dut,
                "l_dut" => &mut w.l_dut,
                _ => &mut w.c_dut,
            };
            *slot = v;
        }
    }

    fn ramp_line(&mut self, text: &str) {
        let (name, vals) = param(text);
        let key = name.to_ascii_lowercase();
        match key.as_str() {
            "r_load" => {
                let v = self.value(vals.first(), "[Ramp] R_load");
                if let Some(r) = self.ramp_mut() {
                    r.r_load = v;
                }
            }
            "dv/dt_r" | "dv/dt_f" => {
                if vals.len() != 3 {
                    self.issue(Severity::Warning, "columns", format!("[Ramp] {name}: {} value(s), expected typ, min and max (NA allowed)", vals.len()));
                }
                let mut d = DvdtTmm::default();
                for (k, t) in vals.iter().take(3).enumerate() {
                    let v = if t.eq_ignore_ascii_case("NA") {
                        None
                    } else {
                        match t.split_once('/') {
                            Some((a, b)) => match (num::parse(a), num::parse(b)) {
                                (Ok(Num::Val { v: dv, suspect: s1 }), Ok(Num::Val { v: dt, suspect: s2 })) => {
                                    if let Some(c) = s1.or(s2) {
                                        self.suspect(t, c, &format!("[Ramp] {name}"));
                                    }
                                    Some(Dvdt { dv, dt })
                                }
                                _ => {
                                    self.issue(Severity::Error, "bad-number", format!("[Ramp] {name}: '{t}' is not a dV/dt value"));
                                    None
                                }
                            },
                            None => {
                                self.issue(Severity::Error, "bad-number", format!("[Ramp] {name}: '{t}' is not a dV/dt value (dv/dt)"));
                                None
                            }
                        }
                    };
                    match k {
                        0 => d.typ = v,
                        1 => d.min = v,
                        _ => d.max = v,
                    }
                }
                if d.typ.is_none() && vals.first().is_some_and(|t| t.eq_ignore_ascii_case("NA")) {
                    self.issue(Severity::Error, "na-typ", format!("[Ramp] {name}: the typ value is NA (IBIS requires typ)"));
                }
                let rising = key == "dv/dt_r";
                if let Some(r) = self.ramp_mut() {
                    if rising {
                        r.rising = Some(d);
                    } else {
                        r.falling = Some(d);
                    }
                }
            }
            _ => self.issue(Severity::Warning, "unknown-parameter", format!("[Ramp] sub-parameter '{name}' unknown: ignored")),
        }
    }

    fn ramp_mut(&mut self) -> Option<&mut Ramp> {
        match self.ctx {
            Ctx::Model(i) => self.f.models[i].ramp.as_mut(),
            Ctx::Submodel(i) => self.f.submodels[i].ramp.as_mut(),
            _ => None,
        }
    }

    fn pkg_pin(&self, i: usize, pin: &str) -> Option<usize> {
        let pins = &self.f.package_models[i].pins;
        pins.iter().position(|p| p == pin).or_else(|| pins.iter().position(|p| p.eq_ignore_ascii_case(pin)))
    }

    fn matrix_mut(&mut self, i: usize, w: Mat) -> Option<&mut Matrix> {
        let pm = &mut self.f.package_models[i];
        match w {
            Mat::R => pm.r.as_mut(),
            Mat::L => pm.l.as_mut(),
            Mat::C => pm.c.as_mut(),
        }
    }

    fn matrix_line(&mut self, toks: &[&str]) {
        let (Ctx::Pkg(i) | Ctx::PkgData(i)) = self.ctx else { return };
        let Some((w, Some(row), mut col)) = self.matrix else { return };
        let n = self.f.package_models[i].number_of_pins.unwrap_or(self.f.package_models[i].pins.len());
        let kind = self.matrix_mut(i, w).map(|m| m.kind.clone()).unwrap_or_default();
        let mut new = Vec::new();
        if kind == "sparse_matrix" {
            for pair in toks.chunks(2) {
                let [pin, val] = pair else {
                    self.issue(Severity::Warning, "matrix", "sparse matrix row: pin and value expected in pairs");
                    break;
                };
                match (self.pkg_pin(i, pin), num::value(val)) {
                    (Some(c), Ok(v)) => new.push((row, c, v)),
                    (None, _) => self.issue(Severity::Warning, "matrix", format!("matrix column pin '{pin}' is not in [Pin Numbers]")),
                    (_, Err(e)) => self.issue(Severity::Error, "bad-number", format!("matrix value: {e}")),
                }
            }
        } else {
            let banded = kind == "banded_matrix";
            let bw = self.matrix_mut(i, w).and_then(|m| m.bandwidth);
            for t in toks {
                let c = if banded { row + col } else { col };
                let over = c >= n || banded && bw.is_some_and(|b| col > b);
                if over {
                    self.issue(Severity::Warning, "matrix", "too many values in a matrix row (ignored)");
                    break;
                }
                match num::value(t) {
                    Ok(v) => new.push((row, c, v)),
                    Err(e) => self.issue(Severity::Error, "bad-number", format!("matrix value: {e}")),
                }
                col += 1;
            }
        }
        self.matrix = Some((w, Some(row), col));
        if let Some(m) = self.matrix_mut(i, w) {
            m.entries.extend(new);
        }
    }

    fn finish(&mut self, lines: usize) {
        self.f.lines = lines;
        self.f.header.comment_char = self.comment;
        let line = self.line;
        if self.ctx == Ctx::Start {
            self.f.issues.push(Issue { severity: Severity::Error, line: None, code: "empty", at: None, message: "no IBIS keyword found (not an IBIS file?)".into() });
        } else if !self.f.ended {
            self.f.issues.push(Issue { severity: Severity::Error, line: Some(line), code: "missing-end", at: None, message: "file ends without [END] (truncated?)".into() });
        }
        if let Some(Some(end)) = self.skip {
            self.f.issues.push(Issue { severity: Severity::Warning, line: Some(line), code: "unclosed-block", at: None, message: format!("a skipped block was not closed by [{}]", end.replace('_', " ")) });
        }
        for (_, u) in std::mem::take(&mut self.unsup) {
            let n = if u.count > 1 { format!(" ({}×)", u.count) } else { String::new() };
            self.f.issues.push(Issue { severity: Severity::Warning, line: Some(u.line), code: "unsupported-keyword", at: None, message: format!("{}{n}: {}", u.display, u.why) });
        }
    }
}

fn series_mut(m: &mut Model, s: Series) -> &mut SeriesData {
    match s {
        Series::Main => &mut m.series,
        Series::On => m.on.get_or_insert_with(Default::default),
        Series::Off => m.off.get_or_insert_with(Default::default),
    }
}
