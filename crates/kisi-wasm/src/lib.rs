//! WebAssembly bindings with a plain C ABI (no wasm-bindgen, no JS glue needed).
//!
//! Protocol: JS allocates with `kisi_alloc`, copies UTF-8 text in, calls a
//! function; results (JSON) are read from `kisi_out_ptr/len`.
//! Return codes: 0 = ok, 1 = error (message in out buffer).

use kisi_core::{analyze, apply_project, load_pcb, view, Board, Params};
use std::cell::RefCell;

#[derive(Default)]
struct State {
    /// board as analysed: `board_base` with the assembly variant and the configuration
    board: Option<Board>,
    /// board as read (with the project file): variants and configuration are applied to a copy
    board_base: Option<Board>,
    /// configuration (`<board>.kisi.json`), the files it refers to (BOMs) and the viewer's
    /// choice of assembly variant (wins over the configuration's)
    cfg: Option<kisi_core::config::BoardConfig>,
    files: kisi_core::variant::Files,
    variant_choice: Option<String>,
    variant: Option<kisi_core::variant::VariantState>,
    /// stackup as read from the file (overrides are applied to a copy)
    stackup_file: Option<kisi_core::board::Stackup>,
    /// the viewer's stackup changes of the last analysis
    stack_ov: kisi_core::board::StackupOverride,
    /// the board's own stackup of the last analysis (file, configuration, the Stackup card's changes)
    stack_base: Option<kisi_core::board::Stackup>,
    /// the active stackup of the option "fab stackups & tolerances" (a template and/or changes over
    /// `stack_base`, as `--stackup-preset` / `--stackup`); None: the board's
    active: Option<(Option<String>, Option<kisi_core::board::StackupOverride>)>,
    /// the last analyses by the viewer's key (switching stackups back and forth)
    an_cache: Vec<AnCache>,
    /// the power results of the last stackups left (DC per rail, losses map, temperature), put back when
    /// the board gets that stackup again (`put_stackup`); the viewer decides whether they still apply
    pi_stash: Vec<PiStash>,
    pro: Option<String>,
    out: Vec<u8>,
    zone_pts: Vec<f32>,
    zone_idx: Vec<u32>,
    pi_v: Vec<f32>,
    pi_j: Vec<f32>,
    /// last solved power tree, and the DC results per rail (loss buffers only) for the losses
    tree: Option<kisi_core::tree::TreeResult>,
    pi_store: std::collections::BTreeMap<String, kisi_core::pi::PiResult>,
    /// loss map: a plane per copper layer, then the components plane (W per mm² of board)
    loss_map: Vec<f32>,
    /// the layer planes of the last `kisi_pi_losses`: their copper cells and densities on the rails' DC
    /// grids (W/mm²), handed over to the caller, not kept with the stackup's results
    loss_cells: Vec<u32>,
    loss_layer: Vec<f32>,
    /// last temperature estimate (from `pi_store` and `tree`), with its temperature planes
    thermal: Option<kisi_core::thermal::ThermalResult>,
    /// last analysis and its parameters (for the RF functions)
    an: Option<kisi_core::Analysis>,
    params: Option<Params>,
    /// device-to-device lanes of the board (topology), from `kisi_buses`
    buses: Option<Vec<kisi_core::topology::Bus>>,
    /// Touchstone files for the channel behind connectors (in memory, by file name)
    touchstones: std::collections::HashMap<String, kisi_core::rf::Touchstone>,
}

/// The Touchstone file a connector end names: by the path written, else by its file name.
fn ts_for<'a>(st: &'a State, ce: Option<&kisi_core::eye::ConnEnd>) -> Option<&'a kisi_core::rf::Touchstone> {
    let f = &ce?.touchstone.as_ref()?.file;
    st.touchstones.get(f).or_else(|| st.touchstones.get(f.rsplit(['/', '\\']).next()?))
}

/// One N-port file for the lanes of a connector (a port map in the connector end): the lanes at
/// that connector, aggressors of each other.
fn xt_for(
    st: &State,
    lane: &kisi_core::topology::Lane,
    ce: Option<&kisi_core::eye::ConnEnd>,
) -> Vec<kisi_core::lanenet::XtLane> {
    let mapped = ce.and_then(|c| c.touchstone.as_ref()).is_some_and(|t| !t.ports.is_empty());
    match (mapped, lane.ends.iter().find(|e| e.role == "connector"), st.buses.as_ref()) {
        (true, Some(e), Some(bs)) => kisi_core::topology::connector_lanes(bs.iter().flat_map(|b| &b.lanes), &e.refdes),
        _ => vec![],
    }
}

/// Load (or with "remove": true drop) a Touchstone file for the channel behind connectors:
/// {"name": file name, "text": contents}. Output: {nports, points, f_min_ghz, f_max_ghz, z0,
/// warnings}. Files stay in memory only.
#[no_mangle]
pub extern "C" fn kisi_touchstone(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply(ST.with(|st| -> Result<String, String> {
        let mut st = st.borrow_mut();
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let name = v["name"].as_str().ok_or("name missing")?.to_string();
        if v["remove"].as_bool() == Some(true) {
            st.touchstones.remove(&name);
            return Ok("{}".into());
        }
        let ts = kisi_core::rf::parse_touchstone(v["text"].as_str().ok_or("text missing")?, &name)?;
        let s = ts.summary().to_string();
        st.touchstones.insert(name, ts);
        Ok(s)
    }))
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State::default());
}

/// One cached analysis: its key, the stackup state it was made with, the analysis and its parameters.
struct AnCache {
    key: String,
    su: kisi_core::board::Stackup,
    base: Option<kisi_core::board::Stackup>,
    ov: kisi_core::board::StackupOverride,
    active: Option<(Option<String>, Option<kisi_core::board::StackupOverride>)>,
    an: kisi_core::Analysis,
    params: Params,
}

/// The power results of one stackup (by its JSON) while another one is on the board.
struct PiStash {
    key: String,
    store: std::collections::BTreeMap<String, kisi_core::pi::PiResult>,
    loss_map: Vec<f32>,
    thermal: Option<kisi_core::thermal::ThermalResult>,
}

/// Stackups whose power results are kept besides the current one (on ThunderScope about 100 MB each).
const PI_STASH_MAX: usize = 2;

/// Put a stackup on the analysed board. A different one moves the power results that depend on it
/// aside (the last `PI_STASH_MAX` stackups) and puts back those of the new one, if any.
fn put_stackup(st: &mut State, su: kisi_core::board::Stackup) {
    let Some(b) = st.board.as_mut() else { return };
    let old = b.stackup.as_ref().and_then(|x| serde_json::to_string(x).ok());
    let new = serde_json::to_string(&su).ok();
    b.stackup = Some(su);
    if old == new {
        return;
    }
    if let Some(key) = old.filter(|_| !st.pi_store.is_empty() || st.thermal.is_some()) {
        st.pi_stash.retain(|x| x.key != key);
        st.pi_stash.push(PiStash {
            key,
            store: std::mem::take(&mut st.pi_store),
            loss_map: std::mem::take(&mut st.loss_map),
            thermal: st.thermal.take(),
        });
        while st.pi_stash.len() > PI_STASH_MAX {
            st.pi_stash.remove(0);
        }
    }
    match st.pi_stash.iter().position(|x| Some(&x.key) == new.as_ref()) {
        Some(i) => {
            let x = st.pi_stash.remove(i);
            st.pi_store = x.store;
            st.loss_map = x.loss_map;
            st.thermal = x.thermal;
        }
        None => {
            st.pi_store.clear();
            st.loss_map.clear();
            st.thermal = None;
        }
    }
}

/// The power results the engine holds for the board's stackup: {"rails": [net…], "thermal": bool,
/// "stash": stackups kept aside, "stash_bytes": their value buffers} (the viewer puts its own back only
/// when they match).
#[no_mangle]
pub extern "C" fn kisi_pi_stored() -> i32 {
    ST.with(|st| {
        let st = st.borrow();
        let bytes: usize = st
            .pi_stash
            .iter()
            .map(|x| {
                4 * (x.loss_map.len() + x.store.values().map(|r| r.v.len() + r.j.len() + r.loss.len()).sum::<usize>())
                    + x.store.values().map(|r| r.cu.len()).sum::<usize>()
            })
            .sum();
        let v = serde_json::json!({"rails": st.pi_store.keys().collect::<Vec<_>>(), "thermal": st.thermal.is_some(),
            "stash": st.pi_stash.len(), "stash_bytes": bytes});
        drop(st);
        set_out(v.to_string());
    });
    0
}

/// The stackup of an analysis request: the board's (file and configuration, then the Stackup card's
/// changes, "stackup") and, with the option "fab stackups & tolerances" on, the active stackup over it
/// ("active_stackup": {"preset": id | null, "edits": StackupOverride | null}) – the core's
/// `stackups::active_stackup`, as the CLI's `--stackup-preset` / `--stackup`.
fn set_stackup(st: &mut State, v: &serde_json::Value) -> Result<(), String> {
    let ov: kisi_core::board::StackupOverride = serde_json::from_value(v["stackup"].clone()).unwrap_or_default();
    let a = &v["active_stackup"];
    let act = a.is_object().then(|| {
        (
            a["preset"].as_str().map(str::to_string),
            serde_json::from_value::<kisi_core::board::StackupOverride>(a["edits"].clone()).ok(),
        )
    });
    let Some(file) = st.stackup_file.clone() else { return Ok(()) };
    let base = file.with_overrides(&ov);
    let su = match &act {
        Some((p, e)) => kisi_core::stackups::active_stackup(&base, p.as_deref(), e.as_ref())?,
        None => base.clone(),
    };
    put_stackup(st, su);
    st.stack_ov = ov;
    st.stack_base = Some(base);
    st.active = act;
    Ok(())
}

