//! Small numeric helpers shared by the SI and PI crates: complex numbers and
//! physical constants (SI units).

use std::f64::consts::PI;

pub const C0: f64 = 299_792_458.0;
pub const MU0: f64 = 4.0e-7 * PI;
pub const EPS0: f64 = 8.854_187_812_8e-12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cx {
    pub re: f64,
    pub im: f64,
}

impl Cx {
    pub const fn new(re: f64, im: f64) -> Self {
        Cx { re, im }
    }
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }
    pub fn sqrt(self) -> Cx {
        let r = self.abs().sqrt();
        let a = self.arg() / 2.0;
        Cx::new(r * a.cos(), r * a.sin())
    }
    pub fn exp(self) -> Cx {
        let e = self.re.exp();
        Cx::new(e * self.im.cos(), e * self.im.sin())
    }
    pub fn inv(self) -> Cx {
        let d = self.re * self.re + self.im * self.im;
        Cx::new(self.re / d, -self.im / d)
    }
    pub fn scale(self, k: f64) -> Cx {
        Cx::new(self.re * k, self.im * k)
    }
}
impl std::ops::Add for Cx {
    type Output = Cx;
    fn add(self, o: Cx) -> Cx {
        Cx::new(self.re + o.re, self.im + o.im)
    }
}
impl std::ops::Sub for Cx {
    type Output = Cx;
    fn sub(self, o: Cx) -> Cx {
        Cx::new(self.re - o.re, self.im - o.im)
    }
}
impl std::ops::Mul for Cx {
    type Output = Cx;
    fn mul(self, o: Cx) -> Cx {
        Cx::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}
impl std::ops::Div for Cx {
    type Output = Cx;
    // a / b as a · (1/b), deliberately (the results are compared digit for digit)
    #[allow(clippy::suspicious_arithmetic_impl)]
    fn div(self, o: Cx) -> Cx {
        self * o.inv()
    }
}
