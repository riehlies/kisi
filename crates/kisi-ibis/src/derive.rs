//! Values derived from a model's tables (not values of the file): on-resistances at Vcc/2,
//! the receiver/off-state resistance of the clamps, DC levels into a fixture, edge times.
//!
//! IBIS conventions: currents flow into the buffer; [Pulldown] and [GND Clamp] voltages are
//! pin voltages relative to their reference (pulldown / GND clamp reference, default 0 V);
//! [Pullup] and [POWER Clamp] voltages are the reference minus the pin voltage.

use crate::num::Corner;
use crate::types::*;

/// Reference voltages of a model in one corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rails {
    pub pullup: f64,
    pub pulldown: f64,
    pub power_clamp: f64,
    pub gnd_clamp: f64,
}

impl Rails {
    /// Supply span of the driver (Vcc for a buffer referenced to ground).
    pub fn span(&self) -> f64 {
        self.pullup - self.pulldown
    }
}

/// The references of a corner: the …_Reference keywords, else [Voltage Range] and 0 V.
pub fn rails(m: &Model, c: Corner) -> Option<Rails> {
    let get = |t: &Option<crate::num::Tmm>| t.as_ref().and_then(|t| t.get(c));
    let vr = get(&m.voltage_range);
    let pullup = get(&m.pullup_ref).or(vr)?;
    Some(Rails { pullup, pulldown: get(&m.pulldown_ref).unwrap_or(0.0), power_clamp: get(&m.power_clamp_ref).or(vr).unwrap_or(pullup), gnd_clamp: get(&m.gnd_clamp_ref).unwrap_or(0.0) })
}

/// Driver state for DC solutions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    High,
    Low,
    Off,
}

fn at(t: &Option<Table>, x: f64, c: Corner) -> f64 {
    t.as_ref().and_then(|t| t.at(x, c)).unwrap_or(0.0)
}

/// Current of the clamps into the pin at pin voltage `v`.
pub fn clamp_current(m: &Model, c: Corner, r: &Rails, v: f64) -> f64 {
    at(&m.gnd_clamp, v - r.gnd_clamp, c) + at(&m.power_clamp, r.power_clamp - v, c)
}

/// Current into the pin at pin voltage `v` (driver tables of the state plus clamps).
pub fn buffer_current(m: &Model, c: Corner, r: &Rails, v: f64, s: State) -> f64 {
    clamp_current(m, c, r, v)
        + match s {
            State::High => at(&m.pullup, r.pullup - v, c),
            State::Low => at(&m.pulldown, v - r.pulldown, c),
            State::Off => 0.0,
        }
}

/// Large-signal resistance of [Pulldown] at Vcc/2: (Vcc/2) / I.
pub fn ron_pulldown(m: &Model, c: Corner) -> Option<f64> {
    let r = rails(m, c)?;
    let half = r.span() / 2.0;
    let i = m.pulldown.as_ref()?.at(half, c)?;
    (half > 0.0 && i > 1e-12).then(|| half / i)
}

/// Large-signal resistance of [Pullup] at Vcc/2: (Vcc/2) / −I.
pub fn ron_pullup(m: &Model, c: Corner) -> Option<f64> {
    let r = rails(m, c)?;
    let half = r.span() / 2.0;
    let i = m.pullup.as_ref()?.at(half, c)?;
    (half > 0.0 && -i > 1e-12).then(|| half / -i)
}

/// Small-signal resistance of the clamps at Vcc/2: the input resistance in the receiving or
/// high-impedance state (finite when on-die termination is modelled in the clamp tables).
/// `Some(None)`: clamps present but no measurable conductance (> 1 GΩ).
pub fn roff(m: &Model, c: Corner) -> Option<Option<f64>> {
    if m.gnd_clamp.is_none() && m.power_clamp.is_none() {
        return None;
    }
    let r = rails(m, c)?;
    let mid = (r.pullup + r.pulldown) / 2.0;
    let dv = (0.01 * r.span().abs()).max(1e-3);
    let g = (clamp_current(m, c, &r, mid + dv) - clamp_current(m, c, &r, mid - dv)) / (2.0 * dv);
    Some((g > 1e-9).then(|| 1.0 / g))
}

