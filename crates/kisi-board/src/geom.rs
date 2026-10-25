//! Small 2D geometry helpers. Units: millimetres, KiCad orientation (y down).

pub type P = [f64; 2];

#[inline]
pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
#[inline]
pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
#[inline]
pub fn scale(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s]
}
#[inline]
pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
#[inline]
pub fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
#[inline]
pub fn len(a: P) -> f64 {
    dot(a, a).sqrt()
}
#[inline]
pub fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Circle through three points; None if (nearly) collinear.
pub fn circle_3p(a: P, b: P, c: P) -> Option<(P, f64)> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d.abs() < 1e-12 {
        return None;
    }
    let a2 = dot(a, a);
    let b2 = dot(b, b);
    let c2 = dot(c, c);
    let ux = (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d;
    let uy = (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d;
    let ctr = [ux, uy];
    Some((ctr, len(sub(a, ctr))))
}

/// Points along the arc start -> mid -> end (inclusive of both ends).
pub fn tessellate_arc(a: P, m: P, b: P, max_step: f64) -> Vec<P> {
    let Some((c, r)) = circle_3p(a, m, b) else {
        return vec![a, b];
    };
    let ang = |p: P| (p[1] - c[1]).atan2(p[0] - c[0]);
    let a0 = ang(a);
    let am = ang(m);
    let a1 = ang(b);
    let tau = std::f64::consts::TAU;
    let norm = |x: f64| ((x % tau) + tau) % tau;
    // sweep from a0 to a1 passing through am, in either direction
    let ccw = norm(am - a0) <= norm(a1 - a0);
    let sweep = if ccw { norm(a1 - a0) } else { -norm(a0 - a1) };
    let n = ((sweep.abs() / max_step).ceil() as usize).max(1);
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = a0 + sweep * (i as f64) / (n as f64);
        out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
    }
    out[0] = a;
    out[n] = b;
    out
}

#[derive(Debug, Clone, Copy)]
pub struct BBox {
    pub min: P,
    pub max: P,
}

impl BBox {
    pub fn empty() -> Self {
        BBox { min: [f64::INFINITY; 2], max: [f64::NEG_INFINITY; 2] }
    }
    pub fn add(&mut self, p: P) {
        self.min[0] = self.min[0].min(p[0]);
        self.min[1] = self.min[1].min(p[1]);
        self.max[0] = self.max[0].max(p[0]);
        self.max[1] = self.max[1].max(p[1]);
    }
    pub fn is_valid(&self) -> bool {
        self.min[0] <= self.max[0]
    }
    pub fn of(pts: &[P]) -> Self {
        let mut b = BBox::empty();
        for &p in pts {
            b.add(p);
        }
        b
    }
}

/// Intersection of ray p + t*d (t >= 0) with segment a-b. Returns t.
#[inline]
pub fn ray_seg(p: P, d: P, a: P, b: P) -> Option<f64> {
    let e = sub(b, a);
    let den = cross(d, e);
    if den.abs() < 1e-15 {
        return None;
    }
    let ap = sub(a, p);
    let t = cross(ap, e) / den;
    let u = cross(ap, d) / den;
    if t >= 0.0 && (-1e-12..=1.0 + 1e-12).contains(&u) {
        Some(t)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arc_quarter() {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let pts = tessellate_arc([1.0, 0.0], [s, s], [0.0, 1.0], 0.1);
        assert!(pts.len() > 5);
        for p in &pts {
            assert!((len(*p) - 1.0).abs() < 1e-6);
        }
        // must pass through upper-right quadrant
        assert!(pts[pts.len() / 2][0] > 0.5);
    }
    #[test]
    fn ray() {
        let t = ray_seg([0.0, 0.0], [1.0, 0.0], [2.0, -1.0], [2.0, 1.0]).unwrap();
        assert!((t - 2.0).abs() < 1e-12);
    }
}