fn input(p: *const u8, n: usize) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(p, n) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn set_out(s: String) {
    ST.with(|st| st.borrow_mut().out = s.into_bytes());
}

/// Rebuild the analysed board from the board as read: assembly variant, then the configuration
/// (fields, stackup). Results that depend on the parts (lanes, power) are dropped.
fn rebuild(st: &mut State) -> Result<Option<kisi_core::config::Applied>, String> {
    let mut b = st.board_base.clone().ok_or("no board loaded")?;
    let choice = st.variant_choice.as_deref().map(|v| (v, "viewer"));
    let vs = kisi_core::variant::apply(&mut b, st.cfg.as_ref(), choice, &st.files);
    let applied = st.cfg.as_ref().map(|c| c.apply(&mut b));
    st.stackup_file = b.stackup.clone();
    st.stack_base = b.stackup.clone();
    st.an_cache.clear();
    st.pi_stash.clear();
    st.board = Some(b);
    st.variant = Some(vs);
    st.buses = None;
    st.tree = None;
    st.pi_store.clear();
    st.loss_map.clear();
    st.thermal = None;
    Ok(applied)
}

fn variant_json(st: &State) -> serde_json::Value {
    serde_json::to_value(&st.variant).unwrap_or(serde_json::Value::Null)
}

#[no_mangle]
pub extern "C" fn kisi_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `p` and `n` are a pointer and the size passed to `kisi_alloc`, not freed yet.
#[no_mangle]
pub unsafe extern "C" fn kisi_free(p: *mut u8, n: usize) {
    unsafe { drop(Vec::from_raw_parts(p, 0, n.max(1))) }
}

#[no_mangle]
pub extern "C" fn kisi_out_ptr() -> *const u8 {
    ST.with(|st| st.borrow().out.as_ptr())
}

#[no_mangle]
pub extern "C" fn kisi_out_len() -> usize {
    ST.with(|st| st.borrow().out.len())
}

/// Load a `.kicad_pcb`. Applies a previously loaded project file.
#[no_mangle]
pub extern "C" fn kisi_load_pcb(p: *const u8, n: usize) -> i32 {
    let text = input(p, n);
    match load_pcb(&text) {
        Ok(mut b) => {
            let pro = ST.with(|st| st.borrow().pro.clone());
            if let Some(pro) = pro {
                if let Err(e) = apply_project(&mut b, &pro) {
                    b.warnings.push(format!("project file: {e}"));
                }
            }
            ST.with(|st| {
                let mut st = st.borrow_mut();
                st.board_base = Some(b);
                st.cfg = None;
                st.files.clear();
                st.variant_choice = None;
                let _ = rebuild(&mut st);
            });
            set_out(String::new());
            0
        }
        Err(e) => {
            set_out(e.to_string());
            1
        }
    }
}

