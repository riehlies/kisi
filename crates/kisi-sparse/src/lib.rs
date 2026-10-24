//! Sparse direct solver used by the upstream quasi-static field solver
//! (js_2d_fields, rectilinear FDM backend).
//!
//! Upstream ships an emscripten build of Eigen as `src/wasm_solver/solver.js`
//! that is not part of its repository. This crate provides the same entry
//! point, `solve_sparse_multi`, built with faer, so the upstream JavaScript
//! runs unmodified (see web/solver/sparse_module.js for the JS shim).
//!
//! Matrix input is CSR (rowPtr, colIdx, values), right-hand sides are stored
//! one after the other. Symmetric matrices are factorized with Cholesky
//! (on A or -A), everything else with LU. Return 0 on success.

use faer::linalg::solvers::Solve;
use faer::sparse::{SparseColMat, Triplet};
use faer::{Mat, Side};

/// Pure-Rust API: solve A X = B for several right-hand sides.
/// `b` holds `n_rhs` vectors of length `n` back to back; returns X the same way.
pub fn solve_csr_multi(
    n: usize,
    row_ptr: &[i32],
    col_idx: &[i32],
    values: &[f64],
    n_rhs: usize,
    b: &[f64],
    force_lu: bool,
) -> Result<Vec<f64>, i32> {
    if row_ptr.len() != n + 1 {
        return Err(12);
    }
    let mut trip = Vec::with_capacity(values.len());
    let mut symmetric = !force_lu;
    for i in 0..n {
        let (s, e) = (row_ptr[i] as usize, row_ptr[i + 1] as usize);
        for p in s..e {
            let j = col_idx[p];
            if j < 0 || j as usize >= n {
                return Err(10);
            }
            trip.push(Triplet::new(i, j as usize, values[p]));
        }
    }
    let a = SparseColMat::<usize, f64>::try_new_from_triplets(n, n, &trip).map_err(|_| 13)?;
    if symmetric {
        symmetric = is_symmetric(n, row_ptr, col_idx, values);
    }
    let mut rhs = Mat::<f64>::zeros(n, n_rhs);
    for k in 0..n_rhs {
        for i in 0..n {
            rhs[(i, k)] = b[k * n + i];
        }
    }
    let x: Mat<f64> = 'solve: {
        if symmetric {
            if let Ok(llt) = a.sp_cholesky(Side::Lower) {
                break 'solve llt.solve(&rhs);
            }
            // negative definite operator (sign convention of the assembly)
            let neg_trip: Vec<_> = trip.iter().map(|t| Triplet::new(t.row, t.col, -t.val)).collect();
            if let Ok(na) = SparseColMat::<usize, f64>::try_new_from_triplets(n, n, &neg_trip) {
                if let Ok(llt) = na.sp_cholesky(Side::Lower) {
                    let mut y = llt.solve(&rhs);
                    for v in y.col_iter_mut() {
                        for e in v.iter_mut() {
                            *e = -*e;
                        }
                    }
                    break 'solve y;
                }
            }
        }
        match a.sp_lu() {
            Ok(lu) => lu.solve(&rhs),
            Err(_) => return Err(20),
        }
    };
    let mut out = vec![0.0; n * n_rhs];
    for k in 0..n_rhs {
        for i in 0..n {
            let v = x[(i, k)];
            if !v.is_finite() {
                return Err(21);
            }
            out[k * n + i] = v;
        }
    }
    Ok(out)
}

