//! Numbers in IBIS files: scaling factors, "NA", typ/min/max triples.
//!
//! IBIS reads a number, then looks at only one alphabetic character after it: one of the
//! scaling factors T G M k m u n p f multiplies, any other letter (a unit such as V, A, F,
//! Ohm, C) is ignored. "NA" means "not available". Case matters for the factors: `M` is
//! mega, `m` milli.

use serde::Serialize;

/// Scaling factor of a suffix character, if it is one.
pub fn scale(c: char) -> Option<f64> {
    Some(match c {
        'T' => 1e12,
        'G' => 1e9,
        'M' => 1e6,
        'k' => 1e3,
        'm' => 1e-3,
        'u' => 1e-6,
        'n' => 1e-9,
        'p' => 1e-12,
        'f' => 1e-15,
        _ => return None,
    })
}

/// A parsed number token.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    /// "NA": not available
    Na,
    /// the value; `suspect` is a letter after it that looks like a scaling factor in the wrong
    /// case (K, U, N, P): IBIS ignores it, which is rarely what the author meant
    Val { v: f64, suspect: Option<char> },
}

/// Parse one token: `1.5`, `-2e-3`, `0.8V`, `1.0pF`, `5mm`, `1.0Meg`, `NA`.
pub fn parse(tok: &str) -> Result<Num, String> {
    let t = tok.trim();
    if t.eq_ignore_ascii_case("NA") {
        return Ok(Num::Na);
    }
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return Err(format!("'{t}' is not a number"));
    }
    // exponent only when digits follow (else the 'e' is a suffix letter)
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let k = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > k {
            i = j;
        }
    }
    let mut v: f64 = t[..i].parse().map_err(|_| format!("'{t}' is not a number"))?;
    let rest = &t[i..];
    let mut suspect = None;
    if let Some(c) = rest.chars().next() {
        if !c.is_ascii_alphabetic() {
            return Err(format!("'{t}' is not a number"));
        }
        match scale(c) {
            Some(f) => v *= f,
            None if matches!(c, 'K' | 'U' | 'N' | 'P') => suspect = Some(c),
            None => {}
        }
    }
    if !v.is_finite() {
        return Err(format!("'{t}' is not a finite number"));
    }
    Ok(Num::Val { v, suspect })
}

/// Parse a token that must be a number (NA is an error here).
pub fn value(tok: &str) -> Result<f64, String> {
    match parse(tok)? {
        Num::Na => Err("NA where a value is required".into()),
        Num::Val { v, .. } => Ok(v),
    }
}

/// Corner of a typ/min/max value. In IBIS, "min" and "max" are the slow and fast corners
/// for tables (weak/strong buffer) and the minimum/maximum for single values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Corner {
    Typ,
    Min,
    Max,
}

impl Corner {
    pub const ALL: [Corner; 3] = [Corner::Typ, Corner::Min, Corner::Max];
    pub fn parse(s: &str) -> Option<Corner> {
        match s.trim().to_ascii_lowercase().as_str() {
            "typ" => Some(Corner::Typ),
            "min" => Some(Corner::Min),
            "max" => Some(Corner::Max),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Corner::Typ => "typ",
            Corner::Min => "min",
            Corner::Max => "max",
        }
    }
}

/// A typ/min/max triple; `None` = NA (or not given).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Tmm {
    pub typ: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Tmm {
    pub fn new(typ: f64, min: f64, max: f64) -> Tmm {
        Tmm { typ: Some(typ), min: Some(min), max: Some(max) }
    }
    /// The value of a corner; NA in min or max falls back to typ (as simulators do).
    pub fn get(&self, c: Corner) -> Option<f64> {
        match c {
            Corner::Typ => self.typ,
            Corner::Min => self.min.or(self.typ),
            Corner::Max => self.max.or(self.typ),
        }
    }
    pub fn map(&self, f: impl Fn(f64) -> f64) -> Tmm {
        Tmm { typ: self.typ.map(&f), min: self.min.map(&f), max: self.max.map(&f) }
    }
    pub fn values(&self) -> impl Iterator<Item = f64> + '_ {
        [self.typ, self.min, self.max].into_iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(t: &str) -> f64 {
        value(t).unwrap()
    }

    #[test]
    fn numbers_and_suffixes() {
        assert_eq!(v("1.5"), 1.5);
        assert_eq!(v("-2e-3"), -2e-3);
        assert_eq!(v("+.5"), 0.5);
        assert!((v("0.8V") - 0.8).abs() < 1e-15);
        assert!((v("1.0pF") - 1e-12).abs() < 1e-24);
        assert!((v("5mm") - 5e-3).abs() < 1e-15);
        assert!((v("1.0Meg") - 1e6).abs() < 1e-6);
        assert!((v("2.2k") - 2200.0).abs() < 1e-9);
        assert!((v("3G") - 3e9).abs() < 1.0);
        assert!((v("1T") - 1e12).abs() < 1.0);
        assert!((v("4u") - 4e-6).abs() < 1e-18);
        assert!((v("7f") - 7e-15).abs() < 1e-27);
        assert!((v("1e-3n") - 1e-12).abs() < 1e-24);
        assert!((v("1.49nH") - 1.49e-9).abs() < 1e-20);
        assert_eq!(v("50C"), 50.0);
        assert_eq!(v("25.0Ohm"), 25.0);
        assert_eq!(v("2e"), 2.0);
        assert_eq!(parse("NA").unwrap(), Num::Na);
        assert_eq!(parse("na").unwrap(), Num::Na);
        assert_eq!(parse("4.7K").unwrap(), Num::Val { v: 4.7, suspect: Some('K') });
        for bad in ["", "abc", "-", ".", "1.2.3", "1/2", "e5", "1,5"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(value("NA").is_err());
    }

    #[test]
    fn corners() {
        let t = Tmm { typ: Some(1.0), min: None, max: Some(3.0) };
        assert_eq!((t.get(Corner::Typ), t.get(Corner::Min), t.get(Corner::Max)), (Some(1.0), Some(1.0), Some(3.0)));
        assert_eq!(Corner::parse(" MAX "), Some(Corner::Max));
        assert_eq!(t.values().count(), 2);
    }
}