/// DC pin voltage of a state into a fixture R to V (bisection; None if not bracketed).
pub fn dc_level(m: &Model, c: Corner, s: State, r_fix: f64, v_fix: f64) -> Option<f64> {
    if r_fix <= 0.0 {
        return None;
    }
    let r = rails(m, c)?;
    let f = |v: f64| buffer_current(m, c, &r, v, s) - (v_fix - v) / r_fix;
    let span = r.span().abs().max(1.0);
    let (mut lo, mut hi) = (r.pulldown.min(r.gnd_clamp).min(v_fix) - 2.0 * span, r.pullup.max(r.power_clamp).max(v_fix) + 2.0 * span);
    if f(lo) > 0.0 || f(hi) < 0.0 {
        return None;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if f(mid) > 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Start, end and 20–80 % time of a waveform table in one corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub t_start: f64,
    pub t_end: f64,
    pub v_start: f64,
    pub v_end: f64,
    /// 20 %–80 % of the swing (None: no swing or no crossing)
    pub t20_80: Option<f64>,
}

pub fn edge(w: &Waveform, c: Corner) -> Option<Edge> {
    let pts: Vec<(f64, f64)> = w.table.rows.iter().filter_map(|r| r.y.get(c).map(|y| (r.x, y))).collect();
    let (a, b) = (*pts.first()?, *pts.last()?);
    if pts.len() < 2 {
        return None;
    }
    let swing = b.1 - a.1;
    let cross = |level: f64| -> Option<f64> {
        for w in pts.windows(2) {
            let (p, q) = (w[0], w[1]);
            let hit = if swing > 0.0 { p.1 < level && q.1 >= level } else { p.1 > level && q.1 <= level };
            if hit {
                return Some(p.0 + (q.0 - p.0) * (level - p.1) / (q.1 - p.1));
            }
        }
        None
    };
    let t20_80 = if swing.abs() > 1e-9 {
        match (cross(a.1 + 0.2 * swing), cross(a.1 + 0.8 * swing)) {
            (Some(t2), Some(t8)) if t8 >= t2 => Some(t8 - t2),
            _ => None,
        }
    } else {
        None
    };
    Some(Edge { t_start: a.0, t_end: b.0, v_start: a.1, v_end: b.1, t20_80 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::num::Tmm;

    fn table(rows: &[(f64, f64)]) -> Option<Table> {
        Some(Table { line: 1, rows: rows.iter().map(|&(x, y)| Row { x, y: Tmm { typ: Some(y), min: None, max: None }, line: 1 }).collect() })
    }

    /// A linear 50 Ω push-pull buffer on 3.3 V with an ideal 100 Ω clamp pair.
    fn linear() -> Model {
        let v = [-3.3, 0.0, 3.3, 6.6];
        Model {
            voltage_range: Some(Tmm::new(3.3, 3.0, 3.6)),
            pulldown: table(&v.map(|x| (x, x / 50.0))),
            pullup: table(&v.map(|x| (x, -x / 50.0))),
            gnd_clamp: table(&v.map(|x| (x, x / 200.0))),
            power_clamp: table(&v.map(|x| (x, -x / 200.0))),
            ..Default::default()
        }
    }

    #[test]
    fn resistances_and_levels() {
        let m = linear();
        assert!((ron_pulldown(&m, Corner::Typ).unwrap() - 50.0).abs() < 1e-9);
        assert!((ron_pullup(&m, Corner::Typ).unwrap() - 50.0).abs() < 1e-9);
        // two 200 Ω clamps in parallel at Vcc/2
        assert!((roff(&m, Corner::Typ).unwrap().unwrap() - 100.0).abs() < 1e-6);
        // high state into 50 Ω to ground: 50 Ω pullup ∥ clamps (Thevenin 100 Ω to Vcc/2)
        let v = dc_level(&m, Corner::Typ, State::High, 50.0, 0.0).unwrap();
        // node equation: (3.3−v)/50 + (1.65−v)/100 = v/50 → v = (3.3/50 + 1.65/100)/(2/50 + 1/100)
        let want = (3.3 / 50.0 + 1.65 / 100.0) / (2.0 / 50.0 + 1.0 / 100.0);
        assert!((v - want).abs() < 1e-9, "{v} vs {want}");
        let mut no_clamps = linear();
        no_clamps.gnd_clamp = None;
        no_clamps.power_clamp = None;
        assert_eq!(roff(&no_clamps, Corner::Typ), None);
        assert!((dc_level(&no_clamps, Corner::Typ, State::Low, 50.0, 3.3).unwrap() - 1.65).abs() < 1e-9);
    }

    #[test]
    fn edges() {
        let w = Waveform { table: table(&[(0.0, 0.0), (1e-9, 0.0), (2e-9, 1.0), (3e-9, 1.0)]).unwrap(), rising: true, ..Default::default() };
        let e = edge(&w, Corner::Typ).unwrap();
        assert!((e.t20_80.unwrap() - 0.6e-9).abs() < 1e-15);
        assert_eq!((e.v_start, e.v_end), (0.0, 1.0));
    }
}
