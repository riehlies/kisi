//! Net names in the power fields (PI_REG, PI_LOAD, PI_SOURCE, PI_TARGET).
//!
//! A field value is split at separators (`;` `,` `=` `@` `>` and whitespace). A net name that
//! contains one of them – hierarchical names such as `/06 Peripherals/IMU_VDD` – is written in
//! double quotes: `"/06 Peripherals/IMU_VDD"`, also with a voltage (`"/Power/VDD CORE"@0.9V`)
//! or a value (`"/06 Peripherals/IMU_VDD"=0.9mA`). Separators inside quotes are part of the
//! name; values without quotes are read as before.

/// Split at any of `seps` outside double quotes; pieces are trimmed, empty pieces dropped.
pub fn split_unquoted<'a>(s: &'a str, seps: &[char]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && seps.contains(&c) {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&s[start..]);
    out.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// Whitespace-separated tokens, a quoted name being part of one token.
pub fn tokens(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (None, false);
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

/// Byte position of the first (`first`) or last `sep` outside quotes.
fn find_unquoted(s: &str, sep: char, first: bool) -> Option<usize> {
    let mut quoted = false;
    let mut found = None;
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && c == sep {
            if first {
                return Some(i);
            }
            found = Some(i);
        }
    }
    found
}

/// Split once at the first `sep` outside quotes (both sides trimmed).
pub fn split_once_unquoted(s: &str, sep: char) -> Option<(&str, &str)> {
    find_unquoted(s, sep, true).map(|i| (s[..i].trim(), s[i + sep.len_utf8()..].trim()))
}

/// Split once at the last `sep` outside quotes (both sides trimmed).
pub fn rsplit_once_unquoted(s: &str, sep: char) -> Option<(&str, &str)> {
    find_unquoted(s, sep, false).map(|i| (s[..i].trim(), s[i + sep.len_utf8()..].trim()))
}

/// Split a PI_REG entry at its arrow (`>`, `->` or `→`) outside quotes: (input side, output side).
pub fn split_arrow(s: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && (c == '>' || c == '→') {
            let lhs = &s[..i];
            let lhs = lhs.strip_suffix('-').filter(|_| c == '>').unwrap_or(lhs);
            return Some((lhs.trim(), s[i + c.len_utf8()..].trim()));
        }
    }
    None
}

/// A net name as written in a field, without its quotes (`"A B"` → `A B`, `A` → `A`).
pub fn unquote(s: &str) -> String {
    let t = s.trim();
    match t.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        Some(inner) if t.len() >= 2 => inner.to_string(),
        _ => t.to_string(),
    }
}

/// A rail as written in PI_REG: `NAME`, `NAME@1.8V`, `"NAME"` or `"NAME"@1.8V` → (name, voltage).
/// An `@` that is not followed by a voltage stays part of the name.
pub fn rail_token(t: &str) -> (String, Option<f64>) {
    if let Some((n, v)) = rsplit_once_unquoted(t, '@') {
        if let Ok(x) = v.trim_end_matches(['V', 'v']).replace(',', ".").parse::<f64>() {
            return (unquote(n), Some(x));
        }
    }
    (unquote(t), None)
}

/// A net name for a field value: in quotes when it contains whitespace or a separator.
pub fn quote_name(n: &str) -> String {
    if n.chars().any(|c| c.is_whitespace() || matches!(c, '"' | ';' | ',' | '=' | '@' | '>' | '→')) {
        format!("\"{n}\"")
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(split_unquoted("A=1mA, \"/x y/B,C\"=2mA; D=3mA", &[',', ';']), vec!["A=1mA", "\"/x y/B,C\"=2mA", "D=3mA"]);
        assert_eq!(tokens("\"/06 Peripherals/IMU_VDD\"@3.3V  pass r=0.5"), vec!["\"/06 Peripherals/IMU_VDD\"@3.3V", "pass", "r=0.5"]);
        assert_eq!(split_once_unquoted("\"a=b c\"=2mA@L1", '='), Some(("\"a=b c\"", "2mA@L1")));
        assert_eq!(split_arrow("+5V -> \"/P/a>b\" ldo"), Some(("+5V", "\"/P/a>b\" ldo")));
        assert_eq!(split_arrow("\"/a b/x\" → +3V3 pass"), Some(("\"/a b/x\"", "+3V3 pass")));
        assert_eq!(split_arrow("+12V input"), None);
        assert_eq!(rail_token("\"/06 Peripherals/IMU_VDD\""), ("/06 Peripherals/IMU_VDD".to_string(), None));
        assert_eq!(rail_token("\"/P/VDD CORE\"@0.9V"), ("/P/VDD CORE".to_string(), Some(0.9)));
        assert_eq!(rail_token("VCORE@0,9V"), ("VCORE".to_string(), Some(0.9)));
        assert_eq!(rail_token("\"N@1\"@1.8V"), ("N@1".to_string(), Some(1.8)));
        assert_eq!(rail_token("U11-NC@P10"), ("U11-NC@P10".to_string(), None));
        assert_eq!(unquote("\""), "\"");
        assert_eq!(quote_name("/06 Peripherals/IMU_VDD"), "\"/06 Peripherals/IMU_VDD\"");
        assert_eq!(quote_name("IMU&MIC_3V3"), "IMU&MIC_3V3");
    }
}
