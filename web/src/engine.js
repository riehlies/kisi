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
    const { module, instance } = await WebAssembly.instantiate(wasm, {});
    this.mod = module; // compiled once, posted to the check workers (check-pool.js)
    this.k = instance.exports;
    this.log = new Map(); this.stateGen = 0; this.onState = null;
  }
  // The calls that set the engine's state, for engines in workers that compute the channel check
  // in parallel (check-pool.js): the last call of each kind (Touchstone files by name) in the
  // order they were made – a new board drops what it replaces. replay(): [[export, input], …];
  // stateGen counts the changes, onState() hears of them.
  rec(slot, fn, text) {
    const last = this.log.get(slot);
    if (last && last.text === text && [...this.log.keys()].pop() === slot) return; // the same call again
    if (slot === "pcb") for (const s of ["config", "variant", "analyze", "skeleton", "buses"]) this.log.delete(s);
    this.log.delete(slot);
    this.log.set(slot, { fn, text });
    this.stateGen++;
    if (this.onState) this.onState();
  }
  replay() { return [...this.log.values()].map((e) => [e.fn, e.text]); }
  // a state call: run it, then note it (calls that fail are not noted)
  callRec(slot, fn, text) { this.call(this.k[fn], text); this.rec(slot, fn, text); }
  out() { return dec.decode(new Uint8Array(this.k.memory.buffer, this.k.kisi_out_ptr(), this.k.kisi_out_len())); }
  call(fn, text) {
    const b = enc.encode(text || "");
    const p = this.k.kisi_alloc(b.length);
    new Uint8Array(this.k.memory.buffer, p, b.length).set(b);
    const rc = fn(p, b.length);
    this.k.kisi_free(p, b.length);
    if (rc !== 0) throw new Error(this.out());
  }
  loadPro(text) { this.callRec("pro", "kisi_load_pro", text); }
  loadPcb(text) { this.callRec("pcb", "kisi_load_pcb", text); }
  // board configuration; files: BOMs its variants name ({path: text}); variant: the viewer's choice or null
  config(text, board, files, variant) { this.callRec("config", "kisi_config", JSON.stringify({ config: text, board, files: files || {}, variant: variant || null })); return JSON.parse(this.out()); }
  // board preparation (kisi_core::gaps / config): the gap list with a configuration text (the loaded
  // kisi.json or a draft; null: none), a check of a text without applying it, one category's entries
  gaps(board, config, view) { this.call(this.k.kisi_gaps, JSON.stringify({ board, config: config ?? null, for: view || null })); return JSON.parse(this.out()); }
  configCheck(board, config) { this.call(this.k.kisi_config_check, JSON.stringify({ board, config })); return JSON.parse(this.out()); }
  configSubset(config, view) { this.call(this.k.kisi_config_subset, JSON.stringify({ config, view })); return this.out(); }
  // the viewer's power tree / rail setups as kisi.json fields: {fields: [{ref, name, value}], lost}
  editsToFields(tree, setups) { this.call(this.k.kisi_edits_to_fields, JSON.stringify({ tree: tree || null, setups: setups || null })); return JSON.parse(this.out()); }
  boardSheets() { if (this.k.kisi_board_sheets() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  // assembly variant: the state, or choose one (null: the configuration's) – the board is rebuilt
  variants() { if (this.k.kisi_variants() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  variant(name) { this.callRec("variant", "kisi_variant", JSON.stringify({ variant: name || null })); return JSON.parse(this.out()); }
  scene() { if (this.k.kisi_scene() !== 0) throw new Error(this.out()); return JSON.parse(this.out()); }
  analyze(params) { this.callRec("analyze", "kisi_analyze", JSON.stringify(params)); return JSON.parse(this.out()); }
  buses(params) { this.callRec("buses", "kisi_buses", JSON.stringify(params)); return JSON.parse(this.out()); }
  laneRf(req) { this.call(this.k.kisi_lane_rf, JSON.stringify(req)); return JSON.parse(this.out()); }
  // a lane or a bus for circuit and RF simulators (kisi export): {summary, files: [{path, content}], name}
  exportCircuit(req) { this.call(this.k.kisi_export, JSON.stringify(req)); return JSON.parse(this.out()); }
  // Touchstone file for the channel behind a connector (kept in the engine's memory)
  touchstone(name, text) { this.callRec("ts:" + name, "kisi_touchstone", JSON.stringify({ name, text })); return JSON.parse(this.out()); }
  skeleton(params) { this.callRec("skeleton", "kisi_skeleton", JSON.stringify(params)); return JSON.parse(this.out()); }
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
  // temperature estimate of the stored DC results: ThermalResult + t (°C planes per copper layer, NaN outside)
  piThermal(params) {
    this.call(this.k.kisi_pi_thermal, JSON.stringify(params || {}));
    const r = JSON.parse(this.out());
    r.t = new Float32Array(this.k.memory.buffer, this.k.kisi_pi_temp_ptr(), this.k.kisi_pi_temp_len()).slice();
    return r;
  }
  // one rail at the estimated temperature: RailThermal (copper temperature, drop at 20 °C and at temperature)
  piThermalRail(setup, params) { this.call(this.k.kisi_pi_thermal_rail, JSON.stringify({ setup, params })); return JSON.parse(this.out()); }
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