fn is_symmetric(n: usize, row_ptr: &[i32], col_idx: &[i32], values: &[f64]) -> bool {
    // CSR rows are sorted by column in the upstream assembly; use binary search
    for i in 0..n {
        for p in row_ptr[i] as usize..row_ptr[i + 1] as usize {
            let j = col_idx[p] as usize;
            if j == i {
                continue;
            }
            let (s, e) = (row_ptr[j] as usize, row_ptr[j + 1] as usize);
            let row = &col_idx[s..e];
            match row.binary_search(&(i as i32)) {
                Ok(q) => {
                    let (x, y) = (values[p], values[s + q]);
                    if (x - y).abs() > 1e-12 * x.abs().max(y.abs()) {
                        return false;
                    }
                }
                Err(_) => {
                    // unsorted row: linear scan
                    match row.iter().position(|&c| c as usize == i) {
                        Some(q) => {
                            let (x, y) = (values[p], values[s + q]);
                            if (x - y).abs() > 1e-12 * x.abs().max(y.abs()) {
                                return false;
                            }
                        }
                        None => return false,
                    }
                }
            }
        }
    }
    true
}

// --- C ABI (emscripten-compatible shape, wrapped by sparse_module.js) ---------

/// Allocation with an 8-byte size header so `ks_free(ptr)` needs no length.
#[no_mangle]
pub extern "C" fn ks_alloc(n: usize) -> *mut u8 {
    let total = n + 8;
    let mut v = Vec::<u64>::with_capacity(total.div_ceil(8));
    let p = v.as_mut_ptr() as *mut u8;
    std::mem::forget(v);
    unsafe {
        *(p as *mut u64) = total as u64;
        p.add(8)
    }
}

#[no_mangle]
pub extern "C" fn ks_free(p: *mut u8) {
    if p.is_null() {
        return;
    }
    unsafe {
        let base = p.sub(8);
        let total = *(base as *mut u64) as usize;
        drop(Vec::<u64>::from_raw_parts(base as *mut u64, 0, total.div_ceil(8)));
    }
}

