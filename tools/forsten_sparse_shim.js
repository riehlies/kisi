// Test shim (not part of upstream): pure-JS replacement for the emscripten
// sparse solver so the quasi-static backend runs in node without emcc.
// Implements _malloc/_free/HEAP views and _solve_sparse_multi via
// Jacobi-preconditioned CG (symmetric) or BiCGSTAB (otherwise).
export default async function createWASMModule() {
  let buffer = new ArrayBuffer(1 << 24);
  let top = 16, live = 0;
  const M = {};
  const refresh = () => { M.HEAP32 = new Int32Array(buffer); M.HEAPF64 = new Float64Array(buffer); };
  refresh();
  M._malloc = (n) => {
    const p = (top + 7) & ~7; top = p + n; live++;
    if (top > buffer.byteLength) {
      let sz = buffer.byteLength; while (sz < top) sz *= 2;
      const nb = new ArrayBuffer(sz); new Uint8Array(nb).set(new Uint8Array(buffer)); buffer = nb; refresh();
    }
    return p;
  };
  M._free = () => { live--; if (live <= 0) { live = 0; top = 16; } };
  M._solve_sparse_multi = (N, nnz, pRow, pCol, pVal, nRhs, pB, pX, forceLU) => {
    const row = new Int32Array(buffer, pRow, N + 1), col = new Int32Array(buffer, pCol, nnz);
    const val = new Float64Array(buffer, pVal, nnz);
    const B = new Float64Array(buffer, pB, N * nRhs), X = new Float64Array(buffer, pX, N * nRhs);
    const diag = new Float64Array(N);
    for (let i = 0; i < N; i++) for (let p = row[i]; p < row[i + 1]; p++) if (col[p] === i) diag[i] = val[p];
    for (let i = 0; i < N; i++) if (diag[i] === 0) diag[i] = 1;
    const mv = (x, y) => { for (let i = 0; i < N; i++) { let s = 0; for (let p = row[i]; p < row[i + 1]; p++) s += val[p] * x[col[p]]; y[i] = s; } };
    // symmetry check (sampled exhaustive via map of transposed entries)
    let sym = true;
    outer: for (let i = 0; i < N; i++) for (let p = row[i]; p < row[i + 1]; p++) {
      const j = col[p]; let found = false;
      for (let q = row[j]; q < row[j + 1]; q++) if (col[q] === i) { found = Math.abs(val[q] - val[p]) <= 1e-12 * Math.abs(val[p]) + 1e-300; break; }
      if (!found) { sym = false; break outer; }
    }
    const dotf = (a, b) => { let s = 0; for (let i = 0; i < N; i++) s += a[i] * b[i]; return s; };
    for (let k = 0; k < nRhs; k++) {
      const b = B.subarray(k * N, (k + 1) * N);
      const x = new Float64Array(N);
      const bn = Math.sqrt(dotf(b, b)) || 1;
      const tol = 1e-12 * bn, maxit = 50000;
      const r = Float64Array.from(b), t = new Float64Array(N);
      if (sym && !forceLU) {
        const z = new Float64Array(N), pv = new Float64Array(N), Ap = new Float64Array(N);
        for (let i = 0; i < N; i++) { z[i] = r[i] / diag[i]; pv[i] = z[i]; }
        let rz = dotf(r, z);
        for (let it = 0; it < maxit; it++) {
          mv(pv, Ap); const a = rz / dotf(pv, Ap);
          for (let i = 0; i < N; i++) { x[i] += a * pv[i]; r[i] -= a * Ap[i]; }
          if (Math.sqrt(dotf(r, r)) < tol) break;
          for (let i = 0; i < N; i++) z[i] = r[i] / diag[i];
          const rz2 = dotf(r, z); const beta = rz2 / rz; rz = rz2;
          for (let i = 0; i < N; i++) pv[i] = z[i] + beta * pv[i];
        }
      } else {
        // Jacobi-preconditioned BiCGSTAB
        const r0 = Float64Array.from(r), p = new Float64Array(N), v = new Float64Array(N),
          y = new Float64Array(N), s = new Float64Array(N), zz = new Float64Array(N);
        let rho = 1, alpha = 1, omega = 1;
        for (let it = 0; it < maxit; it++) {
          const rho1 = dotf(r0, r); const beta = (rho1 / rho) * (alpha / omega); rho = rho1;
          for (let i = 0; i < N; i++) p[i] = r[i] + beta * (p[i] - omega * v[i]);
          for (let i = 0; i < N; i++) y[i] = p[i] / diag[i];
          mv(y, v); alpha = rho / dotf(r0, v);
          for (let i = 0; i < N; i++) s[i] = r[i] - alpha * v[i];
          for (let i = 0; i < N; i++) zz[i] = s[i] / diag[i];
          mv(zz, t); omega = dotf(t, s) / dotf(t, t);
          for (let i = 0; i < N; i++) { x[i] += alpha * y[i] + omega * zz[i]; r[i] = s[i] - omega * t[i]; }
          if (Math.sqrt(dotf(r, r)) < tol) break;
        }
      }
      X.set(x, k * N);
    }
    return 0;
  };
  return M;
}
