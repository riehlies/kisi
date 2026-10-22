//! The contents of an IBIS file as data (no interpretation beyond units).

use crate::num::{Corner, Tmm};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// the file breaks a rule of the specification; the data may be unusable
    Error,
    /// suspicious or incomplete data, or a keyword kisi does not use
    Warning,
    /// information
    Note,
}

/// A finding of the parser or the checks, with the line it refers to.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// stable short id, e.g. "unknown-keyword", "non-monotonic", "waveform-endpoint"
    pub code: &'static str,
    /// where: "component X", "model Y", "header", …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub message: String,
}

/// One row of a table: x (voltage or time) and the typ/min/max values.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub x: f64,
    pub y: Tmm,
    pub line: usize,
}

/// I-V table (x = voltage, y = current into the buffer) or V-t / I-t table (x = time).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Table {
    /// line of the keyword
    pub line: usize,
    pub rows: Vec<Row>,
}

impl Table {
    /// Linear interpolation of a corner (NA in min/max: typ); held constant beyond the ends.
    pub fn at(&self, x: f64, c: Corner) -> Option<f64> {
        let pts: Vec<(f64, f64)> = self.rows.iter().filter_map(|r| r.y.get(c).map(|y| (r.x, y))).collect();
        let (first, last) = (pts.first()?, pts.last()?);
        if x <= first.0 {
            return Some(first.1);
        }
        if x >= last.0 {
            return Some(last.1);
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if x >= a.0 && x <= b.0 {
                return Some(if b.0 > a.0 { a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0) } else { b.1 });
            }
        }
        Some(last.1)
    }
    /// Range of the x column.
    pub fn x_range(&self) -> Option<(f64, f64)> {
        let lo = self.rows.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
        let hi = self.rows.iter().map(|r| r.x).fold(f64::NEG_INFINITY, f64::max);
        (lo <= hi).then_some((lo, hi))
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    /// Whether the min / max columns hold values (not only NA).
    pub fn has_corner(&self, c: Corner) -> bool {
        match c {
            Corner::Typ => !self.rows.is_empty(),
            Corner::Min => self.rows.iter().any(|r| r.y.min.is_some()),
            Corner::Max => self.rows.iter().any(|r| r.y.max.is_some()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Header {
    /// as written, e.g. "5.1"
    pub ibis_ver: Option<String>,
    pub ibis_ver_line: Option<usize>,
    pub file_name: Option<String>,
    pub file_rev: Option<String>,
    pub date: Option<String>,
    pub source: Option<String>,
    pub notes: Option<String>,
    pub disclaimer: Option<String>,
    pub copyright: Option<String>,
    /// comment character in use at the end of the file
    pub comment_char: char,
}

impl Header {
    /// [IBIS Ver] as a number.
    pub fn version(&self) -> Option<f64> {
        self.ibis_ver.as_deref().and_then(|v| v.trim().parse().ok())
    }
}

/// [Package]: default R/L/C of the pins (used where [Pin] gives none or NA).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Package {
    pub line: Option<usize>,
    pub r: Option<Tmm>,
    pub l: Option<Tmm>,
    pub c: Option<Tmm>,
}

/// A row of [Pin].
#[derive(Debug, Clone, Serialize)]
pub struct Pin {
    pub name: String,
    pub signal: String,
    /// a [Model], a [Model Selector] or a reserved word (POWER, GND, NC, CIRCUITCALL)
    pub model: String,
    /// R_pin, L_pin, C_pin (None: not given or NA – the [Package] value applies)
    pub r: Option<f64>,
    pub l: Option<f64>,
    pub c: Option<f64>,
    pub line: usize,
}

impl Pin {
    /// POWER, GND, NC or CIRCUITCALL: no buffer model.
    pub fn reserved(&self) -> Option<&'static str> {
        RESERVED_MODELS.iter().find(|r| self.model.eq_ignore_ascii_case(r)).copied()
    }
}

pub const RESERVED_MODELS: &[&str] = &["POWER", "GND", "NC", "CIRCUITCALL"];

/// A row of [Pin Mapping] (supply buses a buffer refers to), kept as data.
#[derive(Debug, Clone, Serialize)]
pub struct PinMapping {
    pub pin: String,
    pub pulldown_ref: String,
    pub pullup_ref: String,
    pub gnd_clamp_ref: Option<String>,
    pub power_clamp_ref: Option<String>,
    pub ext_ref: Option<String>,
    pub line: usize,
}

/// A row of [Diff Pin]: non-inverting and inverting pin, receiver threshold, driver skew.
#[derive(Debug, Clone, Serialize)]
pub struct DiffPin {
    pub pin: String,
    pub inv_pin: String,
    /// differential receiver threshold in V (None: NA / not given)
    pub vdiff: Option<f64>,
    /// delay of the inverting output in s, typ/min/max
    pub tdelay: Tmm,
    pub line: usize,
}

/// A row of [Series Pin Mapping].
#[derive(Debug, Clone, Serialize)]
pub struct SeriesPin {
    pub pin1: String,
    pub pin2: String,
    pub model: String,
    pub group: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Component {
    pub name: String,
    pub line: usize,
    pub manufacturer: Option<String>,
    pub package: Package,
    pub pins: Vec<Pin>,
    /// [Pin] has R_pin / L_pin / C_pin columns
    pub pin_rlc_columns: bool,
    /// [Package Model] name (a [Define Package Model] here or in a .pkg file)
    pub package_model: Option<String>,
    pub pin_mapping: Vec<PinMapping>,
    pub diff_pins: Vec<DiffPin>,
    pub series_pins: Vec<SeriesPin>,
    pub si_location: Option<String>,
    pub timing_location: Option<String>,
}

impl Component {
    /// A pin by name (exact, then ignoring case).
    pub fn pin(&self, name: &str) -> Option<&Pin> {
        self.pins.iter().find(|p| p.name == name).or_else(|| self.pins.iter().find(|p| p.name.eq_ignore_ascii_case(name)))
    }
    /// R, L, C of a pin: the pin's own values, else the [Package] values; `true` = from [Pin].
    pub fn pin_rlc(&self, p: &Pin, c: Corner) -> [(Option<f64>, bool); 3] {
        let pk = |t: &Option<Tmm>| t.as_ref().and_then(|t| t.get(c));
        [
            p.r.map(|v| (Some(v), true)).unwrap_or((pk(&self.package.r), false)),
            p.l.map(|v| (Some(v), true)).unwrap_or((pk(&self.package.l), false)),
            p.c.map(|v| (Some(v), true)).unwrap_or((pk(&self.package.c), false)),
        ]
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectorEntry {
    pub model: String,
    pub description: String,
    pub line: usize,
}

/// [Model Selector]: alternatives for a pin (drive strength, ODT, I/O standard).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelSelector {
    pub name: String,
    pub line: usize,
    pub entries: Vec<SelectorEntry>,
}

/// dV/dt of [Ramp]: the 20 %–80 % voltage change and the time it takes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Dvdt {
    pub dv: f64,
    pub dt: f64,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DvdtTmm {
    pub typ: Option<Dvdt>,
    pub min: Option<Dvdt>,
    pub max: Option<Dvdt>,
}

impl DvdtTmm {
    pub fn get(&self, c: Corner) -> Option<Dvdt> {
        match c {
            Corner::Typ => self.typ,
            Corner::Min => self.min.or(self.typ),
            Corner::Max => self.max.or(self.typ),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Ramp {
    pub line: usize,
    /// R_load in Ω (None: not given, IBIS default 50 Ω)
    pub r_load: Option<f64>,
    pub rising: Option<DvdtTmm>,
    pub falling: Option<DvdtTmm>,
}

impl Ramp {
    pub const DEFAULT_R_LOAD: f64 = 50.0;
    pub fn r_load(&self) -> f64 {
        self.r_load.unwrap_or(Self::DEFAULT_R_LOAD)
    }
}

/// [Rising Waveform] / [Falling Waveform]: V-t table into a test fixture.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Waveform {
    pub line: usize,
    pub rising: bool,
    pub r_fixture: Option<f64>,
    pub v_fixture: Option<f64>,
    pub v_fixture_min: Option<f64>,
    pub v_fixture_max: Option<f64>,
    pub l_fixture: Option<f64>,
    pub c_fixture: Option<f64>,
    pub r_dut: Option<f64>,
    pub l_dut: Option<f64>,
    pub c_dut: Option<f64>,
    pub table: Table,
    /// [Composite Current] that follows the waveform (power-aware data, not used by kisi)
    pub composite_current: Option<Table>,
}

impl Waveform {
    /// V_fixture of a corner (V_fixture_min / _max, else V_fixture).
    pub fn v_fixture_at(&self, c: Corner) -> Option<f64> {
        match c {
            Corner::Typ => self.v_fixture,
            Corner::Min => self.v_fixture_min.or(self.v_fixture),
            Corner::Max => self.v_fixture_max.or(self.v_fixture),
        }
    }
}

/// A named typ/min/max value ([Model Spec], [Submodel Spec], [TTgnd], …).
#[derive(Debug, Clone, Serialize)]
pub struct NamedTmm {
    pub name: String,
    pub value: Tmm,
    pub line: usize,
}

/// A [Receiver Thresholds] entry: `Vth = 0.675V` or `Reference_supply Pullup_ref`.
#[derive(Debug, Clone, Serialize)]
pub struct RxParam {
    pub name: String,
    pub value: Option<f64>,
    pub text: String,
    pub line: usize,
}

/// A row of [Driver Schedule] (multi-stage drivers), kept as data.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleRow {
    pub model: String,
    pub rise_on: Option<f64>,
    pub rise_off: Option<f64>,
    pub fall_on: Option<f64>,
    pub fall_off: Option<f64>,
    pub line: usize,
}

/// [Add Submodel] entry.
#[derive(Debug, Clone, Serialize)]
pub struct SubmodelRef {
    pub name: String,
    pub mode: String,
    pub line: usize,
}

/// Series elements of a Series / Series_switch model (in [Model], [On] or [Off]).
#[derive(Debug, Clone, Default, Serialize)]
pub struct SeriesData {
    pub r: Option<Tmm>,
    pub l: Option<Tmm>,
    pub c: Option<Tmm>,
    pub rl: Option<Tmm>,
    pub rc: Option<Tmm>,
    pub lc: Option<Tmm>,
    pub current: Option<Table>,
    /// [Series MOSFET]: Vds and the I-V table
    pub mosfet: Vec<(Option<f64>, Table)>,
}

impl SeriesData {
    pub fn is_empty(&self) -> bool {
        self.r.is_none() && self.l.is_none() && self.c.is_none() && self.rl.is_none() && self.rc.is_none() && self.lc.is_none() && self.current.is_none() && self.mosfet.is_empty()
    }
}

/// [Model]: one buffer.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Model {
    pub name: String,
    pub line: usize,
    /// canonical spelling, e.g. "I/O", "3-state", "Open_drain" (None: missing or unknown)
    pub model_type: Option<String>,
    pub polarity: Option<String>,
    pub enable: Option<String>,
    pub vinl: Option<f64>,
    pub vinh: Option<f64>,
    pub vmeas: Option<f64>,
    pub vref: Option<f64>,
    pub rref: Option<f64>,
    pub cref: Option<f64>,
    /// other single-value sub-parameters (Rref_diff, Cref_rising, Vmeas_falling, …)
    pub params: Vec<(String, f64)>,
    pub c_comp: Option<Tmm>,
    pub c_comp_pullup: Option<Tmm>,
    pub c_comp_pulldown: Option<Tmm>,
    pub c_comp_power_clamp: Option<Tmm>,
    pub c_comp_gnd_clamp: Option<Tmm>,
    pub voltage_range: Option<Tmm>,
    pub temperature_range: Option<Tmm>,
    pub pullup_ref: Option<Tmm>,
    pub pulldown_ref: Option<Tmm>,
    pub power_clamp_ref: Option<Tmm>,
    pub gnd_clamp_ref: Option<Tmm>,
    pub ext_ref: Option<Tmm>,
    pub pulldown: Option<Table>,
    pub pullup: Option<Table>,
    pub gnd_clamp: Option<Table>,
    pub power_clamp: Option<Table>,
    /// terminator elements
    pub rgnd: Option<Tmm>,
    pub rpower: Option<Tmm>,
    pub rac: Option<Tmm>,
    pub cac: Option<Tmm>,
    pub ramp: Option<Ramp>,
    pub waveforms: Vec<Waveform>,
    pub model_spec: Vec<NamedTmm>,
    pub receiver_thresholds: Vec<RxParam>,
    pub driver_schedule: Vec<ScheduleRow>,
    pub add_submodel: Vec<SubmodelRef>,
    pub series: SeriesData,
    pub on: Option<SeriesData>,
    pub off: Option<SeriesData>,
    /// tables kisi keeps but does not use: [ISSO PU], [ISSO PD]
    pub extra_tables: Vec<(String, Table)>,
    /// values kisi keeps but does not use: [TTgnd], [TTpower]
    pub extra_values: Vec<NamedTmm>,
    /// line of each keyword / sub-parameter seen (for the checks)
    #[serde(skip)]
    pub seen: Vec<(String, usize)>,
}

impl Model {
    pub fn kind(&self) -> ModelKind {
        ModelKind::of(self.model_type.as_deref())
    }
    /// C_comp of a corner: C_comp, else the sum of the C_comp_* parts.
    pub fn c_comp_at(&self, c: Corner) -> Option<f64> {
        if let Some(v) = self.c_comp.as_ref().and_then(|t| t.get(c)) {
            return Some(v);
        }
        let parts: Vec<f64> = [&self.c_comp_pullup, &self.c_comp_pulldown, &self.c_comp_power_clamp, &self.c_comp_gnd_clamp].iter().filter_map(|t| t.as_ref().and_then(|t| t.get(c))).collect();
        (!parts.is_empty()).then(|| parts.iter().sum())
    }
    pub fn rising(&self) -> impl Iterator<Item = &Waveform> {
        self.waveforms.iter().filter(|w| w.rising)
    }
    pub fn falling(&self) -> impl Iterator<Item = &Waveform> {
        self.waveforms.iter().filter(|w| !w.rising)
    }
    pub fn line_of(&self, what: &str) -> Option<usize> {
        self.seen.iter().find(|(k, _)| k == what).map(|(_, l)| *l)
    }
    /// A [Model Spec] value.
    pub fn spec(&self, name: &str) -> Option<&Tmm> {
        self.model_spec.iter().find(|s| s.name.eq_ignore_ascii_case(name)).map(|s| &s.value)
    }
}

/// What a Model_type means for the checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelKind {
    pub known: bool,
    /// receives (thresholds apply)
    pub input: bool,
    /// drives: [Ramp] and pullup/pulldown apply
    pub output: bool,
    pub has_pullup: bool,
    pub has_pulldown: bool,
    pub ecl: bool,
    pub terminator: bool,
    pub series: bool,
    pub series_switch: bool,
}

/// Model_type values (IBIS 1.1–7.x), canonical spelling.
pub const MODEL_TYPES: &[&str] = &[
    "Input", "Output", "I/O", "3-state", "Open_drain", "I/O_open_drain", "Open_sink", "I/O_open_sink", "Open_source", "I/O_open_source", "Input_ECL", "Output_ECL", "I/O_ECL", "3-state_ECL",
    "Terminator", "Series", "Series_switch", "Input_diff", "Output_diff", "I/O_diff", "3-state_diff",
];

impl ModelKind {
    pub fn of(t: Option<&str>) -> ModelKind {
        let t = t.unwrap_or("").to_ascii_lowercase();
        let known = MODEL_TYPES.iter().any(|m| m.eq_ignore_ascii_case(&t));
        let input = known && (t.starts_with("input") || t.starts_with("i/o"));
        let output = known && (t.starts_with("output") || t.starts_with("3-state") || t.starts_with("i/o") || t.starts_with("open_"));
        let open_low = t.contains("open_drain") || t.contains("open_sink");
        let open_high = t.contains("open_source");
        ModelKind {
            known,
            input,
            output,
            has_pullup: output && !open_low,
            has_pulldown: output && !open_high,
            ecl: t.contains("ecl"),
            terminator: t == "terminator",
            series: t == "series",
            series_switch: t == "series_switch",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Submodel {
    pub name: String,
    pub line: usize,
    pub submodel_type: Option<String>,
    pub spec: Vec<NamedTmm>,
    pub tables: Vec<(String, Table)>,
    pub waveforms: Vec<Waveform>,
    pub ramp: Option<Ramp>,
}

/// A matrix of [Define Package Model]: entries (row pin, column pin, value).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Matrix {
    pub line: usize,
    /// Full_matrix, Banded_matrix or Sparse_matrix
    pub kind: String,
    pub bandwidth: Option<usize>,
    pub entries: Vec<(usize, usize, f64)>,
}

impl Matrix {
    pub fn diagonal(&self, i: usize) -> Option<f64> {
        self.entries.iter().find(|e| e.0 == i && e.1 == i).map(|e| e.2)
    }
}

/// [Define Package Model] … [End Package Model].
#[derive(Debug, Clone, Default, Serialize)]
pub struct PackageModel {
    pub name: String,
    pub line: usize,
    pub manufacturer: Option<String>,
    pub oem: Option<String>,
    pub description: Option<String>,
    pub number_of_pins: Option<usize>,
    pub pins: Vec<String>,
    /// [Pin Numbers] with section descriptions (Len=… L=… /): not parsed
    pub sections: bool,
    pub r: Option<Matrix>,
    pub l: Option<Matrix>,
    pub c: Option<Matrix>,
}

/// A parsed IBIS file.
#[derive(Debug, Clone, Default, Serialize)]
pub struct IbisFile {
    pub header: Header,
    pub components: Vec<Component>,
    pub model_selectors: Vec<ModelSelector>,
    pub models: Vec<Model>,
    pub submodels: Vec<Submodel>,
    pub package_models: Vec<PackageModel>,
    /// [END] found
    pub ended: bool,
    pub lines: usize,
    /// parser findings, then the checks (see `check`)
    pub issues: Vec<Issue>,
}

impl IbisFile {
    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name).or_else(|| self.components.iter().find(|c| c.name.eq_ignore_ascii_case(name)))
    }
    pub fn model(&self, name: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.name == name).or_else(|| self.models.iter().find(|m| m.name.eq_ignore_ascii_case(name)))
    }
    pub fn selector(&self, name: &str) -> Option<&ModelSelector> {
        self.model_selectors.iter().find(|m| m.name == name).or_else(|| self.model_selectors.iter().find(|m| m.name.eq_ignore_ascii_case(name)))
    }
    pub fn package_model(&self, name: &str) -> Option<&PackageModel> {
        self.package_models.iter().find(|m| m.name == name).or_else(|| self.package_models.iter().find(|m| m.name.eq_ignore_ascii_case(name)))
    }
    pub fn count(&self, s: Severity) -> usize {
        self.issues.iter().filter(|i| i.severity == s).count()
    }
}