/// Same signature as upstream's `solve_sparse_multi`.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn ks_solve_sparse_multi(
    n: i32,
    nnz: i32,
    row_ptr: *const i32,
    col_idx: *const i32,
    values: *const f64,
    n_rhs: i32,
    b: *const f64,
    x_out: *mut f64,
    force_lu: i32,
) -> i32 {
    let (n, nnz, nr) = (n as usize, nnz as usize, n_rhs as usize);
    let (rp, ci, va, bb) = unsafe {
        (
            std::slice::from_raw_parts(row_ptr, n + 1),
            std::slice::from_raw_parts(col_idx, nnz),
            std::slice::from_raw_parts(values, nnz),
            std::slice::from_raw_parts(b, n * nr),
        )
    };
    match solve_csr_multi(n, rp, ci, va, nr, bb, force_lu != 0) {
        Ok(x) => {
            unsafe { std::slice::from_raw_parts_mut(x_out, n * nr).copy_from_slice(&x) };
            0
        }
        Err(code) => code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn laplace_1d() {
        // -u'' = 1 on 5 points with Dirichlet ends (as identity rows)
        let n = 5;
        let mut rp = vec![0i32];
        let mut ci = Vec::new();
        let mut va = Vec::new();
        for i in 0..n {
            if i == 0 || i == n - 1 {
                ci.push(i as i32);
                va.push(1.0);
            } else {
                // keep symmetric by not coupling to boundary rows
                if i > 1 {
                    ci.push(i as i32 - 1);
                    va.push(-1.0);
                }
                ci.push(i as i32);
                va.push(2.0);
                if i < n - 2 {
                    ci.push(i as i32 + 1);
                    va.push(-1.0);
                }
            }
            rp.push(ci.len() as i32);
        }
        let b = vec![0.0, 1.0, 1.0, 1.0, 0.0];
        let x = solve_csr_multi(n, &rp, &ci, &va, 1, &b, false).unwrap();
        assert!((x[1] - 1.5).abs() < 1e-12 && (x[2] - 2.0).abs() < 1e-12);
        // negated system goes through the -A path
        let vn: Vec<f64> = va.iter().map(|v| -v).collect();
        let bn: Vec<f64> = b.iter().map(|v| -v).collect();
        let x2 = solve_csr_multi(n, &rp, &ci, &vn, 1, &bn, false).unwrap();
        assert!((x2[2] - 2.0).abs() < 1e-12);
        // LU path
        let x3 = solve_csr_multi(n, &rp, &ci, &va, 1, &b, true).unwrap();
        assert!((x3[2] - 2.0).abs() < 1e-12);
    }
}

/// Solve a symmetric positive definite system given as (row, col, value)
/// triplets (duplicates are summed). Used by the DC IR-drop solver.
pub fn solve_spd_triplets(n: usize, trip: &[(usize, usize, f64)], rhs: &[f64]) -> Result<Vec<f64>, i32> {
    let t: Vec<Triplet<usize, usize, f64>> = trip.iter().map(|&(r, c, v)| Triplet::new(r, c, v)).collect();
    let a = SparseColMat::<usize, f64>::try_new_from_triplets(n, n, &t).map_err(|_| 13)?;
    let mut b = Mat::<f64>::zeros(n, 1);
    for i in 0..n {
        b[(i, 0)] = rhs[i];
    }
    let x = match a.sp_cholesky(Side::Lower) {
        Ok(llt) => llt.solve(&b),
        Err(_) => match a.sp_lu() {
            Ok(lu) => lu.solve(&b),
            Err(_) => return Err(20),
        },
    };
    let out: Vec<f64> = (0..n).map(|i| x[(i, 0)]).collect();
    if out.iter().any(|v| !v.is_finite()) {
        return Err(21);
    }
    Ok(out)
}

/// Symbolic analysis of a sparse LU, reused for matrices with the same pattern.
pub type SymbolicLu = faer::sparse::linalg::solvers::SymbolicLu<usize>;

/// LU factorisation of a complex sparse matrix, for several solves with the same matrix.
/// Used by the AC power-integrity grid.
pub struct ComplexLu {
    n: usize,
    lu: faer::sparse::linalg::solvers::Lu<usize, faer::c64>,
}

impl ComplexLu {
    /// Factor A given as triplets (row, col, re, im; duplicates are summed).
    pub fn new(n: usize, trip: &[(usize, usize, f64, f64)]) -> Result<Self, i32> {
        Self::with_symbolic(n, trip, None).map(|x| x.0)
    }
    /// As `new`, reusing the symbolic analysis of an earlier matrix with the same triplet
    /// positions (frequency sweeps); returns it for the next call.
    pub fn with_symbolic(
        n: usize,
        trip: &[(usize, usize, f64, f64)],
        sym: Option<faer::sparse::linalg::solvers::SymbolicLu<usize>>,
    ) -> Result<(Self, faer::sparse::linalg::solvers::SymbolicLu<usize>), i32> {
        use faer::c64;
        use faer::sparse::linalg::solvers::{Lu, SymbolicLu};
        let t: Vec<Triplet<usize, usize, c64>> = trip.iter().map(|&(r, c, re, im)| Triplet::new(r, c, c64::new(re, im))).collect();
        let a = SparseColMat::<usize, c64>::try_new_from_triplets(n, n, &t).map_err(|_| 13)?;
        let sym = match sym {
            Some(s) => s,
            None => SymbolicLu::try_new(a.symbolic()).map_err(|_| 20)?,
        };
        let lu = Lu::try_new_with_symbolic(sym.clone(), a.as_ref()).map_err(|_| 20)?;
        Ok((ComplexLu { n, lu }, sym))
    }
    /// Solve for the right-hand sides `b[k]` (complex as (re, im)).
    pub fn solve(&self, b: &[Vec<(f64, f64)>]) -> Result<Vec<Vec<(f64, f64)>>, i32> {
        use faer::c64;
        let n = self.n;
        let mut rhs = Mat::<c64>::zeros(n, b.len());
        for (k, v) in b.iter().enumerate() {
            for i in 0..n {
                rhs[(i, k)] = c64::new(v[i].0, v[i].1);
            }
        }
        let x = self.lu.solve(&rhs);
        let mut out = Vec::with_capacity(b.len());
        for k in 0..b.len() {
            let col: Vec<(f64, f64)> = (0..n).map(|i| (x[(i, k)].re, x[(i, k)].im)).collect();
            if col.iter().any(|v| !v.0.is_finite() || !v.1.is_finite()) {
                return Err(21);
            }
            out.push(col);
        }
        Ok(out)
    }
}

/// The sparsity pattern of a complex matrix assembled from triplets at fixed positions, for
/// sweeps where only the values change (frequency sweeps of a grid): the column-compressed
/// structure and the slot of every triplet are found once, so a new matrix is a sum into its
/// values instead of a sort of the triplets.
pub struct ComplexPattern {
    n: usize,
    col_ptr: Vec<usize>,
    row_idx: Vec<usize>,
    /// slot in the compressed values of every triplet, in the triplets' order
    slot: Vec<usize>,
}

impl ComplexPattern {
    /// the pattern of an n×n matrix with triplets at `pos` (row, column; repeats are summed)
    pub fn new(n: usize, pos: &[(usize, usize)]) -> Result<Self, i32> {
        let mut order: Vec<usize> = (0..pos.len()).collect();
        if pos.iter().any(|&(r, c)| r >= n || c >= n) {
            return Err(10);
        }
        order.sort_by_key(|&k| (pos[k].1, pos[k].0, k));
        let mut col_ptr = vec![0usize; n + 1];
        let mut row_idx: Vec<usize> = Vec::with_capacity(pos.len());
        let mut slot = vec![0usize; pos.len()];
        let mut last: Option<(usize, usize)> = None;
        for &k in &order {
            let (r, c) = pos[k];
            if last != Some((r, c)) {
                row_idx.push(r);
                col_ptr[c + 1] += 1;
                last = Some((r, c));
            }
            slot[k] = row_idx.len() - 1;
        }
        for c in 0..n {
            col_ptr[c + 1] += col_ptr[c];
        }
        Ok(ComplexPattern { n, col_ptr, row_idx, slot })
    }

    /// LU of the matrix with the triplet values `vals` (re, im, in the order of the positions),
    /// reusing the symbolic analysis of an earlier call; returns it for the next one. The repeats
    /// of a position are summed in the triplets' order.
    pub fn lu(&self, vals: &[(f64, f64)], sym: Option<SymbolicLu>) -> Result<(ComplexLu, SymbolicLu), i32> {
        use faer::c64;
        use faer::sparse::linalg::solvers::Lu;
        use faer::sparse::{SparseColMatRef, SymbolicSparseColMatRef};
        if vals.len() != self.slot.len() {
            return Err(11);
        }
        let mut v = vec![c64::new(0.0, 0.0); self.row_idx.len()];
        for (k, &(re, im)) in vals.iter().enumerate() {
            let q = &mut v[self.slot[k]];
            *q = *q + c64::new(re, im);
        }
        let symb = SymbolicSparseColMatRef::new_checked(self.n, self.n, &self.col_ptr, None, &self.row_idx);
        let a = SparseColMatRef::new(symb, &v);
        let sym = match sym {
            Some(s) => s,
            None => SymbolicLu::try_new(symb).map_err(|_| 20)?,
        };
        let lu = Lu::try_new_with_symbolic(sym.clone(), a).map_err(|_| 20)?;
        Ok((ComplexLu { n: self.n, lu }, sym))
    }
}

/// Solve a complex sparse system A X = B (LU) given as triplets (row, col, re, im;
/// duplicates are summed) for several right-hand sides.
pub fn solve_complex_multi(n: usize, trip: &[(usize, usize, f64, f64)], b: &[Vec<(f64, f64)>]) -> Result<Vec<Vec<(f64, f64)>>, i32> {
    ComplexLu::new(n, trip)?.solve(b)
}