/// Apply a `<board>.kisi.json` to the loaded board (call right after kisi_load_pcb).
/// Input: {"config": file contents, "board": name of the loaded board, "files": {path: text}
/// (BOMs the variants refer to), "variant": the viewer's choice | null}.
/// Output: {"check": {ok, errors, warnings, entries}, "targets": {class: {z, diff}},
/// "profiles": {class: id}, "connector_ends": {connector: ConnEnd}, "fields": n, "dielectrics": n,
/// "variant": VariantState}. The board's
/// fields and file stackup are replaced by the configured values (viewer edits stay on top).
#[no_mangle]
pub extern "C" fn kisi_config(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let mut st = st.borrow_mut();
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let cfg = kisi_core::config::parse(v["config"].as_str().ok_or("config missing")?)?;
        let board_name = v["board"].as_str().map(str::to_string);
        let files: kisi_core::variant::Files = v["files"].as_object().map(|m| m.iter().filter_map(|(k, t)| Some((k.clone(), Ok(t.as_str()?.to_string())))).collect()).unwrap_or_default();
        let b = st.board_base.as_ref().ok_or("no board loaded")?;
        let an0 = kisi_core::extract::skeleton(b, &Params::default());
        let classes: Vec<String> = an0.classes.iter().map(|c| c.name.clone()).collect();
        let check = kisi_core::config::check(&cfg, b, &classes, &kisi_core::profile::Profiles::builtin(), board_name.as_deref(), &files);
        let notes = cfg.notes.clone();
        let conn_ends: serde_json::Map<String, serde_json::Value> = cfg.connector_ends.iter().filter_map(|(k, c)| Some((k.clone(), serde_json::to_value(&c.end).ok()?))).collect();
        for (path, ts) in kisi_core::service::touchstones_of(Some(&cfg), &files) {
            if let Ok(ts) = ts {
                st.touchstones.insert(path, ts);
            }
        }
        st.cfg = Some(cfg);
        st.files = files;
        st.variant_choice = v["variant"].as_str().map(str::to_string);
        let a = rebuild(&mut st)?.unwrap_or_default();
        let targets: serde_json::Map<String, serde_json::Value> = a.targets.iter().map(|(k, t)| (k.clone(), serde_json::json!({"z": t.z, "diff": t.diff}))).collect();
        let tolerances = st.cfg.as_ref().and_then(|c| c.tolerances.clone());
        Ok(serde_json::json!({"check": check, "targets": targets, "profiles": a.profiles, "connector_ends": conn_ends, "fields": a.fields, "dielectrics": a.dielectrics, "notes": notes, "variant": variant_json(&st), "tolerances": tolerances}).to_string())
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// The assembly variant in use: VariantState {variants, chosen, chosen_by, needs_choice,
/// message, forks, not_fitted, …}.
#[no_mangle]
pub extern "C" fn kisi_variants() -> i32 {
    reply(ST.with(|st| Ok(variant_json(&st.borrow()).to_string())))
}

/// Choose the assembly variant in the viewer: {"variant": name | null (the configuration's)}.
/// Rebuilds the board (parts, fields); lanes and power results must be computed again.
/// Output: VariantState.
#[no_mangle]
pub extern "C" fn kisi_variant(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply(ST.with(|st| -> Result<String, String> {
        let mut st = st.borrow_mut();
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        st.variant_choice = v["variant"].as_str().map(str::to_string);
        rebuild(&mut st)?;
        Ok(variant_json(&st).to_string())
    }))
}

/// Load a `.kicad_pro` (net classes). May be called before or after the board.
#[no_mangle]
pub extern "C" fn kisi_load_pro(p: *const u8, n: usize) -> i32 {
    let text = input(p, n);
    let r = ST.with(|st| {
        let mut st = st.borrow_mut();
        if let Some(b) = st.board_base.as_mut() {
            let _ = apply_project(b, &text);
        }
        let res = match st.board.as_mut() {
            Some(b) => apply_project(b, &text),
            None => serde_json::from_str::<serde_json::Value>(&text).map(|_| ()).map_err(|e| e.to_string()),
        };
        if res.is_ok() {
            st.pro = Some(text);
        }
        res
    });
    match r {
        Ok(()) => 0,
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Scene JSON (geometry for drawing, stackup, nets) into the out buffer.
#[no_mangle]
pub extern "C" fn kisi_scene() -> i32 {
    let s = ST.with(|st| {
        let st = st.borrow();
        st.board.as_ref().map(|b| serde_json::to_string(&view::scene(b)).unwrap())
    });
    match s {
        Some(s) => {
            set_out(s);
            0
        }
        None => {
            set_out("no board loaded".into());
            1
        }
    }
}

/// Run the analysis with JSON parameters (may be empty -> defaults).
#[no_mangle]
pub extern "C" fn kisi_analyze(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let params: Params = if txt.trim().is_empty() {
        Params::default()
    } else {
        match serde_json::from_str(&txt) {
            Ok(v) => v,
            Err(e) => {
                set_out(format!("params: {e}"));
                return 1;
            }
        }
    };
    // optional stackup overrides: {"stackup": {"dielectrics": [{index, epsilon_r, loss_tangent, thickness}],
    // "mask_epsilon_r"}}, the active stackup ("active_stackup"), the viewer's cache key ("cache_key";
    // with "restore_only" only a cached analysis is put back: exit code 2 when there is none)
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or_default();
    let key = v["cache_key"].as_str().map(str::to_string);
    if v["restore_only"].as_bool() == Some(true) {
        return ST.with(|st| {
            let mut g = st.borrow_mut();
            let st: &mut State = &mut g;
            let Some(i) = key.as_ref().and_then(|k| st.an_cache.iter().position(|c| &c.key == k)) else { return 2 };
            let c = st.an_cache.remove(i);
            put_stackup(st, c.su.clone());
            st.stack_base = c.base.clone();
            st.stack_ov = c.ov.clone();
            st.active = c.active.clone();
            st.an = Some(c.an.clone());
            st.params = Some(c.params.clone());
            st.an_cache.push(c);
            st.out = b"{}".to_vec();
            0
        });
    }
    let s = ST.with(|st| -> Result<String, String> {
        let mut g = st.borrow_mut();
        let st: &mut State = &mut g;
        set_stackup(st, &v)?;
        let an = st.board.as_ref().map(|b| analyze(b, &params)).ok_or("no board loaded")?;
        let js = serde_json::to_string(&an).unwrap();
        if let (Some(k), Some(su)) = (key, st.board.as_ref().and_then(|b| b.stackup.clone())) {
            st.an_cache.retain(|c| c.key != k);
            if st.an_cache.len() >= 4 {
                st.an_cache.remove(0);
            }
            st.an_cache.push(AnCache {
                key: k,
                su,
                base: st.stack_base.clone(),
                ov: st.stack_ov.clone(),
                active: st.active.clone(),
                an: an.clone(),
                params: params.clone(),
            });
        }
        st.an = Some(an);
        st.params = Some(params);
        Ok(js)
    });
    match s {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Board summary without impedances (net classes, targets, one piece per track):
/// lets the viewer show the board immediately; `kisi_analyze` adds the impedances.
#[no_mangle]
pub extern "C" fn kisi_skeleton(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let params: Params = serde_json::from_str(&txt).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or_default();
    let s = ST.with(|st| {
        let mut g = st.borrow_mut();
        let st: &mut State = &mut g;
        // the stackup as for the analysis (the power views use it before the impedance estimate)
        if set_stackup(st, &v).is_err() {
            let _ = set_stackup(st, &serde_json::json!({"stackup": v["stackup"]}));
        }
        st.an = None; // channel / RF need the full analysis
        st.board.as_ref().map(|b| serde_json::to_string(&kisi_core::extract::skeleton(b, &params)).unwrap())
    });
    match s {
        Some(s) => {
            set_out(s);
            0
        }
        None => {
            set_out("no board loaded".into());
            1
        }
    }
}

/// Prepare zone fill buffers; returns number of polygons.
#[no_mangle]
pub extern "C" fn kisi_zones() -> u32 {
    ST.with(|st| {
        let mut st = st.borrow_mut();
        let Some(b) = st.board.as_ref() else { return 0 };
        let (pts, idx) = view::zone_buffers(b);
        st.zone_pts = pts;
        st.zone_idx = idx;
        (st.zone_idx.len() / 4) as u32
    })
}

#[no_mangle]
pub extern "C" fn kisi_zone_pts_ptr() -> *const f32 {
    ST.with(|st| st.borrow().zone_pts.as_ptr())
}
#[no_mangle]
pub extern "C" fn kisi_zone_pts_len() -> usize {
    ST.with(|st| st.borrow().zone_pts.len())
}
#[no_mangle]
pub extern "C" fn kisi_zone_idx_ptr() -> *const u32 {
    ST.with(|st| st.borrow().zone_idx.as_ptr())
}
#[no_mangle]
pub extern "C" fn kisi_zone_idx_len() -> usize {
    ST.with(|st| st.borrow().zone_idx.len())
}

// --- DC power integrity ---------------------------------------------------------

fn with_board<F: FnOnce(&Board) -> Result<String, String>>(f: F) -> i32 {
    let r = ST.with(|st| match st.borrow().board.as_ref() {
        Some(b) => f(b),
        None => Err("no board loaded".into()),
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Supply nets of the board (JSON).
#[no_mangle]
pub extern "C" fn kisi_pi_nets() -> i32 {
    with_board(|b| Ok(serde_json::to_string(&kisi_core::pi::power_nets(b)).unwrap()))
}

/// Rail setups from the PI_SOURCE / PI_LOAD footprint fields: {rails, warnings}.
#[no_mangle]
pub extern "C" fn kisi_pi_fields() -> i32 {
    with_board(|b| {
        let (rails, warnings) = kisi_core::pi::rails_from_fields(b);
        Ok(serde_json::json!({"rails": rails, "warnings": warnings}).to_string())
    })
}

/// Solve one rail: input {"setup": RailSetup, "params": PiParams}. Summary JSON
/// in the out buffer; voltage / current density grids via kisi_pi_v_* / kisi_pi_j_*.
#[no_mangle]
pub extern "C" fn kisi_pi_solve(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let mut bufs: Option<(Vec<f32>, Vec<f32>, kisi_core::pi::PiResult)> = None;
    let rc = with_board(|b| {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let setup: kisi_core::pi::RailSetup = serde_json::from_value(v["setup"].clone()).map_err(|e| e.to_string())?;
        let params: kisi_core::pi::PiParams = serde_json::from_value(v["params"].clone()).unwrap_or_default();
        let mut r = kisi_core::pi::solve_rail(b, &setup, &params)?;
        let out = serde_json::to_string(&r).unwrap();
        let (vb, jb) = (std::mem::take(&mut r.v), std::mem::take(&mut r.j));
        bufs = Some((vb, jb, r));
        Ok(out)
    });
    if let Some((v, j, r)) = bufs {
        ST.with(|st| {
            let mut st = st.borrow_mut();
            st.pi_v = v;
            st.pi_j = j;
            st.pi_store.insert(r.net.clone(), r);
        });
    }
    rc
}

#[no_mangle]
pub extern "C" fn kisi_pi_v_ptr() -> *const f32 {
    ST.with(|st| st.borrow().pi_v.as_ptr())
}
#[no_mangle]
pub extern "C" fn kisi_pi_v_len() -> usize {
    ST.with(|st| st.borrow().pi_v.len())
}
#[no_mangle]
pub extern "C" fn kisi_pi_j_ptr() -> *const f32 {
    ST.with(|st| st.borrow().pi_j.as_ptr())
}
#[no_mangle]
pub extern "C" fn kisi_pi_j_len() -> usize {
    ST.with(|st| st.borrow().pi_j.len())
}

/// Solve a power tree (regulator chains, loads): TreeResult JSON incl. the
/// IR-drop rail setups.
#[no_mangle]
pub extern "C" fn kisi_pi_tree(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let mut tree = None;
    let rc = with_board(|b| {
        let spec: kisi_core::tree::TreeSpec = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let r = kisi_core::tree::solve_tree(&spec, Some(b))?;
        let out = serde_json::to_string(&r).unwrap();
        tree = Some(r);
        Ok(out)
    });
    ST.with(|st| st.borrow_mut().tree = tree);
    rc
}

/// Forget the stored DC results (before all rails are solved again).
#[no_mangle]
pub extern "C" fn kisi_pi_clear() {
    ST.with(|st| {
        let mut st = st.borrow_mut();
        st.pi_store.clear();
        st.loss_map.clear();
        st.loss_layer = Vec::new();
        st.loss_cells = Vec::new();
        st.thermal = None;
    });
}

/// Ground return setups of the last power tree: {setups, warnings}.
#[no_mangle]
pub extern "C" fn kisi_pi_ground() -> i32 {
    let r = ST.with(|st| {
        let st = st.borrow();
        match (st.board.as_ref(), st.tree.as_ref()) {
            (Some(b), Some(t)) => {
                let (setups, warnings) = kisi_core::losses::ground_setups(b, t);
                Ok(serde_json::json!({"setups": setups, "warnings": warnings}).to_string())
            }
            _ => Err("no power tree".to_string()),
        }
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Losses of the stored DC results and the last power tree. Input {"cell": mm|null}.
/// Out: {summary, flow, map, layers}; the map planes via kisi_pi_map_ptr/len (W per mm² of board): one per
/// copper layer of `map.layers`, then the components plane, nx*ny cells each;
/// `layers`: per copper layer of the map its rails on their own DC grids (LayerLoss: planes with
/// offsets into two buffers of their copper cells – the index in the plane's grid, via
/// kisi_pi_loss_cells_ptr/len – and the W per mm² of copper there, via kisi_pi_loss_layer_ptr/len).
#[no_mangle]
pub extern "C" fn kisi_pi_losses(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let cell = serde_json::from_str::<serde_json::Value>(&txt).ok().and_then(|v| v["cell"].as_f64());
    let r = ST.with(|st| {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let rails: Vec<&kisi_core::pi::PiResult> = st.pi_store.values().collect();
        let s = kisi_core::losses::loss_summary(b, st.tree.as_ref(), &rails);
        let flow = st.tree.as_ref().map(|t| {
            let copper: Vec<(String, f64)> =
                s.rails.iter().filter(|r| !r.ground).map(|r| (r.net.clone(), r.w)).collect();
            kisi_core::losses::power_flow(t, &copper, s.ground_w)
        });
        let m = kisi_core::losses::loss_map(b, &rails, &s.components, cell)?;
        let (mut lbuf, mut cbuf): (Vec<f32>, Vec<u32>) = (Vec::new(), Vec::new());
        let mut layers = Vec::new();
        for &l in &m.layers {
            let mut ll = kisi_core::losses::loss_layer(b, &rails, l);
            for p in ll.planes.iter_mut() {
                p.offset += lbuf.len();
            }
            lbuf.append(&mut ll.density);
            cbuf.append(&mut ll.cells);
            layers.push(ll);
        }
        let out = serde_json::json!({"summary": s, "flow": flow, "map": m, "layers": layers}).to_string();
        let mut buf = m.copper;
        buf.extend_from_slice(&m.components);
        Ok::<_, String>((out, buf, lbuf, cbuf))
    });
    match r {
        Ok((out, buf, lbuf, cbuf)) => {
            ST.with(|st| {
                let mut st = st.borrow_mut();
                st.loss_map = buf;
                st.loss_layer = lbuf;
                st.loss_cells = cbuf;
            });
            set_out(out);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}
#[no_mangle]
pub extern "C" fn kisi_pi_loss_cells_ptr() -> *const u32 {
    ST.with(|st| st.borrow().loss_cells.as_ptr())
}
#[no_mangle]
pub extern "C" fn kisi_pi_loss_cells_len() -> usize {
    ST.with(|st| st.borrow().loss_cells.len())
}
#[no_mangle]
pub extern "C" fn kisi_pi_loss_layer_ptr() -> *const f32 {
    ST.with(|st| st.borrow().loss_layer.as_ptr())
}
#[no_mangle]
pub extern "C" fn kisi_pi_loss_layer_len() -> usize {
    ST.with(|st| st.borrow().loss_layer.len())
}
#[no_mangle]
pub extern "C" fn kisi_pi_map_ptr() -> *const f32 {
    ST.with(|st| st.borrow().loss_map.as_ptr())
}
#[no_mangle]
pub extern "C" fn kisi_pi_map_len() -> usize {
    ST.with(|st| st.borrow().loss_map.len())
}

/// Temperature estimate from the stored DC results (copper loss) and the last power tree (part
/// heat). Input: ThermalParams {ambient_c, h, airflow_m_s, vertical}. Out: ThermalResult (rails
/// empty: `kisi_pi_thermal_rail` per rail); the temperature planes (°C, NaN outside the board) via
/// kisi_pi_temp_ptr/len: one per copper layer, nx*ny cells each.
#[no_mangle]
pub extern "C" fn kisi_pi_thermal(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let prm: kisi_core::thermal::ThermalParams = serde_json::from_str(&txt).unwrap_or_default();
    reply(ST.with(|st| -> Result<String, String> {
        let th = {
            let st = st.borrow();
            let b = st.board.as_ref().ok_or("no board loaded")?;
            let rails: Vec<&kisi_core::pi::PiResult> = st.pi_store.values().collect();
            if rails.is_empty() && st.tree.is_none() {
                return Err("no power results: compute the voltage drop first".into());
            }
            let ls = kisi_core::losses::loss_summary(b, st.tree.as_ref(), &rails);
            let mut th = kisi_core::thermal::solve(b, &rails, &ls.components, &prm)?;
            if rails.is_empty() {
                // the viewer puts the DC results back first; never a temperature without copper loss unsaid
                th.warnings.push("no DC results in the engine: the copper losses are not included".into());
            }
            th
        };
        let out = serde_json::to_string(&th).unwrap();
        st.borrow_mut().thermal = Some(th);
        Ok(out)
    }))
}

/// Copper temperature and IR drop at temperature of one rail (one electro-thermal step), from
/// its stored DC result and the last temperature estimate. Input: {"setup": RailSetup, "params":
/// PiParams} → RailThermal.
#[no_mangle]
pub extern "C" fn kisi_pi_thermal_rail(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply(ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let setup: kisi_core::pi::RailSetup = serde_json::from_value(v["setup"].clone()).map_err(|e| e.to_string())?;
        let params: kisi_core::pi::PiParams = serde_json::from_value(v["params"].clone()).unwrap_or_default();
        let th = st.thermal.as_ref().ok_or("no temperature estimate")?;
        let base = st.pi_store.get(&setup.net).ok_or_else(|| format!("{}: no DC result", setup.net))?;
        Ok(serde_json::to_string(&kisi_core::thermal::rail_at_temperature(b, &setup, base, th, &params)).unwrap())
    }))
}
#[no_mangle]
pub extern "C" fn kisi_pi_temp_ptr() -> *const f32 {
    ST.with(|st| st.borrow().thermal.as_ref().map(|t| t.t.as_ptr()).unwrap_or(std::ptr::null()))
}
#[no_mangle]
pub extern "C" fn kisi_pi_temp_len() -> usize {
    ST.with(|st| st.borrow().thermal.as_ref().map(|t| t.t.len()).unwrap_or(0))
}

// --- high frequency (M3) -----------------------------------------------------------

/// Verdict of a channel against a profile: {"metrics", "profile", "limits"} → Verdict
#[no_mangle]
pub extern "C" fn kisi_judge(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = (|| -> Result<String, String> {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let m: kisi_core::profile::Metrics = serde_json::from_value(v["metrics"].clone()).map_err(|e| e.to_string())?;
        let p: kisi_core::profile::Profile = serde_json::from_value(v["profile"].clone()).map_err(|e| e.to_string())?;
        let l: kisi_core::profile::Limits = serde_json::from_value(v["limits"].clone()).unwrap_or_default();
        Ok(serde_json::to_string(&kisi_core::profile::judge(&m, &p, &l)).unwrap())
    })();
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Automatic profile of a net from the built-in rules: {"net", "class", "diff"} → "id"
#[no_mangle]
pub extern "C" fn kisi_profile_auto(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or_default();
    let ps = PROFILES.with(|p| p.clone());
    let id = ps
        .auto(v["net"].as_str().unwrap_or(""), v["class"].as_str().unwrap_or(""), v["diff"].as_bool().unwrap_or(false))
        .to_string();
    set_out(id);
    0
}

thread_local! {
    static PROFILES: kisi_core::profile::Profiles = kisi_core::profile::Profiles::builtin();
}

/// Power tree from PI_REG / PI_LOAD fields: {"spec": TreeSpec, "warnings": [..]} or null.
#[no_mangle]
pub extern "C" fn kisi_pi_tree_fields() -> i32 {
    let s = ST.with(|st| {
        let st = st.borrow();
        match st.board.as_ref().and_then(kisi_core::tree::tree_from_fields) {
            Some((spec, w)) => serde_json::json!({"spec": spec, "warnings": w}).to_string(),
            None => "null".to_string(),
        }
    });
    set_out(s);
    0
}

/// AC impedance of a rail. Input: {"setup": RailSetup, "params": PdnParams} → PdnResult
#[no_mangle]
pub extern "C" fn kisi_pdn(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let setup: kisi_core::pi::RailSetup = serde_json::from_value(v["setup"].clone()).map_err(|e| e.to_string())?;
        let pp: kisi_core::pdn::PdnParams = serde_json::from_value(v["params"].clone()).unwrap_or_default();
        let res = kisi_core::pdn::pdn(b, &setup, &pp)?;
        Ok(serde_json::to_string(&res).unwrap())
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// PDN seen from every IC of a rail domain. Input: {"setup": RailSetup, "setups": \[RailSetup\] (all
/// rails, for the nets behind series parts), "params": PdnParams} → IcPdnResult
#[no_mangle]
pub extern "C" fn kisi_pdn_ic(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let setup: kisi_core::pi::RailSetup = serde_json::from_value(v["setup"].clone()).map_err(|e| e.to_string())?;
        let pp: kisi_core::pdn::PdnParams = serde_json::from_value(v["params"].clone()).unwrap_or_default();
        let all: Vec<kisi_core::pi::RailSetup> = serde_json::from_value(v["setups"].clone()).unwrap_or_default();
        Ok(serde_json::to_string(&kisi_core::pdn_ic::pdn_ic(b, &setup, &all, &pp)?).unwrap())
    });
    reply(r)
}

/// Channel check of many nets (eye, loss at Nyquist, skew). Input:
/// {"rf": RfParams, "items": [{"net": id, "gbps": f, "target": [z, diff] | null}]} → \[CheckRow\]
#[no_mangle]
pub extern "C" fn kisi_check(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let an = st.an.as_ref().ok_or("no analysis")?;
        let params = st.params.as_ref().ok_or("no analysis")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let rp: kisi_core::rf::RfParams = serde_json::from_value(v["rf"].clone()).unwrap_or_default();
        let mut rows: Vec<serde_json::Value> = Vec::new();
        for item in v["items"].as_array().ok_or("items missing")? {
            let net = item["net"].as_u64().unwrap_or(0) as u32;
            let gbps = item["gbps"].as_f64().unwrap_or(1.0);
            let target = item["target"].as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_bool()?)));
            let end = |k: &str| {
                item[k]
                    .as_object()
                    .and_then(|_| serde_json::from_value::<kisi_core::eye::EndModel>(item[k].clone()).ok())
            };
            let rise = item["rise"].as_f64();
            if let Some(start) = item["lane"].as_u64() {
                // device-to-device lane (see kisi_buses), identified by its first net
                let lane = st.buses.as_ref().and_then(|bs| {
                    bs.iter().flat_map(|b| &b.lanes).find(|l| {
                        l.nets[0] as u64 == start
                            && item["to"].as_str().is_none_or(|t| l.ends.last().is_some_and(|e| e.refdes == t))
                    })
                });
                let Some(lane) = lane else {
                    rows.push(serde_json::json!({"net": net, "name": "", "error": "lane not found"}));
                    continue;
                };
                let cd = kisi_core::topology::ComponentData::builtin();
                let rev = item["reversed"].as_bool().unwrap_or(lane.rx_end == 0);
                let cend: Option<kisi_core::eye::ConnEnd> =
                    item["conn_end"].as_object().and_then(|_| serde_json::from_value(item["conn_end"].clone()).ok());
                let cts = ts_for(&st, cend.as_ref());
                let xt = xt_for(&st, lane, cend.as_ref());
                rows.push(
                    serde_json::to_value(kisi_core::topology::check_lane(
                        b,
                        an,
                        &params.field,
                        lane,
                        target,
                        &rp,
                        gbps,
                        (end("tx"), end("rx"), rise),
                        rev,
                        cend.as_ref(),
                        cts,
                        &xt,
                        &cd,
                    ))
                    .unwrap(),
                );
                continue;
            }
            rows.push(
                serde_json::to_value(kisi_core::eye::check_net(
                    b,
                    an,
                    &params.field,
                    net,
                    target,
                    &rp,
                    gbps,
                    (end("tx"), end("rx"), rise),
                ))
                .unwrap(),
            );
        }
        Ok(serde_json::to_string(&rows).unwrap())
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Device-to-device lanes grouped into buses (topology through series parts and
/// terminations). Input: the analysis parameters (targets). Works before the
/// impedance estimate (uses the board only).
#[no_mangle]
pub extern "C" fn kisi_buses(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let params: Params = serde_json::from_str(&txt).unwrap_or_default();
    let r = ST.with(|st| -> Result<String, String> {
        let mut st = st.borrow_mut();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let sk = kisi_core::extract::skeleton(b, &params);
        let cd = kisi_core::topology::ComponentData::builtin();
        let buses = kisi_core::topology::buses(b, &sk, &cd);
        let js = serde_json::to_string(&buses).unwrap();
        st.buses = Some(buses);
        Ok(js)
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Channel of one lane. Input: {"lane": first net id, "to": refdes of the far end,
/// "reversed": bool | null, "rf", "target", "bitrate", "tx", "rx", "eye_rise", "prbs"}.
/// Output like kisi_rf plus "rx_note".
#[no_mangle]
pub extern "C" fn kisi_lane_rf(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let an = st.an.as_ref().ok_or("no analysis")?;
        let params = st.params.as_ref().ok_or("no analysis")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let start = v["lane"].as_u64().ok_or("lane missing")?;
        let lane = st
            .buses
            .as_ref()
            .and_then(|bs| bs.iter().flat_map(|b| &b.lanes).find(|l| l.nets[0] as u64 == start && v["to"].as_str().is_none_or(|t| l.ends.last().is_some_and(|e| e.refdes == t))))
            .ok_or("lane not found")?;
        let rp: kisi_core::rf::RfParams = serde_json::from_value(v["rf"].clone()).unwrap_or_default();
        let target = v["target"].as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_bool()?)));
        let end = |k: &str| v[k].as_object().and_then(|_| serde_json::from_value::<kisi_core::eye::EndModel>(v[k].clone()).ok());
        let rev = v["reversed"].as_bool().unwrap_or(lane.rx_end == 0);
        let cd = kisi_core::topology::ComponentData::builtin();
        let ep = v["bitrate"].as_f64().map(|br| (br, v["eye_rise"].as_f64().unwrap_or(rp.rise), v["prbs"].as_u64().unwrap_or(7) as u32, end("tx"), end("rx")));
        let cend: Option<kisi_core::eye::ConnEnd> = v["conn_end"].as_object().and_then(|_| serde_json::from_value(v["conn_end"].clone()).ok());
        let cts = ts_for(&st, cend.as_ref());
        let xt = xt_for(&st, lane, cend.as_ref());
        let out = match kisi_core::topology::lane_eval(b, an, &params.field, lane, target, &rp, ep, rev, cend.as_ref(), cts, &xt, &cd) {
            Ok(ev) => {
                let eye = match ev.eye {
                    Some(Ok(e)) => serde_json::to_value(&e).unwrap(),
                    Some(Err(e)) => serde_json::json!({"error": e}),
                    None => serde_json::Value::Null,
                };
                let skew = ev.skew.map(|(ps, mm, pn, unc)| serde_json::json!({"partner": pn, "skew_ps": ps, "skew_mm": mm, "uncertain": unc}));
                serde_json::json!({"channel": ev.ch, "error": null, "touchstone": kisi_core::rf::touchstone(&ev.ch), "eye": eye, "skew": skew,
                    "rx_used": ev.rx_used, "rx_note": ev.rx_note, "tx_used": ev.tx_used, "end_note": ev.end_note, "rest_applied": ev.rest_applied, "reversed": rev,
                    "model": ev.model, "scd21_db": ev.scd21_db, "receivers": ev.receivers,
                    "crosstalk": kisi_core::rf::crosstalk(b, an, lane.nets[0], &params.field, &rp)})
            }
            Err(e) => serde_json::json!({"channel": null, "error": e, "crosstalk": {"aggressors": [], "geoms": []}}),
        };
        Ok(out.to_string())
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Export of a lane or a bus for circuit and RF simulators (`kisi export`, `kisi_core::channel_export`)
/// with the viewer's settings. Input: {"board": file name, "formats": "all", "kicad_style":
/// "detailed" | "compact", "model": "ladder" | "compact", "fmax": GHz | "profile" | null, "fmin_ghz": GHz | null, "bus": name | null,
/// "devices": \[refdes\], "lanes": [{"lane": first net id, "to", "reversed", "rf", "target",
/// "profile", "bitrate", "tx", "rx", "eye_rise", "swing_v", "conn_end"}]} → {"summary", "files":
/// [{"path", "content"}], "name"}.
#[no_mangle]
pub extern "C" fn kisi_export(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        use kisi_core::channel_export::{bus_result, export_req, lane_result, LaneReq};
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let an = st.an.as_ref().ok_or("no analysis")?;
        let params = st.params.as_ref().ok_or("no analysis")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let mut f = kisi_core::export::files::Formats::parse(v["formats"].as_str().unwrap_or("all"))?;
        f.kicad_compact = v["kicad_style"].as_str() == Some("compact");
        f.model = kisi_core::export::Model::parse(v["model"].as_str().unwrap_or("compact"))?;
        let board = v["board"].as_str().unwrap_or("board.kicad_pcb").to_string();
        let bus = v["bus"].as_str().map(str::to_string);
        let reqs = v["lanes"].as_array().ok_or("lanes missing")?;
        // the export's frequency range: "fmax" in GHz or "profile", "fmin_ghz"
        let fmax_s = match &v["fmax"] {
            serde_json::Value::Number(x) => Some(x.to_string()),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        };
        let fmin_s = v["fmin_ghz"].as_f64().map(|x| x.to_string());
        let mut freq = kisi_core::channel_export::FreqRange::parse(fmin_s.as_deref(), fmax_s.as_deref())?;
        if bus.is_some() {
            // one grid for the bus's N-port: the highest f max of its lanes
            let rp: kisi_core::rf::RfParams =
                reqs.first().and_then(|i| serde_json::from_value(i["rf"].clone()).ok()).unwrap_or_default();
            let rates: Vec<(f64, f64)> = reqs
                .iter()
                .map(|i| (i["bitrate"].as_f64().unwrap_or(1e9) / 1e9, i["eye_rise"].as_f64().unwrap_or(100e-12)))
                .collect();
            freq = freq.for_bus(&rp, &rates);
        }
        let mut done = Vec::new();
        let mut skipped = Vec::new();
        let find_lane = |item: &serde_json::Value| -> Result<&kisi_core::topology::Lane, String> {
            let start = item["lane"].as_u64().ok_or("lane missing")?;
            st.buses
                .as_ref()
                .and_then(|bs| {
                    bs.iter().flat_map(|b| &b.lanes).find(|l| {
                        l.nets[0] as u64 == start
                            && item["to"].as_str().is_none_or(|t| l.ends.last().is_some_and(|e| e.refdes == t))
                    })
                })
                .ok_or_else(|| "lane not found".to_string())
        };
        let lanes: Vec<&kisi_core::topology::Lane> = reqs.iter().map(&find_lane).collect::<Result<_, _>>()?;
        // the option "fab stackups & tolerances" on: the stackup chosen in the Compare & tolerances view
        // (a fab template or an edited copy over the board's) and, on request, the tolerance corners
        let tol = if v["tolerance"].is_null() { None } else { Some(tolset_of(&v["tolerance"])?) };
        // the board as analysed: with the option on, the active stackup (the Stackup view's choice)
        let (eb, ea) = (b, an);
        let cs = match (&tol, v["corners"].as_bool() == Some(true)) {
            (Some(t), true) => kisi_core::channel_export::corners(eb, params, &t.tol),
            _ => Vec::new(),
        };
        // the corners' cross-sections of the exported lanes that the field solver should add first, so
        // that every variant stands at the level of the board's own (only where its lanes have field results)
        if v["no_field"].as_bool() != Some(true) && !cs.is_empty() {
            let own = kisi_core::channel_export::lane_geoms(b, an, &lanes);
            if own.keys().any(|id| params.field.contains_key(id)) {
                let need = kisi_core::channel_export::corner_geoms(&cs, &lanes, &params.field);
                if !need.is_empty() {
                    return Ok(serde_json::json!({ "field_needed": need }).to_string());
                }
            }
        }
        for (item, lane) in reqs.iter().zip(lanes.iter().copied()) {
            let end = |k: &str| serde_json::from_value::<kisi_core::eye::EndModel>(item[k].clone()).unwrap_or_default();
            let cend: Option<kisi_core::eye::ConnEnd> =
                item["conn_end"].as_object().and_then(|_| serde_json::from_value(item["conn_end"].clone()).ok());
            let req = LaneReq {
                lane,
                target: item["target"].as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_bool()?))),
                rp: serde_json::from_value(item["rf"].clone()).unwrap_or_default(),
                profile: item["profile"].as_str().unwrap_or("").to_string(),
                gbps: item["bitrate"].as_f64().unwrap_or(1e9) / 1e9,
                rise: item["eye_rise"].as_f64().unwrap_or(100e-12),
                swing: item["swing_v"].as_f64().unwrap_or(0.4),
                tx: end("tx"),
                rx: end("rx"),
                reversed: item["reversed"].as_bool().unwrap_or(lane.rx_end == 0),
                conn_end: cend.as_ref(),
                freq,
            };
            let dir =
                if bus.is_some() { format!("{}/", kisi_core::export::files::stem(&lane.name)) } else { String::new() };
            match export_req(eb, ea, &params.field, &req, f, &dir, &board, &cs) {
                Ok(d) => done.push(d),
                Err(e) if bus.is_some() => skipped.push(format!("{}: {e}", lane.name)),
                Err(e) => return Err(e),
            }
        }
        let (summary, files, name) = match &bus {
            Some(bn) => {
                let devices: Vec<String> = v["devices"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                bus_result(bn, &devices, done, skipped, f, &board)?
            }
            None => lane_result(done.pop().ok_or("no lane")?, ""),
        };
        let mut files = files;
        let classes: Vec<String> = summary["lanes"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| class_of(&st, l["lane"].as_str()?)).collect())
            .unwrap_or_default();
        let (short, full) = export_header(&st, eb, ea, tol.as_ref(), &classes);
        kisi_core::channel_export::stamp(&mut files, &short, &full);
        Ok(serde_json::json!({"summary": summary, "files": files, "name": name}).to_string())
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Path options of a request: {"flip": bool, "hint": [x, y] | null} (start end near `hint`).
fn path_opts(v: &serde_json::Value) -> (bool, Option<kisi_core::geom::P>) {
    let flip = v["flip"].as_bool().unwrap_or(false);
    let hint = v["hint"].as_array().and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]));
    (flip, hint)
}

/// Main path of a net with ends and branches: {"net": id, "flip", "hint"} → NetPath | null
#[no_mangle]
pub extern "C" fn kisi_net_path(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let an = st.an.as_ref().ok_or("no analysis")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let net = v["net"].as_u64().ok_or("net missing")? as u32;
        let (flip, hint) = path_opts(&v);
        Ok(serde_json::to_string(&kisi_core::rf::net_path_info(b, an, net, flip, hint)).unwrap())
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// Stage 1 of the coupling analysis for several nets (formula estimate, or the field solver where
/// it has already solved). Input: {"nets": [id…], "rf": RfParams, "threshold": 0.01}.
/// Output: XtScan {rows, threshold, critical_geoms}.
#[no_mangle]
pub extern "C" fn kisi_xt_scan(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let an = st.an.as_ref().ok_or("no analysis")?;
        let params = st.params.as_ref().ok_or("no analysis")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let rp: kisi_core::rf::RfParams = serde_json::from_value(v["rf"].clone()).unwrap_or_default();
        let nets: Vec<u32> = v["nets"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as u32).collect())
            .unwrap_or_default();
        let th = v["threshold"].as_f64().unwrap_or(kisi_core::rf::XT_CRITICAL);
        Ok(serde_json::to_string(&kisi_core::rf::crosstalk_scan(b, an, &nets, &params.field, &rp, th)).unwrap())
    });
    reply(r)
}

/// Channel and crosstalk of one net. Input:
/// {"net": id, "rf": RfParams, "target": [z, diff] | null, "bitrate": f | null, "prbs": 7,
///  "flip": bool, "hint": [x, y] | null, "mixed": bool (pairs: also the 4-port model)}
/// Output: {"channel", "error", "crosstalk", "touchstone", "eye": Eye | null, "skew": {...} | null,
///          "mixed": MixedMode | null (pairs: 4-port model), "touchstone4": ".s4p text" | ""}
#[no_mangle]
pub extern "C" fn kisi_rf(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let an = st.an.as_ref().ok_or("no analysis")?;
        let params = st.params.as_ref().ok_or("no analysis")?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let net = v["net"].as_u64().ok_or("net missing")? as u32;
        let rp: kisi_core::rf::RfParams = serde_json::from_value(v["rf"].clone()).unwrap_or_default();
        let target = v["target"].as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_bool()?)));
        let (flip, hint) = path_opts(&v);
        let ch = kisi_core::rf::channel_dir(b, an, net, &params.field, target, &rp, flip, hint);
        let xt = kisi_core::rf::crosstalk_dir(b, an, net, &params.field, &rp, flip, hint);
        let mut eye = serde_json::Value::Null;
        let mut skew = serde_json::Value::Null;
        let mut mixed = serde_json::Value::Null;
        let mut ts4 = String::new();
        let (chv, err, ts) = match ch {
            Ok(c) => {
                let mut ts = kisi_core::rf::touchstone(&c);
                if let Some(br) = v["bitrate"].as_f64() {
                    let end = |k: &str| v[k].as_object().and_then(|_| serde_json::from_value::<kisi_core::eye::EndModel>(v[k].clone()).ok());
                    let ep = kisi_core::eye::EyeParams {
                        bitrate: br,
                        rise: v["eye_rise"].as_f64().unwrap_or(rp.rise),
                        prbs: v["prbs"].as_u64().unwrap_or(7) as u32,
                        tx: end("tx"),
                        rx: end("rx"),
                        ..Default::default()
                    };
                    // a pair: the eye of the 4-port (both lines, mode conversion), as in the check
                    let res = match kisi_core::rf::mixed_mode(b, an, net, &params.field, target, &rp, flip, hint).ok().filter(|_| c.diff) {
                        Some(m) => {
                            let d = kisi_core::eye::EndModel { r: c.z_ref, c_pf: 0.0, l_nh: 0.0 };
                            kisi_core::lanenet::ends_transfer(&c.freq, &[m.pair_net().block()], 2, &ep.tx.unwrap_or(d), &ep.rx.unwrap_or(d)).and_then(|(h, dc)| kisi_core::eye::eye_of(&c.freq, &h, Some(dc), c.delay_ps, &ep))
                        }
                        None => kisi_core::eye::eye(&c, &ep),
                    };
                    eye = match res {
                        Ok(e) => serde_json::to_value(&e).unwrap(),
                        Err(e) => serde_json::json!({"error": e}),
                    };
                }
                if c.diff && v["mixed"].as_bool().unwrap_or(false) {
                    mixed = match kisi_core::rf::mixed_mode(b, an, net, &params.field, target, &rp, flip, hint) {
                        Ok(m) => {
                            ts4 = kisi_core::rf::touchstone4(&m);
                            // the .s2p of a pair is Sdd of the 4-port (matches the plotted curves)
                            ts = kisi_core::rf::touchstone_dd(&m);
                            serde_json::to_value(&m).unwrap()
                        }
                        Err(e) => serde_json::json!({"error": e}),
                    };
                }
                if let (true, Some(pn)) = (c.diff, c.partner) {
                    let rq = kisi_core::rf::RfParams { npts: 10, ..rp.clone() };
                    if let Ok(pc) = kisi_core::rf::channel_dir(b, an, pn, &params.field, target, &rq, flip, hint) {
                        let branched = |c: &kisi_core::rf::Channel| c.path.as_ref().is_some_and(|p| !p.branches.is_empty() || p.parts > 1);
                        skew = serde_json::json!({"partner": pc.name, "delay_ps": pc.delay_ps, "length_mm": pc.length_mm,
                            "skew_ps": c.delay_ps - pc.delay_ps, "skew_mm": c.length_mm - pc.length_mm,
                            "uncertain": branched(&c) || branched(&pc)});
                    }
                }
                (serde_json::to_value(&c).unwrap(), serde_json::Value::Null, ts)
            }
            Err(e) => (serde_json::Value::Null, serde_json::Value::String(e), String::new()),
        };
        // the stackup used at the head of the Touchstone files the viewer saves
        let cls: Vec<String> = b.nets.get(net as usize).and_then(|n| class_of(&st, n)).into_iter().collect();
        let tol = if v["tolerance"].is_null() { None } else { tolset_of(&v["tolerance"]).ok() };
        let head = export_header(&st, b, an, tol.as_ref(), &cls);
        let stamp = |name: &str, t: String| {
            if t.is_empty() {
                return t;
            }
            let mut f = [kisi_core::export::files::ExportFile { path: name.into(), content: t }];
            kisi_core::channel_export::stamp(&mut f, &head.0, &head.1);
            let [f] = f;
            f.content
        };
        let (ts, ts4) = (stamp("net.s2p", ts), stamp("net.s4p", ts4));
        Ok(serde_json::json!({"channel": chv, "error": err, "crosstalk": xt, "touchstone": ts, "eye": eye, "skew": skew, "mixed": mixed, "touchstone4": ts4}).to_string())
    });
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

// --- command line (kisi.mjs) ------------------------------------------------------------
// The Node CLI reads files and runs the field solver; parsing, analyses and JSON results
// come from kisi_core::commands, exactly as in the native CLI.

thread_local! {
    static CLI: RefCell<Option<kisi_core::service::Project>> = const { RefCell::new(None) };
}

fn reply(r: Result<String, String>) -> i32 {
    match r {
        Ok(s) => {
            set_out(s);
            0
        }
        Err(e) => {
            set_out(e);
            1
        }
    }
}

/// The comment block of the viewer's exports: the stackup used with the source of every value (board
/// file, kisi.json, the viewer's edits, the active stackup's template) and, with the option "fab stackups & tolerances" on (`tol`), the band of `classes`.
fn export_header(
    st: &State,
    b: &Board,
    an: &kisi_core::extract::Analysis,
    tol: Option<&kisi_core::stackups::TolSet>,
    classes: &[String],
) -> (Vec<String>, Vec<String>) {
    // the active stackup's template and changes, else the Stackup card's changes
    let (preset, edits) = st.active.clone().unwrap_or_default();
    let ov = edits
        .as_ref()
        .filter(|o| !o.is_empty())
        .or(Some(&st.stack_ov))
        .filter(|o| !o.is_empty())
        .map(|o| (o, "viewer edit"));
    let prov = kisi_core::stackups::provenance(b.stackup(), st.cfg.as_ref(), preset.as_deref(), ov);
    let (mut short, mut full) =
        (kisi_core::stackups::header_short(b.stackup(), &prov), kisi_core::stackups::header_lines(b.stackup(), &prov));
    if let (Some(t), Some(params)) = (tol, st.params.as_ref()) {
        let names: Vec<String> = an.classes.iter().map(|c| c.name.clone()).collect();
        let assign = st
            .cfg
            .as_ref()
            .map(|c| c.net_classes.iter().filter_map(|(k, e)| Some((k.clone(), e.profile.clone()?))).collect())
            .unwrap_or_default();
        let w = kisi_core::stackups::windows(&names, &assign, &kisi_core::profile::Profiles::builtin());
        let col = kisi_core::stackups::Column {
            id: "board".into(),
            label: "board".into(),
            source: String::new(),
            stackup: b.stackup().clone(),
        };
        let (study, _) = kisi_core::stackups::study_json(b, an, &params.field, &[col], Some((t, 0)), &w);
        let tl = kisi_core::stackups::tolerance_lines(&study, Some(classes));
        short.extend(tl.iter().cloned());
        full.extend(tl);
    }
    (short, full)
}

/// The viewer's tolerances: the values (`Tolerances`) and where each comes from (`from`:
/// {key: "default" | "kisi.json" | "viewer"}).
fn tolset_of(v: &serde_json::Value) -> Result<kisi_core::stackups::TolSet, String> {
    let tol: kisi_core::stackups::Tolerances =
        serde_json::from_value(v.clone()).map_err(|e| format!("tolerance: {e}"))?;
    let mut t = kisi_core::stackups::TolSet::defaults();
    t.tol = tol;
    for k in kisi_core::stackups::TOL_KEYS {
        if let Some(f) = v["from"][k].as_str() {
            t.from.insert(k, f.to_string());
        }
    }
    Ok(t)
}

/// The net class of a net (by name).
fn class_of(st: &State, net: &str) -> Option<String> {
    let b = st.board.as_ref()?;
    let id = b.nets.iter().position(|n| n == net)?;
    let ci = b.net_class.get(id).copied().unwrap_or(0) as usize;
    st.an.as_ref()?.classes.get(ci).map(|c| c.name.clone())
}

/// Fab stackup templates (`kisi stackups`) with their fit to the loaded board (its stackup with the
/// configuration, before the viewer's edits) and each one's `stackup` entry for kisi.json:
/// {"presets": [{id, fab, template, title, layers, nominal_mm, …, fit, kisi_json}], "board", "fitting"}.
#[no_mangle]
pub extern "C" fn kisi_stackup_presets() -> i32 {
    reply(ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let su = st.stackup_file.as_ref();
        let mut v = kisi_core::stackups::list_json(su);
        if let Some(list) = v["presets"].as_array_mut() {
            for x in list.iter_mut() {
                if let Some(p) = kisi_core::stackups::library().get(x["id"].as_str().unwrap_or("")) {
                    let cfg = kisi_core::config::BoardConfig { stackup: p.to_entry(), ..Default::default() };
                    x["kisi_json"] = kisi_core::config::to_json(&cfg)["stackup"].clone();
                }
            }
        }
        Ok(v.to_string())
    }))
}

