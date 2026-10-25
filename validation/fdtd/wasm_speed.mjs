// Run a case in the wasm build of kisi-fdtd (same engine as in the browser: V8) and time it.
// usage: node wasm_speed.mjs kisi_fdtd.wasm case.json mesh.json out.json
import { readFileSync, writeFileSync } from "node:fs";
const [wasmPath, casePath, meshPath, outPath] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const k = instance.exports;
const req = new TextEncoder().encode(JSON.stringify({ case: JSON.parse(readFileSync(casePath, "utf8")), mesh: JSON.parse(readFileSync(meshPath, "utf8")) }));
const p = k.fd_alloc(req.length);
new Uint8Array(k.memory.buffer, p, req.length).set(req);
const t0 = performance.now();
const code = k.fd_run(p, req.length);
const secs = (performance.now() - t0) / 1000;
const out = new TextDecoder().decode(new Uint8Array(k.memory.buffer, k.fd_out_ptr(), k.fd_out_len()));
const r = JSON.parse(out);
if (code !== 0) throw new Error(out);
r.runs[0].seconds = secs;
writeFileSync(outPath, JSON.stringify(r));
console.log(`${r.name}: wasm ${secs.toFixed(1)} s, ${r.runs[0].steps} steps, ${(r.cells * r.runs[0].steps / secs / 1e6).toFixed(1)} Mcells/s, memory ${(k.memory.buffer.byteLength / 2 ** 20).toFixed(0)} MB`);
