// SPDX-License-Identifier: GPL-3.0-only
// Save-handler behavior with a deferred/failed host; browser-to-disk coverage is check_kicad_local.py.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function viewer(request) {
  const calls = { requests: [], loads: [], downloads: 0, pickers: 0, stored: {} };
  const elements = new Map();
  const s = { name: "Board", scene: {}, kicad: { bridge: { revision: "before" } },
    engine: { configCheck: () => ({ ok: true }) }, lastFiles: { name: "Board", pcbText: "pcb", proText: null } };
  const ctx = vm.createContext({ S: s, console, document: { addEventListener() {} },
    $: (id) => { if (!elements.has(id)) elements.set(id, { addEventListener() {} }); return elements.get(id); },
    window: { showSaveFilePicker: async () => { calls.pickers++; throw new Error("unexpected picker"); } },
    kicadBridgeActive: () => true, kicadActive: () => true,
    kicadRequest: async (path, body) => { calls.requests.push({ path, body }); return request(body); },
    lsSet: (key, value) => { calls.stored[key] = value; },
    loadFiles: async (...args) => { calls.loads.push(args); },
  });
  vm.runInContext(readFileSync(new URL("../web/src/prepare.js", import.meta.url), "utf8"), ctx);
  vm.runInContext(`
    S.prep.scope = "all";
    S.prep.d = { name: S.name, work: { kisi_config: 1, board: "Board.kicad_pcb", notes: ["saved"] } };
    prepText = () => JSON.stringify(prepW());
    prepRender = () => {};
  `, ctx);
  ctx.prepDownload = () => { calls.downloads++; };
  return { ctx, s, calls };
}

test("plugin save uses the host and advances the revision only after success", async () => {
  const { ctx, s, calls } = viewer(async () => ({ revision: "after" }));
  await ctx.prepSave();
  assert.equal(calls.requests.length, 1);
  assert.equal(calls.requests[0].path, "/api/config");
  assert.equal(calls.requests[0].body.revision, "before");
  assert.equal(JSON.parse(calls.requests[0].body.text).notes[0], "saved");
  assert.equal(s.kicad.bridge.revision, "after");
  assert.equal(calls.loads.length, 1);
  assert.equal(calls.downloads + calls.pickers, 0);
  assert.equal(s.prep.saving, false);
  assert.match(s.prep.savedNext, /saved into the board/);
});

test("a rejected save preserves the draft and does not fall back to a download", async () => {
  const { ctx, s, calls } = viewer(async () => { throw new Error("conflict"); });
  await ctx.prepSave();
  assert.equal(s.prep.d.work.notes[0], "saved");
  assert.equal(s.kicad.bridge.revision, "before");
  assert.equal(calls.loads.length + calls.downloads + calls.pickers, 0);
  assert.match(s.prep.saved, /Not saved: conflict/);
  assert.equal(s.prep.saving, false);
});

test("a double click sends once and edits during the request stay unsaved", async () => {
  let finish;
  const { ctx, s, calls } = viewer(() => new Promise((resolve) => { finish = resolve; }));
  const first = ctx.prepSave();
  await ctx.prepSave();
  s.prep.d.work.notes = ["typed during save"];
  finish({ revision: "after" });
  await first;
  assert.equal(calls.requests.length, 1);
  const saved = calls.stored["kisi.prep.v1.Board"];
  assert.equal(saved.work.notes[0], "typed during save");
  assert.equal(JSON.parse(saved.baseText).notes[0], "saved");
  assert.equal(calls.downloads + calls.pickers, 0);
});

test("switching boards while saving does not replace the new board's state", async () => {
  let finish;
  const { ctx, s, calls } = viewer(() => new Promise((resolve) => { finish = resolve; }));
  const pending = ctx.prepSave();
  s.name = "Other";
  s.kicad = { bridge: { revision: "other revision" } };
  finish({ revision: "after" });
  await pending;
  assert.equal(s.kicad.bridge.revision, "other revision");
  assert.equal(calls.loads.length + calls.downloads + calls.pickers, 0);
});

test("cancelling the standalone file picker clears the saving state", async () => {
  const { ctx, s, calls } = viewer(async () => { throw new Error("unexpected host"); });
  ctx.kicadBridgeActive = () => false;
  ctx.kicadActive = () => false;
  ctx.textHash = () => "id";
  ctx.window.showSaveFilePicker = async () => { const e = new Error("cancelled"); e.name = "AbortError"; throw e; };
  await ctx.prepSave();
  assert.equal(s.prep.saving, false);
  assert.equal(s.prep.saved, "Save cancelled.");
  assert.equal(calls.requests.length + calls.loads.length + calls.downloads, 0);
});