/// Stackup comparison and tolerance band on the last analysis (`kisi stackups`, `overview
/// --tolerance`). Input: {"columns": [{"id", "label", "preset": id | null, "edits": StackupOverride |
/// null}] – the board's stackup as analysed is column 0 and not listed; a preset is laid over it, then
/// the edits –, "tolerance": Tolerances | null, "tol_column": index, "assign": {class: profile id}}.
/// Output: the study (columns, rows with cells and the band) and "field_needed": [[id, text, weight]],
/// the cross-sections the field solver would add.
#[no_mangle]
pub extern "C" fn kisi_stackup_study(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply(ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let b = st.board.as_ref().ok_or("no board loaded")?;
        let an = st.an.as_ref().ok_or("the impedance estimate first (step 2)")?;
        let params = st.params.as_ref().ok_or("the impedance estimate first (step 2)")?;
        // the board's own stackup, also while another one is active in all views
        let su = st.stack_base.as_ref().unwrap_or(b.stackup());
        let lib = kisi_core::stackups::library();
        let mut cols = vec![kisi_core::stackups::Column {
            id: "board".into(),
            label: "board".into(),
            source: v["board_source"].as_str().unwrap_or("board").to_string(),
            stackup: su.clone(),
        }];
        for c in v["columns"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let mut s = su.clone();
            let mut source = Vec::new();
            if let Some(id) = c["preset"].as_str() {
                let pr = lib.find(id)?;
                if let Some(why) = pr.fit(su).problem {
                    return Err(why);
                }
                s = pr.apply(&s);
                source.push(pr.document());
            }
            if !c["edits"].is_null() {
                let ov: kisi_core::board::StackupOverride =
                    serde_json::from_value(c["edits"].clone()).map_err(|e| format!("edits: {e}"))?;
                if !ov.is_empty() {
                    s = s.with_overrides(&ov);
                    source.push("edited in the viewer".into());
                }
            }
            cols.push(kisi_core::stackups::Column {
                id: c["id"].as_str().unwrap_or("").to_string(),
                label: c["label"].as_str().unwrap_or("").to_string(),
                source: if source.is_empty() { "copy of the board's stackup".into() } else { source.join(", ") },
                stackup: s,
            });
        }
        let tol = if v["tolerance"].is_null() { None } else { Some(tolset_of(&v["tolerance"])?) };
        let k = v["tol_column"].as_u64().unwrap_or(0) as usize;
        let assign: std::collections::HashMap<String, String> =
            serde_json::from_value(v["assign"].clone()).unwrap_or_default();
        let classes: Vec<String> = an.classes.iter().map(|c| c.name.clone()).collect();
        let w = kisi_core::stackups::windows(&classes, &assign, &kisi_core::profile::Profiles::builtin());
        let (mut out, need) = kisi_core::stackups::study_json(
            b,
            an,
            &params.field,
            &cols,
            tol.as_ref().map(|t| (t, k.min(cols.len() - 1))),
            &w,
        );
        out["field_needed"] = serde_json::json!(need);
        Ok(out.to_string())
    }))
}

