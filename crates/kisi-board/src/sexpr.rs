//! Minimal, fast S-expression reader for KiCad files.
//!
//! KiCad board files can be tens of megabytes, most of it zone fill points.
//! The reader therefore
//!   * parses `(pts (xy ..) ..)` lists straight into a point vector,
//!   * lets the caller skip uninteresting top-level items without building a tree.

#[derive(Debug, Clone)]
pub enum Node {
    List(Vec<Node>),
    Sym(String),
    Str(String),
    Pts(Vec<[f64; 2]>),
}

impl Node {
    pub fn items(&self) -> &[Node] {
        match self {
            Node::List(v) => v,
            _ => &[],
        }
    }
    /// Head symbol of a list, e.g. `segment` for `(segment ...)`.
    pub fn head(&self) -> Option<&str> {
        match self.items().first() {
            Some(Node::Sym(s)) => Some(s),
            _ => None,
        }
    }
    /// First child list with the given head.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.items().iter().skip(1).find(|n| n.head() == Some(name))
    }
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.items().iter().skip(1).filter(move |n| n.head() == Some(name))
    }
    /// Atom (symbol or string) at position `i` (0 = head).
    pub fn atom(&self, i: usize) -> Option<&str> {
        match self.items().get(i) {
            Some(Node::Sym(s)) | Some(Node::Str(s)) => Some(s),
            _ => None,
        }
    }
    pub fn f64(&self, i: usize) -> Option<f64> {
        self.atom(i)?.parse().ok()
    }
    /// `(name v)` -> v as f64
    pub fn child_f64(&self, name: &str) -> Option<f64> {
        self.child(name)?.f64(1)
    }
    pub fn child_str(&self, name: &str) -> Option<&str> {
        self.child(name)?.atom(1)
    }
    /// `(name x y)` -> [x, y]
    pub fn child_xy(&self, name: &str) -> Option<[f64; 2]> {
        let c = self.child(name)?;
        Some([c.f64(1)?, c.f64(2)?])
    }
    pub fn pts(&self) -> Option<&[[f64; 2]]> {
        for c in self.items() {
            if let Node::Pts(p) = c {
                return Some(p);
            }
        }
        None
    }
}

pub struct Reader<'a> {
    s: &'a [u8],
    pub i: usize,
}

#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'a> Reader<'a> {
    pub fn new(s: &'a str) -> Self {
        Reader { s: s.as_bytes(), i: 0 }
    }

    fn err<T>(&self, msg: &str) -> Result<T, ParseError> {
        Err(ParseError(format!("{} (byte {})", msg, self.i)))
    }

    pub fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    pub fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s.get(self.i).copied()
    }

    pub fn expect(&mut self, c: u8) -> Result<(), ParseError> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            self.err(&format!("expected '{}'", c as char))
        }
    }

    pub fn read_symbol(&mut self) -> &'a str {
        let st = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_whitespace() || c == b'(' || c == b')' {
                break;
            }
            self.i += 1;
        }
        // KiCad files are UTF-8; symbols are ASCII in practice.
        std::str::from_utf8(&self.s[st..self.i]).unwrap_or("")
    }

    fn read_string(&mut self) -> Result<String, ParseError> {
        // at opening quote
        self.i += 1;
        let mut out = Vec::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            match c {
                b'"' => return Ok(String::from_utf8_lossy(&out).into_owned()),
                b'\\' if self.i < self.s.len() => {
                    let e = self.s[self.i];
                    self.i += 1;
                    out.push(match e {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        other => other,
                    });
                }
                _ => out.push(c),
            }
        }
        self.err("unterminated string")
    }

    /// Reads the head symbol of a list whose '(' was already consumed,
    /// without consuming anything else.
    pub fn peek_head(&mut self) -> &'a str {
        self.skip_ws();
        let save = self.i;
        let h = self.read_symbol();
        self.i = save;
        h
    }

    /// Skip the rest of a list whose '(' was already consumed.
    pub fn skip_rest(&mut self) -> Result<(), ParseError> {
        let mut depth = 1usize;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            match c {
                b'"' => {
                    self.read_string()?;
                    continue;
                }
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        return Ok(());
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        self.err("unbalanced parentheses")
    }

    /// Parse the rest of a list whose '(' was already consumed.
    pub fn parse_rest(&mut self) -> Result<Node, ParseError> {
        let head = self.read_symbol();
        if head == "pts" {
            return self.parse_pts_rest();
        }
        let mut items = vec![Node::Sym(head.to_string())];
        loop {
            match self.peek() {
                None => return self.err("unexpected end of file"),
                Some(b')') => {
                    self.i += 1;
                    return Ok(Node::List(items));
                }
                Some(b'(') => {
                    self.i += 1;
                    items.push(self.parse_rest()?);
                }
                Some(b'"') => items.push(Node::Str(self.read_string()?)),
                Some(_) => items.push(Node::Sym(self.read_symbol().to_string())),
            }
        }
    }

    /// `(pts (xy x y) (xy x y) ... )` -> Node::Pts. Arc entries inside pts
    /// (`(arc (start)(mid)(end))`, used in board outlines) are tessellated.
    fn parse_pts_rest(&mut self) -> Result<Node, ParseError> {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        loop {
            match self.peek() {
                None => return self.err("unexpected end of file in pts"),
                Some(b')') => {
                    self.i += 1;
                    return Ok(Node::Pts(pts));
                }
                Some(b'(') => {
                    self.i += 1;
                    let h = self.peek_head();
                    if h == "xy" {
                        self.read_symbol();
                        self.skip_ws();
                        let x: f64 = self.read_symbol().parse().unwrap_or(0.0);
                        self.skip_ws();
                        let y: f64 = self.read_symbol().parse().unwrap_or(0.0);
                        self.expect(b')')?;
                        pts.push([x, y]);
                    } else {
                        let n = self.parse_rest()?;
                        if h == "arc" {
                            if let (Some(a), Some(m), Some(b)) =
                                (n.child_xy("start"), n.child_xy("mid"), n.child_xy("end"))
                            {
                                pts.extend(crate::geom::tessellate_arc(a, m, b, 10f64.to_radians()));
                            }
                        }
                    }
                }
                Some(_) => {
                    self.read_symbol();
                }
            }
        }
    }
}

/// Parse a whole S-expression document fully (used for small files and tests).
pub fn parse_all(s: &str) -> Result<Node, ParseError> {
    let mut r = Reader::new(s);
    r.expect(b'(')?;
    r.parse_rest()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic() {
        let n = parse_all(r#"(a (b 1 2) (c "x \"y\"") (pts (xy 1 2) (xy 3 4)))"#).unwrap();
        assert_eq!(n.head(), Some("a"));
        assert_eq!(n.child("b").unwrap().f64(2), Some(2.0));
        assert_eq!(n.child_str("c"), Some("x \"y\""));
        assert_eq!(n.pts().unwrap().len(), 2);
    }
}
