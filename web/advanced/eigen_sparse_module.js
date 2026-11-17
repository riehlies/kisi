// kisi-advanced.html without the full-wave WebAssembly modules: a stand-in for the original's
// wasm_solver/eigen_solver.js (Eigen + Spectra, emscripten, not in its git) with the two linear solves the
// quasi-static path calls, backed by kisi-sparse (web/solver/sparse_module.js, the module kisi.html already ships):
//   _solve_sparse_multi      real sparse solve, as sparse_module.js
//   _solve_complex_symmetric (Ar + j Ai) x = b as the real block system [Ar -Ai; Ai Ar] [xr; xi] = [br; bi],
//                            solved by kisi-sparse's LU (the quasi-static plot fields under a complex permittivity,
//                            field_solver.js _solve_laplace_complex; the original uses an LDL^T / complex LU there)
// The eigenproblem of the full-wave backend (_solve_generalized) is not offered: the Full-wave choice needs the
// real module (tools/advanced_bundle.py --fw).
// './solver.js' is the bundle's path of the original's wasm_solver/solver.js, i.e. kisi-sparse (sparse_module.js)
import createSparse from './solver.js';

export default async function createEigenStandIn() {
    const S = await createSparse();
    const i32 = () => S.HEAP32, f64 = () => S.HEAPF64;
    function solveComplexSymmetric(n, nnz, pR, pC, pVr, pVi, nRhs, pRhs, pX) {
        const rp = i32().slice(pR >> 2, (pR >> 2) + n + 1), ci = i32().slice(pC >> 2, (pC >> 2) + nnz);
        const vr = f64().slice(pVr >> 3, (pVr >> 3) + nnz), vi = f64().slice(pVi >> 3, (pVi >> 3) + nnz);
        // block CSR of size 2n: row r < n: [Ar | -Ai], row n + r: [Ai | Ar]
        const N2 = 2 * n, rp2 = new Int32Array(N2 + 1), ci2 = new Int32Array(4 * nnz), v2 = new Float64Array(4 * nnz);
        let k = 0;
        for (let half = 0; half < 2; half++) {
            for (let r = 0; r < n; r++) {
                rp2[half * n + r] = k;
                for (const off of [0, n]) {          // column blocks in ascending order
                    for (let q = rp[r]; q < rp[r + 1]; q++) {
                        ci2[k] = ci[q] + off;
                        v2[k] = half === 0 ? (off === 0 ? vr[q] : -vi[q]) : (off === 0 ? vi[q] : vr[q]);
                        k++;
                    }
                }
            }
        }
        rp2[N2] = k;
        const alloc = (arr, bytes) => { const p = S._malloc(bytes * arr.length); (bytes === 4 ? new Int32Array(S.HEAP32.buffer, p, arr.length) : new Float64Array(S.HEAPF64.buffer, p, arr.length)).set(arr); return p; };
        // the right-hand sides are already packed as [re (n), im (n)] per RHS, the block system's layout
        const rhs = f64().slice(pRhs >> 3, (pRhs >> 3) + nRhs * N2);
        const ptrs = [alloc(rp2, 4), alloc(ci2, 4), alloc(v2, 8), alloc(rhs, 8)];
        const pOut = S._malloc(8 * nRhs * N2);
        ptrs.push(pOut);
        try {
            const rc = S._solve_sparse_multi(N2, k, ptrs[0], ptrs[1], ptrs[2], nRhs, ptrs[3], pOut, 1);
            if (rc < 0) return rc;
            f64().set(new Float64Array(S.HEAPF64.buffer, pOut, nRhs * N2).slice(), pX >> 3);
            return 0;
        } finally {
            for (const p of ptrs) S._free(p);
        }
    }
    return {
        _malloc: S._malloc, _free: S._free,
        get HEAP32() { return S.HEAP32; }, get HEAPF64() { return S.HEAPF64; },
        _solve_sparse_multi: S._solve_sparse_multi,
        _solve_complex_symmetric: solveComplexSymmetric,
        _solve_generalized: () => { throw new Error('kisi-advanced.html without the full-wave modules: no eigenproblem solver (tools/advanced_bundle.py --fw)'); },
    };
}