/// The gap list (`kisi gaps`) of the board as the viewer read it, with a configuration: the loaded
/// kisi.json or a draft of the Prepare page. Input: {"board": file name, "config": text | null,
/// "for": category | null}. Uses the viewer's choice of assembly variant and the files dropped with
/// the configuration (BOMs). Output: the `kisi gaps` JSON (with "config": the check of the text).
#[no_mangle]
pub extern "C" fn kisi_gaps(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply(ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let b = st.board_base.clone().ok_or("no board loaded")?;
        let name = v["board"].as_str().unwrap_or("board.kicad_pcb").to_string();
        let stem = name.trim_end_matches(".kicad_pcb").to_string();
        let config =
            v["config"].as_str().map(|t| (std::path::PathBuf::from(format!("{stem}.kisi.json")), Ok(t.to_string())));
        let inp = kisi_core::service::Inputs {
            pcb_path: std::path::PathBuf::from(&name),
            pcb: String::new(),
            pro: None,
            config,
            stackup: None,
            stackup_preset: None,
            tol: None,
            field: Default::default(),
            variant: st.variant_choice.clone(),
            files: st.files.clone(),
        };
        let proj = kisi_core::service::Project::from_board(b, inp)?;
        let mut out = kisi_core::gaps::gaps(&proj, &kisi_core::profile::Profiles::builtin());
        if let Some(view) = v["for"].as_str() {
            kisi_core::gaps::only_view(&mut out, view);
        }
        Ok(out.to_string())
    }))
}

