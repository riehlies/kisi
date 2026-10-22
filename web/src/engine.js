"use strict";
// ============================================================== engine (WASM)
const enc = new TextEncoder(), dec = new TextDecoder();
const b64ToBytes = (b64) => { const s = atob(b64); const u = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i); return u; };
const gunzipResponse = (b64) => new Response(new Blob([b64ToBytes(b64)]).stream().pipeThrough(new DecompressionStream("gzip")));
async function gunzipB64(b64) { return await gunzipResponse(b64).text(); }
async function gunzipBytes(b64) { return new Uint8Array(await gunzipResponse(b64).arrayBuffer()); }
class Engine {
  async init() {
    const [wasm, sparse, solver] = await Promise.all([gunzipBytes(WASM_GZ), gunzipBytes(SPARSE_GZ), gunzipB64(SOLVER_GZ)]);
    SPARSE_WASM = sparse; SOLVER_SRC = solver;
    const { instance } = await WebAssembly.instantiate(wasm, {});
    this.k = instance.exports;
  }
  out() { return dec.decode(new Uint8Array(this.k.memory.buffer, this.k.kisi_out_ptr(), this.k.kisi_out_len())); }
  call(fn, text) {
    const b = enc.encode(text || "");
    const p = this.k.kisi_alloc(b.length);
    new Uint8Array(this.k.memory.buffer, p, b.length).set(b);
    const rc = fn(p, b.length);
    this.k.kisi_free(p, b.length);
    if (rc !== 0) throw new Error(this.out());
  }
  loadPro(text) { this.call(this.k.kisi_load_pro, text); }
  loadPcb(text) { this.call(this.k.kisi_load_pcb, text); }
  // board configuration; files: BOMs its variants name ({path: text}); variant: the viewer's choice or null;
  // ibis (optional): IBIS models {path: base64} – parsed into the engine's memory; the viewer registers dropped
  // and card-loaded files with ibisLoad instead, and the configuration finds them by file name
  config(text, board, files, variant, ibis) { this.call(this.k.kisi_config, JSON.stringify({ config: text, board, files: files || {}, variant: variant || null, ibis: ibis || {} })); return JSON.parse(this.out()); }
  // assembly variant: the state, or choose one (null: the configuration's) – the board is rebuilt
  variants() { if (this.k.kisi_variants() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  variant(name) { this.call(this.k.kisi_variant, JSON.stringify({ variant: name || null })); return JSON.parse(this.out()); }
  scene() { if (this.k.kisi_scene() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  analyze(params) { this.call(this.k.kisi_analyze, JSON.stringify(params)); return JSON.parse(this.out()); }
  buses(params) { this.call(this.k.kisi_buses, JSON.stringify(params)); return JSON.parse(this.out()); }
  laneRf(req) { this.call(this.k.kisi_lane_rf, JSON.stringify(req)); return JSON.parse(this.out()); }
  // IBIS model file for the lane ends (kept in the engine's memory only): summary of components, selectors, models
  ibisLoad(name, b64) { this.call(this.k.kisi_ibis_load, JSON.stringify({ name, b64 })); return JSON.parse(this.out()); }
  ibisRemove(name) { this.call(this.k.kisi_ibis_load, JSON.stringify({ name, remove: true })); }
  // the IBIS ends of lanes for a mapping {refdes: entry}: {laneKey: {tx, rx}} (kisi_core::ibis::lane_ends)
  ibisEnds(req) { this.call(this.k.kisi_ibis_ends, JSON.stringify(req)); return JSON.parse(this.out()); }
  // Touchstone file for the channel behind a connector (kept in the engine's memory)
  touchstone(name, text) { this.call(this.k.kisi_touchstone, JSON.stringify({ name, text })); return JSON.parse(this.out()); }
  skeleton(params) { this.call(this.k.kisi_skeleton, JSON.stringify(params)); return JSON.parse(this.out()); }
  piNets() { if (this.k.kisi_pi_nets() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  piFields() { if (this.k.kisi_pi_fields() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  rf(req) { this.call(this.k.kisi_rf, JSON.stringify(req)); return JSON.parse(this.out()); }
  xtScan(req) { this.call(this.k.kisi_xt_scan, JSON.stringify(req)); return JSON.parse(this.out()); }
  // main path of a net with ends and branches (kisi_core::rf::net_path_info) or null
  netPath(req) { this.call(this.k.kisi_net_path, JSON.stringify(req)); return JSON.parse(this.out()); }
  pdn(req) { this.call(this.k.kisi_pdn, JSON.stringify(req)); return JSON.parse(this.out()); }
  pdnIc(req) { this.call(this.k.kisi_pdn_ic, JSON.stringify(req)); return JSON.parse(this.out()); }
  judge(req) { this.call(this.k.kisi_judge, JSON.stringify(req)); return JSON.parse(this.out()); }
  profileAuto(req) { this.call(this.k.kisi_profile_auto, JSON.stringify(req)); return this.out(); }
  piTreeFields() { this.k.kisi_pi_tree_fields(); return JSON.parse(this.out()); }
  piTree(spec) { this.call(this.k.kisi_pi_tree, JSON.stringify(spec)); return JSON.parse(this.out()); }
  piClear() { this.k.kisi_pi_clear(); }
  piGround() { if (this.k.kisi_pi_ground() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  // losses of the stored DC results: {summary, flow, map, buf} (buf: copper planes, then components; W/mm²)
  piLosses(cell) {
    this.call(this.k.kisi_pi_losses, JSON.stringify({ cell: cell || null }));
    const r = JSON.parse(this.out()), m = this.k.memory.buffer;
    r.buf = new Float32Array(m, this.k.kisi_pi_map_ptr(), this.k.kisi_pi_map_len()).slice();
    return r;
  }
  piSolve(setup, params) {
    this.call(this.k.kisi_pi_solve, JSON.stringify({ setup, params }));
    const res = JSON.parse(this.out()), m = this.k.memory.buffer;
    return {
      res,
      v: new Float32Array(m, this.k.kisi_pi_v_ptr(), this.k.kisi_pi_v_len()).slice(),
      j: new Float32Array(m, this.k.kisi_pi_j_ptr(), this.k.kisi_pi_j_len()).slice(),
    };
  }
  zones() {
    this.k.kisi_zones();
    const m = this.k.memory.buffer;
    return {
      pts: new Float32Array(m, this.k.kisi_zone_pts_ptr(), this.k.kisi_zone_pts_len()).slice(),
      idx: new Uint32Array(m, this.k.kisi_zone_idx_ptr(), this.k.kisi_zone_idx_len()).slice(),
    };
  }
}
