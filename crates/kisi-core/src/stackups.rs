//! Fab stackup templates (`data/stackups.json`): the library, whether a template fits a board
//! (copper layer count, board thickness), and a template as stackup changes – for `--stackup-preset`
//! – or as the `stackup` entry of `<board>.kisi.json` with its sources (the viewer's “Use a fab
//! stackup”, `kisi stackups <board> --preset ID`).

use crate::board::{CopperOverride, DielectricOverride, Stackup, StackupOverride};
use crate::config::{CopperEntry, DielectricEntry, MaskEntry, Source, StackupEntry};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub const STACKUPS_JSON: &str = include_str!("../../../data/stackups.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DataSource {
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PresetDielectric {
    pub kind: String,
    pub material: String,
    /// mm
    pub thickness: f64,
    pub epsilon_r: f64,
    pub loss_tangent: f64,
    /// another published εr of the same material (a second source), for comparisons
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epsilon_r_alt: Option<f64>,
    /// where εr and tanδ come from (assumptions named)
    pub note: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PresetMask {
    pub thickness: f64,
    pub epsilon_r: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Preset {
    pub id: String,
    pub fab: String,
    /// the template's name in the fab's order form
    pub template: String,
    pub title: String,
    pub layers: usize,
    /// nominal board thickness (mm)
    pub nominal_mm: f64,
    /// pressed thickness of the template (mm)
    pub pressed_mm: f64,
    /// finished copper thickness per copper layer, top to bottom (mm)
    pub copper: Vec<f64>,
    pub dielectrics: Vec<PresetDielectric>,
    pub solder_mask: PresetMask,
    pub sources: Vec<DataSource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardThickness {
    pub tolerance_pct: f64,
    pub tolerance_below_1mm_mm: f64,
    pub sources: Vec<DataSource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    pub about: String,
    pub board_thickness: BoardThickness,
    pub presets: Vec<Preset>,
}

/// The built-in library (`data/stackups.json`).
pub fn library() -> &'static Library {
    static L: OnceLock<Library> = OnceLock::new();
    L.get_or_init(|| serde_json::from_str(STACKUPS_JSON).expect("data/stackups.json"))
}

impl Library {
    pub fn get(&self, id: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.id.eq_ignore_ascii_case(id) || p.template.eq_ignore_ascii_case(id))
    }
    /// The preset `id` (or its template name), or an error that lists the library.
    pub fn find(&self, id: &str) -> Result<&Preset, String> {
        self.get(id).ok_or_else(|| {
            format!(
                "unknown stackup preset '{id}' (presets: {}; see `kisi stackups`)",
                self.presets.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")
            )
        })
    }
}

/// Whether a preset fits a board's stackup.
#[derive(Debug, Clone, Serialize)]
pub struct Fit {
    pub ok: bool,
    pub board_layers: usize,
    /// the board's thickness from its stackup (copper and dielectrics), when the stackup is in the file
    pub board_mm: Option<f64>,
    /// why it does not fit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

impl Preset {
    /// The fab's board thickness tolerance around the template's nominal thickness (mm).
    pub fn thickness_tolerance_mm(&self) -> f64 {
        let t = &library().board_thickness;
        if self.nominal_mm >= 1.0 {
            self.nominal_mm * t.tolerance_pct / 100.0
        } else {
            t.tolerance_below_1mm_mm
        }
    }

    /// Does the preset fit the board's stackup: the same number of copper layers, and – when the
    /// stackup is in the board file – a board thickness within the fab's tolerance of the
    /// template's nominal thickness.
    pub fn fit(&self, su: &Stackup) -> Fit {
        let board_mm = su.from_file.then(|| (su.total_thickness() * 1e4).round() / 1e4);
        let mut problem = None;
        if su.n() != self.layers {
            problem = Some(format!(
                "{} has {} copper layers, the board {} – a template applies only to a board with the same layer count",
                self.id,
                self.layers,
                su.n()
            ));
        } else if let Some(b) = board_mm {
            let tol = self.thickness_tolerance_mm();
            if (b - self.nominal_mm).abs() > tol {
                problem = Some(format!(
                    "{} is a {} mm board, the board's stackup is {} mm thick (more than the fab's ±{} mm) – choose a template of the board's thickness or change the thickness in KiCad's stackup",
                    self.id,
                    num(self.nominal_mm),
                    num(b),
                    num(tol)
                ));
            }
        }
        Fit { ok: problem.is_none(), board_layers: su.n(), board_mm, problem }
    }

    /// The preset as stackup changes (all dielectrics, copper and mask). The caller checks `fit`.
    pub fn to_override(&self) -> StackupOverride {
        StackupOverride {
            dielectrics: self
                .dielectrics
                .iter()
                .enumerate()
                .map(|(i, d)| DielectricOverride {
                    index: i,
                    epsilon_r: Some(d.epsilon_r),
                    loss_tangent: Some(d.loss_tangent),
                    thickness: Some(d.thickness),
                    material: Some(d.material.clone()),
                })
                .collect(),
            mask_epsilon_r: Some(self.solder_mask.epsilon_r),
            copper: self
                .copper
                .iter()
                .enumerate()
                .map(|(i, &t)| CopperOverride { index: i, thickness: Some(t) })
                .collect(),
            mask_thickness: Some(self.solder_mask.thickness),
        }
    }

    /// Apply the preset to a stackup: its values replace the board's; the dielectrics' values
    /// count as given (no longer KiCad defaults).
    pub fn apply(&self, su: &Stackup) -> Stackup {
        let mut s = su.with_overrides(&self.to_override());
        for d in s.dielectrics.iter_mut() {
            d.assumed.clear();
        }
        s
    }

    /// The document line of the sources kisi writes for this preset.
    pub fn document(&self) -> String {
        format!("{} impedance template {} (kisi stackup preset {})", self.fab, self.template, self.id)
    }

    fn url(&self, which: &str) -> Option<String> {
        self.sources.iter().filter_map(|s| s.url.clone()).find(|u| u.contains(which))
    }

    /// The preset as the `stackup` entry of `<board>.kisi.json`: every dielectric, every copper
    /// layer and the solder mask, each with a source that names the template and what is assumed.
    pub fn to_entry(&self) -> StackupEntry {
        let doc = self.document();
        let data_url = self.url("raw_json").or_else(|| self.sources.first().and_then(|s| s.url.clone()));
        let caps = self.url("capabilities");
        let guide = self.url("impedance-calculator");
        StackupEntry {
            dielectrics: self
                .dielectrics
                .iter()
                .enumerate()
                .map(|(i, d)| DielectricEntry {
                    index: i,
                    material: Some(d.material.clone()),
                    epsilon_r: Some(d.epsilon_r),
                    loss_tangent: Some(d.loss_tangent),
                    thickness: Some(d.thickness),
                    source: Some(Source {
                        kind: "datasheet".into(),
                        document: Some(doc.clone()),
                        url: data_url.clone(),
                        page: None,
                        note: Some(format!("{} {}: thickness from the template; {}", d.kind, d.material, d.note)),
                    }),
                })
                .collect(),
            copper: self
                .copper
                .iter()
                .enumerate()
                .map(|(i, &t)| CopperEntry {
                    index: i,
                    thickness: Some(t),
                    source: Some(Source {
                        kind: "datasheet".into(),
                        document: Some(doc.clone()),
                        url: caps.clone(),
                        page: None,
                        note: Some(if i == 0 || i + 1 == self.copper.len() {
                            "finished outer copper (1 oz) as in the template".into()
                        } else {
                            "finished inner copper (0.5 oz) as JLC's impedance calculator takes it (0.6 mil)".into()
                        }),
                    }),
                })
                .collect(),
            solder_mask: Some(MaskEntry {
                epsilon_r: Some(self.solder_mask.epsilon_r),
                thickness: Some(self.solder_mask.thickness),
                source: Some(Source {
                    kind: "datasheet".into(),
                    document: Some(doc),
                    url: guide.or(caps),
                    page: None,
                    note: Some("solder mask as the fab's impedance calculator takes it (on laminate and between traces); thinner on copper".into()),
                }),
            }),
        }
    }
}

fn num(v: f64) -> String {
    let s = format!("{:.3}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A preset's summary for `kisi stackups` and the viewer, with its fit to a board's stackup.
pub fn preset_json(p: &Preset, su: Option<&Stackup>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "id": p.id, "fab": p.fab, "template": p.template, "title": p.title, "layers": p.layers,
        "nominal_mm": p.nominal_mm, "pressed_mm": p.pressed_mm,
        "copper_um": p.copper.iter().map(|c| (c * 1e4).round() / 10.0).collect::<Vec<_>>(),
        "dielectrics": p.dielectrics, "solder_mask": p.solder_mask, "sources": p.sources,
    });
    if let Some(su) = su {
        v["fit"] = serde_json::to_value(p.fit(su)).unwrap_or_default();
    }
    v
}

/// `kisi stackups`: the library; with a board, which presets fit it.
pub fn list_json(su: Option<&Stackup>) -> serde_json::Value {
    let lib = library();
    let mut v = serde_json::json!({
        "presets": lib.presets.iter().map(|p| preset_json(p, su)).collect::<Vec<_>>(),
        "about": lib.about,
        "board_thickness_tolerance": {"pct": lib.board_thickness.tolerance_pct, "below_1mm_mm": lib.board_thickness.tolerance_below_1mm_mm, "sources": lib.board_thickness.sources},
    });
    if let Some(su) = su {
        v["board"] = serde_json::json!({"layers": su.n(), "thickness_mm": su.from_file.then(|| (su.total_thickness() * 1e4).round() / 1e4), "stackup_from_file": su.from_file});
        v["fitting"] =
            serde_json::json!(lib.presets.iter().filter(|p| p.fit(su).ok).map(|p| p.id.clone()).collect::<Vec<_>>());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(n: usize, d: f64) -> Stackup {
        use crate::board::{CopperLayer, Dielectric, SolderMask};
        Stackup {
            copper: (0..n).map(|i| CopperLayer { name: format!("L{i}"), thickness: 0.035 }).collect(),
            dielectrics: (0..n - 1)
                .map(|_| Dielectric {
                    name: "d".into(),
                    kind: "core".into(),
                    material: "FR4".into(),
                    thickness: d,
                    epsilon_r: 4.5,
                    loss_tangent: 0.02,
                    assumed: vec!["epsilon_r".into()],
                })
                .collect(),
            mask_top: Some(SolderMask { thickness: 0.01, epsilon_r: 3.3 }),
            mask_bottom: Some(SolderMask { thickness: 0.01, epsilon_r: 3.3 }),
            from_file: true,
        }
    }

    #[test]
    fn library_reads() {
        let l = library();
        assert!(l.presets.len() >= 6);
        for p in &l.presets {
            assert_eq!(p.copper.len(), p.layers, "{}", p.id);
            assert_eq!(p.dielectrics.len() + 1, p.layers, "{}", p.id);
            // the pressed thickness is the sum of the layers (within the template's rounding)
            let sum: f64 = p.copper.iter().sum::<f64>() + p.dielectrics.iter().map(|d| d.thickness).sum::<f64>();
            assert!((sum - p.pressed_mm).abs() < 0.01, "{}: {sum} vs {}", p.id, p.pressed_mm);
            assert!((p.pressed_mm - p.nominal_mm).abs() <= p.thickness_tolerance_mm(), "{}", p.id);
        }
        assert_eq!(l.find("JLC04161H-7628").unwrap().id, "jlc4_7628");
        assert!(l.find("nope").unwrap_err().contains("jlc4_7628"));
    }

    #[test]
    fn fit_and_apply() {
        let p = library().find("jlc4_7628").unwrap();
        // 4 layers, 1.6 mm: fits
        let su = stack(4, (1.6 - 4.0 * 0.035) / 3.0);
        assert!(p.fit(&su).ok, "{:?}", p.fit(&su));
        // a 6-layer board: no
        let f = p.fit(&stack(6, 0.27));
        assert!(!f.ok && f.problem.unwrap().contains("6"));
        // 4 layers but 2.0 mm thick: no
        let f = p.fit(&stack(4, (2.0 - 0.14) / 3.0));
        assert!(!f.ok && f.problem.unwrap().contains("thick"));
        // a default stackup (not in the file): the thickness is not known, the layer count decides
        let mut d = stack(4, 0.6);
        d.from_file = false;
        assert!(p.fit(&d).ok);
        // applied: every value of the template, the defaults no longer assumed
        let a = p.apply(&su);
        assert_eq!(a.dielectrics[0].thickness, 0.2104);
        assert_eq!(a.dielectrics[1].epsilon_r, 4.6);
        assert_eq!(a.dielectrics[1].material, "core 1.1 mm H/HOZ (NP-155F)");
        assert!(a.dielectrics.iter().all(|d| d.assumed.is_empty()));
        assert_eq!(a.copper[1].thickness, 0.0152);
        assert_eq!(a.mask_top.as_ref().unwrap().thickness, 0.0305);
        assert_eq!(a.mask_bottom.as_ref().unwrap().epsilon_r, 3.8);
        assert!((a.total_thickness() - p.pressed_mm).abs() < 1e-6);
    }

    /// The kisi.json entry of a preset passes `config::check` and gives the same stackup as the
    /// preset applied directly.
    #[test]
    fn entry_is_valid_config() {
        let p = library().find("jlc4_3313").unwrap();
        let cfg = crate::config::BoardConfig {
            kisi_config: crate::config::VERSION,
            stackup: p.to_entry(),
            ..Default::default()
        };
        let text = serde_json::to_string(&crate::config::to_json(&cfg)).unwrap();
        let back = crate::config::parse(&text).unwrap();
        assert_eq!(back.stackup.copper.len(), 4);
        let su = stack(4, 0.48);
        let mut board = crate::Board { stackup: Some(su.clone()), ..Default::default() };
        let r =
            crate::config::check(&back, &board, &[], &crate::profile::Profiles::builtin(), None, &Default::default());
        assert!(r.ok, "{:?}", r.errors);
        assert_eq!(r.entries, 3 + 4 + 1);
        back.apply(&mut board);
        let (a, b) = (board.stackup(), p.apply(&su));
        for (x, y) in a.dielectrics.iter().zip(&b.dielectrics) {
            assert_eq!(
                (x.thickness, x.epsilon_r, x.loss_tangent, &x.material),
                (y.thickness, y.epsilon_r, y.loss_tangent, &y.material)
            );
        }
        for (x, y) in a.copper.iter().zip(&b.copper) {
            assert_eq!(x.thickness, y.thickness);
        }
        assert_eq!(a.mask_top.as_ref().unwrap().thickness, 0.0305);
        // the sources name the template, and the assumed tanδ
        let s = back.stackup.dielectrics[0].source.as_ref().unwrap();
        assert!(s.document.as_deref().unwrap().contains("JLC04161H-3313"));
        assert!(s.note.as_deref().unwrap().contains("assumption"));
    }
}