/// Check a configuration text against the board as read, without applying it (the Prepare page's
/// live check). Input: {"board": file name, "config": text}. Output: {ok, entries, errors,
/// warnings, messages: [{level, text, views}]}. Dismissals that cannot apply on the analysed board
/// are reported by `kisi_gaps` (its "config").
#[no_mangle]
pub extern "C" fn kisi_config_check(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply(ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let b = st.board_base.as_ref().ok_or("no board loaded")?;
        let cfg = match kisi_core::config::parse(v["config"].as_str().ok_or("config missing")?) {
            Ok(c) => c,
            Err(e) => return Ok(serde_json::json!({"ok": false, "entries": 0, "errors": [e], "warnings": [], "messages": [{"level": "error", "text": e, "views": kisi_core::gaps::VIEWS}]}).to_string()),
        };
        let an0 = kisi_core::extract::skeleton(b, &Params::default());
        let classes: Vec<String> = an0.classes.iter().map(|c| c.name.clone()).collect();
        let c = kisi_core::config::check(&cfg, b, &classes, &kisi_core::profile::Profiles::builtin(), v["board"].as_str(), &st.files);
        let msgs: Vec<serde_json::Value> = c.errors.iter().map(|t| ("error", t)).chain(c.warnings.iter().map(|t| ("warning", t))).map(|(l, t)| serde_json::json!({"level": l, "text": t, "views": kisi_core::config::message_views(&cfg, t)})).collect();
        Ok(serde_json::json!({"ok": c.ok, "entries": c.entries, "errors": c.errors, "warnings": c.warnings, "messages": msgs}).to_string())
    }))
}

