// Prototype via field viewer: kisi-fdtd volume blobs (+ the case JSON) → a gzip'd KVF file (tools/proto_via_field.py).
// node encode.mjs case.json out.kvf mantissaBits made blob1.bin [blob2.bin …]
import vm from "node:vm";
import fs from "node:fs";
import zlib from "node:zlib";
import path from "node:path";
const dir = path.dirname(new URL(import.meta.url).pathname);
for (const f of ["case.js", "fields.js"]) vm.runInThisContext(fs.readFileSync(path.join(dir, f), "utf8"), { filename: f });
const [casePath, out, mb, made, ...bins] = process.argv.slice(2);
const kase = JSON.parse(fs.readFileSync(casePath, "utf8"));
delete kase.volume;
const ds = globalThis.KVF.fromRaw(bins.map((b) => new Uint8Array(fs.readFileSync(b))), kase);
const hs = bins.map((b) => globalThis.KVF.parseRaw(new Uint8Array(fs.readFileSync(b))).head.extra);
ds.runs.forEach((r, i) => { r.seconds = hs[i].seconds; });
const raw = globalThis.KVF.encode(ds, { mantissaBits: +mb, meta: { made } });
const z = zlib.gzipSync(raw, { level: 9 });
fs.writeFileSync(out, z);
console.log(`${out}: ${(raw.length / 2 ** 20).toFixed(2)} MB raw, ${(z.length / 2 ** 20).toFixed(2)} MB gzip, ${ds.n} nodes, ${ds.freqs.length} frequencies, ${ds.runs.length} runs`);
