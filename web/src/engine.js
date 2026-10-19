"use strict";
// ============================================================== engine (WASM)
const enc = new TextEncoder(), dec = new TextDecoder();
const b64ToBytes = (b64) => { const s = atob(b64); const u = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i); return u; };
async function gunzipB64(b64) {
  const stream = new Blob([b64ToBytes(b64)]).stream().pipeThrough(new DecompressionStream("gzip"));
  return await new Response(stream).text();
}
class Engine {
  async init() {
    const { instance } = await WebAssembly.instantiate(b64ToBytes(WASM_B64), {});
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
  config(text, board) { this.call(this.k.kisi_config, JSON.stringify({ config: text, board })); return JSON.parse(this.out()); }
  scene() { if (this.k.kisi_scene() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  analyze(params) { this.call(this.k.kisi_analyze, JSON.stringify(params)); return JSON.parse(this.out()); }
  buses(params) { this.call(this.k.kisi_buses, JSON.stringify(params)); return JSON.parse(this.out()); }
  laneRf(req) { this.call(this.k.kisi_lane_rf, JSON.stringify(req)); return JSON.parse(this.out()); }
  skeleton(params) { this.call(this.k.kisi_skeleton, JSON.stringify(params)); return JSON.parse(this.out()); }
  piNets() { if (this.k.kisi_pi_nets() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  piFields() { if (this.k.kisi_pi_fields() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  rf(req) { this.call(this.k.kisi_rf, JSON.stringify(req)); return JSON.parse(this.out()); }
  pdn(req) { this.call(this.k.kisi_pdn, JSON.stringify(req)); return JSON.parse(this.out()); }
  judge(req) { this.call(this.k.kisi_judge, JSON.stringify(req)); return JSON.parse(this.out()); }
  profileAuto(req) { this.call(this.k.kisi_profile_auto, JSON.stringify(req)); return this.out(); }
  piTreeFields() { this.k.kisi_pi_tree_fields(); return JSON.parse(this.out()); }
  piTree(spec) { this.call(this.k.kisi_pi_tree, JSON.stringify(spec)); return JSON.parse(this.out()); }
  piKipida(text) { this.call(this.k.kisi_pi_kipida, text); return JSON.parse(this.out()); }
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