/// The entries of one category of a configuration ("save only this category"). Input:
/// {"config": text, "view": "impedance" | "channel" | "power"}. Output: the configuration (JSON).
#[no_mangle]
pub extern "C" fn kisi_config_subset(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply((|| -> Result<String, String> {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let cfg = kisi_core::config::parse(v["config"].as_str().ok_or("config missing")?)?;
        let view = v["view"].as_str().ok_or("view missing")?;
        if !kisi_core::gaps::VIEWS.contains(&view) {
            return Err(format!("view: one of {}", kisi_core::gaps::VIEWS.join(", ")));
        }
        serde_json::to_string_pretty(&kisi_core::config::to_json(&kisi_core::config::subset(&cfg, view)))
            .map_err(|e| e.to_string())
    })())
}

/// Power settings of the viewer as footprint fields for the kisi.json (Prepare page, "take the viewer's
/// edits"): a power tree as PI_REG / PI_LOAD, rail setups without a tree as PI_SOURCE / PI_LOAD.
/// Input: {"tree": TreeSpec | null, "setups": \[RailSetup\] | null}. Output: {"fields": [{ref, name,
/// value}], "lost": [what fields cannot say]}.
#[no_mangle]
pub extern "C" fn kisi_edits_to_fields(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply((|| -> Result<String, String> {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let mut fields = Vec::new();
        let mut lost = Vec::new();
        if !v["tree"].is_null() {
            let spec: kisi_core::tree::TreeSpec =
                serde_json::from_value(v["tree"].clone()).map_err(|e| format!("tree: {e}"))?;
            fields.extend(kisi_core::tree::spec_to_fields(&spec));
        }
        if !v["setups"].is_null() {
            let setups: Vec<kisi_core::pi::RailSetup> =
                serde_json::from_value(v["setups"].clone()).map_err(|e| format!("setups: {e}"))?;
            let (f, l) = kisi_core::pi::setups_to_fields(&setups);
            fields.extend(f);
            lost.extend(l);
        }
        let fields: Vec<serde_json::Value> =
            fields.into_iter().map(|(r, n, v)| serde_json::json!({"ref": r, "name": n, "value": v})).collect();
        Ok(serde_json::json!({"fields": fields, "lost": lost}).to_string())
    })())
}

