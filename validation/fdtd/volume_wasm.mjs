// The volume field output (docs/research/fdtd-field-view.md): run a case in the WebAssembly build of kisi-fdtd.
// node volume_wasm.mjs kisi_fdtd.wasm case.json out.bin [port]: fd_run_volume with the case key `volume` (blob as
// `kisi-fdtd case.json --volume out.bin`); with out "-": fd_run (S only). Prints run time, cell rate and memory.
import { readFileSync, writeFileSync } from "node:fs";
const [wasmPath, casePath, outPath, port] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const k = instance.exports;
const c = JSON.parse(readFileSync(casePath, "utf8"));
const vol = c.volume; delete c.volume;
const req = new TextEncoder().encode(JSON.stringify({ case: c, mesh: null, volume: vol, port: +(port || 0) }));
const p = k.fd_alloc(req.length);
new Uint8Array(k.memory.buffer, p, req.length).set(req);
const t0 = performance.now();
const code = outPath === "-" ? k.fd_run(p, req.length) : k.fd_run_volume(p, req.length);
const secs = (performance.now() - t0) / 1000;
const out = new Uint8Array(k.memory.buffer, k.fd_out_ptr(), k.fd_out_len());
if (code !== 0) throw new Error(new TextDecoder().decode(out));
let steps, cells;
if (outPath === "-") { const r = JSON.parse(new TextDecoder().decode(out)); steps = r.runs[0].steps; cells = r.cells; }
else { writeFileSync(outPath, out); const hl = new DataView(out.buffer, out.byteOffset).getUint32(0, true); const h = JSON.parse(new TextDecoder().decode(out.subarray(4, 4 + hl))); steps = h.extra.steps; cells = h.extra.cells; }
console.log(`${casePath.split("/").pop()} ${outPath === "-" ? "fd_run" : "fd_run_volume"}: ${secs.toFixed(1)} s, ${steps} steps, ${(cells * steps / secs / 1e6).toFixed(1)} Mcells/s, wasm memory ${(k.memory.buffer.byteLength / 2 ** 20).toFixed(0)} MB, output ${(out.length / 2 ** 20).toFixed(1)} MB, load ${(await import("node:os")).loadavg()[0].toFixed(2)}`);