/// The hierarchical sheets of the loaded board with their parts (for the Prepare page's variant
/// form): [{"sheet": "/Power/", "parts": 12}].
#[no_mangle]
pub extern "C" fn kisi_board_sheets() -> i32 {
    reply(ST.with(|st| -> Result<String, String> {
        let st = st.borrow();
        let b = st.board_base.as_ref().ok_or("no board loaded")?;
        let mut m: std::collections::BTreeMap<String, usize> = Default::default();
        for f in &b.footprints {
            let s = kisi_core::variant::norm_sheet(&f.sheet);
            if !s.is_empty() {
                *m.entry(s).or_default() += 1;
            }
        }
        Ok(serde_json::Value::Array(m.into_iter().map(|(s, n)| serde_json::json!({"sheet": s, "parts": n})).collect())
            .to_string())
    }))
}

/// Parse a command line: ["check", "board.kicad_pcb", "--all", …] → Opts.
#[no_mangle]
pub extern "C" fn kisi_cli_parse(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply((|| {
        let args: Vec<String> = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let o = kisi_core::commands::parse_args(&args)?;
        Ok(serde_json::json!({"opts": o, "needs_board": kisi_core::commands::wants_board(&o), "uses_field": kisi_core::commands::uses_field(&o.cmd), "help": kisi_core::commands::HELP}).to_string())
    })())
}

/// Build the project. Input: {"opts", "pcb_path", "pcb", "pro", "config": {"path", "text"|null, "error"|null} | null,
/// "stackup", "field": {id: FieldResult}}.
/// Output: {"plan": [[id, text, weight], …]} – the cross-sections the field solver should solve.
#[no_mangle]
pub extern "C" fn kisi_cli_project(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply((|| {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let o: kisi_core::commands::Opts = serde_json::from_value(v["opts"].clone()).map_err(|e| e.to_string())?;
        let s = |k: &str| v[k].as_str().map(str::to_string);
        let config = v["config"].as_object().map(|c| {
            let path = std::path::PathBuf::from(c.get("path").and_then(|x| x.as_str()).unwrap_or(""));
            let r = match (c.get("text").and_then(|x| x.as_str()), c.get("error").and_then(|x| x.as_str())) {
                (Some(t), _) => Ok(t.to_string()),
                (None, Some(e)) => Err(e.to_string()),
                _ => Err("not readable".to_string()),
            };
            (path, r)
        });
        // files the configuration refers to (BOMs): {path: {"text"} | {"error"}}
        let files: kisi_core::variant::Files = v["files"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, f)| {
                        (
                            k.clone(),
                            f["text"]
                                .as_str()
                                .map(str::to_string)
                                .ok_or_else(|| f["error"].as_str().unwrap_or("not readable").to_string()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let inp = kisi_core::service::Inputs {
            pcb_path: std::path::PathBuf::from(s("pcb_path").unwrap_or_default()),
            pcb: s("pcb").ok_or("pcb missing")?,
            pro: s("pro"),
            config,
            stackup: serde_json::from_value(v["stackup"].clone()).ok(),
            stackup_preset: o.stackup_preset.clone(),
            tol: kisi_core::commands::tol_arg(&o),
            field: serde_json::from_value(v["field"].clone()).unwrap_or_default(),
            variant: o.variant.clone(),
            files,
        };
        let proj = kisi_core::service::Project::from_texts(inp)?;
        let plan = kisi_core::commands::field_plan(&proj, &o);
        CLI.with(|c| *c.borrow_mut() = Some(proj));
        Ok(serde_json::json!({"plan": plan}).to_string())
    })())
}

/// Rows of `check` on the project, for worker engines: {"opts", "items": [index, …]} →
/// {"count": rows in all, "rows": [[index, row], …]} (no items: the count only).
#[no_mangle]
pub extern "C" fn kisi_cli_check_part(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply((|| {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let o: kisi_core::commands::Opts = serde_json::from_value(v["opts"].clone()).map_err(|e| e.to_string())?;
        let idx: Vec<usize> = v["items"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| Some(x.as_u64()? as usize)).collect())
            .unwrap_or_default();
        CLI.with(|c| {
            let c = c.borrow();
            let pr = c.as_ref().ok_or("no board loaded")?;
            let (count, rows) = kisi_core::commands::check_rows(&o, pr, &idx)?;
            Ok(serde_json::json!({"count": count, "rows": rows}).to_string())
        })
    })())
}

/// Run the command on the project. Input: {"opts", "tree": json|null, "report_path": str|null,
/// "check_rows": [[index, row], …] | null (check: the rows from the parts)}.
/// Output: the result; for `report` {"result": summary, "write": {"path", "content"}}.
#[no_mangle]
pub extern "C" fn kisi_cli_run(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply((|| {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let o: kisi_core::commands::Opts = serde_json::from_value(v["opts"].clone()).map_err(|e| e.to_string())?;
        let tree = if v["tree"].is_null() { None } else { Some(v["tree"].clone()) };
        // check rows computed in parts by worker engines (kisi_cli_check_part)
        let rows: Option<Vec<(usize, serde_json::Value)>> = if v["check_rows"].is_array() {
            Some(serde_json::from_value(v["check_rows"].clone()).map_err(|e| e.to_string())?)
        } else {
            None
        };
        CLI.with(|c| {
            let c = c.borrow();
            let r = kisi_core::commands::run_with(&o, c.as_ref(), tree, rows)?;
            if o.cmd == "report" {
                let path = v["report_path"].as_str().unwrap_or("analysis/kisi.json");
                return Ok(serde_json::json!({"result": kisi_core::commands::report_summary(path, &r), "write": {"path": path, "content": serde_json::to_string_pretty(&r).unwrap()}}).to_string());
            }
            Ok(r.to_string())
        })
    })())
}
