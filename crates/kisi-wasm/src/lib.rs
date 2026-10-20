//! WebAssembly bindings with a plain C ABI (no wasm-bindgen, no JS glue needed).
//!
//! Protocol: JS allocates with `kisi_alloc`, copies UTF-8 text in, calls a
//! function; results (JSON) are read from `kisi_out_ptr/len`.
//! Return codes: 0 = ok, 1 = error (message in out buffer).

use kisi_core::{analyze, apply_project, load_pcb, view, Board, Params};
use std::cell::RefCell;

#[derive(Default)]
struct State {
    board: Option<Board>,
    /// stackup as read from the file (overrides are applied to a copy)
    stackup_file: Option<kisi_core::board::Stackup>,
    pro: Option<String>,
    out: Vec<u8>,
    zone_pts: Vec<f32>,
    zone_idx: Vec<u32>,
    pi_v: Vec<f32>,
    pi_j: Vec<f32>,
    /// last solved power tree, and the DC results per rail (loss buffers only) for the losses
    tree: Option<kisi_core::tree::TreeResult>,
    pi_store: std::collections::BTreeMap<String, kisi_core::pi::PiResult>,
    /// loss map: copper planes, then the components plane (W/mm²)
    loss_map: Vec<f32>,
    /// last analysis and its parameters (for the RF functions)
    an: Option<kisi_core::Analysis>,
    params: Option<Params>,
    /// device-to-device lanes of the board (topology), from `kisi_buses`
    buses: Option<Vec<kisi_core::topology::Bus>>,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State::default());
}

fn input(p: *const u8, n: usize) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(p, n) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn set_out(s: String) {
    ST.with(|st| st.borrow_mut().out = s.into_bytes());
}

#[no_mangle]
pub extern "C" fn kisi_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[no_mangle]
pub extern "C" fn kisi_free(p: *mut u8, n: usize) {
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
                st.stackup_file = b.stackup.clone();
                st.board = Some(b);
                st.tree = None;
                st.pi_store.clear();
                st.loss_map.clear();
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
/// Input: {"config": file contents, "board": name of the loaded board}.
/// Output: {"check": {ok, errors, warnings, entries}, "targets": {class: {z, diff}},
/// "profiles": {class: id}, "fields": n, "dielectrics": n}. The board's fields and
/// file stackup are replaced by the configured values (viewer edits stay on top).
#[no_mangle]
pub extern "C" fn kisi_config(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let r = ST.with(|st| -> Result<String, String> {
        let mut st = st.borrow_mut();
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let cfg = kisi_core::config::parse(v["config"].as_str().ok_or("config missing")?)?;
        let board_name = v["board"].as_str().map(str::to_string);
        let b = st.board.as_mut().ok_or("no board loaded")?;
        let an0 = kisi_core::extract::skeleton(b, &Params::default());
        let classes: Vec<String> = an0.classes.iter().map(|c| c.name.clone()).collect();
        let check = kisi_core::config::check(&cfg, b, &classes, &kisi_core::profile::Profiles::builtin(), board_name.as_deref());
        let a = cfg.apply(b);
        let su = b.stackup.clone();
        st.stackup_file = su;
        let targets: serde_json::Map<String, serde_json::Value> = a.targets.iter().map(|(k, t)| (k.clone(), serde_json::json!({"z": t.z, "diff": t.diff}))).collect();
        Ok(serde_json::json!({"check": check, "targets": targets, "profiles": a.profiles, "fields": a.fields, "dielectrics": a.dielectrics, "notes": cfg.notes}).to_string())
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

/// Load a `.kicad_pro` (net classes). May be called before or after the board.
#[no_mangle]
pub extern "C" fn kisi_load_pro(p: *const u8, n: usize) -> i32 {
    let text = input(p, n);
    let r = ST.with(|st| {
        let mut st = st.borrow_mut();
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
    // optional stackup overrides: {"stackup": {"dielectrics": [{index, epsilon_r, loss_tangent, thickness}], "mask_epsilon_r"}}
    let ov: kisi_core::board::StackupOverride = serde_json::from_str::<serde_json::Value>(&txt)
        .ok()
        .and_then(|v| serde_json::from_value(v["stackup"].clone()).ok())
        .unwrap_or_default();
    let s = ST.with(|st| {
        let mut st = st.borrow_mut();
        let file = st.stackup_file.clone();
        if let (Some(b), Some(f)) = (st.board.as_mut(), file) {
            b.stackup = Some(f.with_overrides(&ov));
        }
        let an = st.board.as_ref().map(|b| analyze(b, &params))?;
        let js = serde_json::to_string(&an).unwrap();
        st.an = Some(an);
        st.params = Some(params);
        Some(js)
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

/// Board summary without impedances (net classes, targets, one piece per track):
/// lets the viewer show the board immediately; `kisi_analyze` adds the impedances.
#[no_mangle]
pub extern "C" fn kisi_skeleton(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    let params: Params = serde_json::from_str(&txt).unwrap_or_default();
    let s = ST.with(|st| {
        let mut st = st.borrow_mut();
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

/// Rail setups from a KiPIDA kipida_config.json: {rails, warnings}.
#[no_mangle]
pub extern "C" fn kisi_pi_kipida(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    with_board(|b| {
        let (rails, warnings) = kisi_core::pi::rails_from_kipida(b, &txt)?;
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
/// Out: {summary, flow, map}; the map planes (W/mm²) via kisi_pi_map_ptr/len:
/// `map.layers.len()` copper planes, then the components plane, nx*ny cells each.
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
            let copper: Vec<(String, f64)> = s.rails.iter().filter(|r| !r.ground).map(|r| (r.net.clone(), r.w)).collect();
            kisi_core::losses::power_flow(t, &copper, s.ground_w)
        });
        let m = kisi_core::losses::loss_map(b, &rails, &s.components, cell)?;
        let out = serde_json::json!({"summary": s, "flow": flow, "map": m}).to_string();
        let mut buf = m.copper;
        buf.extend_from_slice(&m.components);
        Ok::<_, String>((out, buf))
    });
    match r {
        Ok((out, buf)) => {
            ST.with(|st| st.borrow_mut().loss_map = buf);
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
pub extern "C" fn kisi_pi_map_ptr() -> *const f32 {
    ST.with(|st| st.borrow().loss_map.as_ptr())
}
#[no_mangle]
pub extern "C" fn kisi_pi_map_len() -> usize {
    ST.with(|st| st.borrow().loss_map.len())
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
    let id = ps.auto(v["net"].as_str().unwrap_or(""), v["class"].as_str().unwrap_or(""), v["diff"].as_bool().unwrap_or(false)).to_string();
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

/// PDN seen from every IC of a rail domain. Input: {"setup": RailSetup, "setups": [RailSetup] (all
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
/// {"rf": RfParams, "items": [{"net": id, "gbps": f, "target": [z, diff] | null}]} → [CheckRow]
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
            let end = |k: &str| item[k].as_object().and_then(|_| serde_json::from_value::<kisi_core::eye::EndModel>(item[k].clone()).ok());
            let rise = item["rise"].as_f64();
            if let Some(start) = item["lane"].as_u64() {
                // device-to-device lane (see kisi_buses), identified by its first net
                let lane = st.buses.as_ref().and_then(|bs| bs.iter().flat_map(|b| &b.lanes).find(|l| l.nets[0] as u64 == start && item["to"].as_str().map_or(true, |t| l.ends.last().is_some_and(|e| e.refdes == t))));
                let Some(lane) = lane else {
                    rows.push(serde_json::json!({"net": net, "name": "", "error": "lane not found"}));
                    continue;
                };
                let cd = kisi_core::topology::ComponentData::builtin();
                let rev = item["reversed"].as_bool().unwrap_or(lane.rx_end == 0);
                rows.push(serde_json::to_value(kisi_core::topology::check_lane(b, an, &params.field, lane, target, &rp, gbps, (end("tx"), end("rx"), rise), rev, &cd)).unwrap());
                continue;
            }
            rows.push(serde_json::to_value(kisi_core::eye::check_net(b, an, &params.field, net, target, &rp, gbps, (end("tx"), end("rx"), rise))).unwrap());
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
            .and_then(|bs| bs.iter().flat_map(|b| &b.lanes).find(|l| l.nets[0] as u64 == start && v["to"].as_str().map_or(true, |t| l.ends.last().is_some_and(|e| e.refdes == t))))
            .ok_or("lane not found")?;
        let rp: kisi_core::rf::RfParams = serde_json::from_value(v["rf"].clone()).unwrap_or_default();
        let target = v["target"].as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_bool()?)));
        let end = |k: &str| v[k].as_object().and_then(|_| serde_json::from_value::<kisi_core::eye::EndModel>(v[k].clone()).ok());
        let rev = v["reversed"].as_bool().unwrap_or(lane.rx_end == 0);
        let cd = kisi_core::topology::ComponentData::builtin();
        let ep = v["bitrate"].as_f64().map(|br| (br, v["eye_rise"].as_f64().unwrap_or(rp.rise), v["prbs"].as_u64().unwrap_or(7) as u32, end("tx"), end("rx")));
        let out = match kisi_core::topology::lane_eval(b, an, &params.field, lane, target, &rp, ep, rev, &cd) {
            Ok(ev) => {
                let eye = match ev.eye {
                    Some(Ok(e)) => serde_json::to_value(&e).unwrap(),
                    Some(Err(e)) => serde_json::json!({"error": e}),
                    None => serde_json::Value::Null,
                };
                let skew = ev.skew.map(|(ps, mm, pn, unc)| serde_json::json!({"partner": pn, "skew_ps": ps, "skew_mm": mm, "uncertain": unc}));
                serde_json::json!({"channel": ev.ch, "error": null, "touchstone": kisi_core::rf::touchstone(&ev.ch), "eye": eye, "skew": skew,
                    "rx_used": ev.rx_used, "rx_note": ev.rx_note, "reversed": rev,
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
        let nets: Vec<u32> = v["nets"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as u32).collect()).unwrap_or_default();
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
                    eye = match kisi_core::eye::eye(&c, &ep) {
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

/// Parse a command line: ["check", "board.kicad_pcb", "--all", …] → Opts.
#[no_mangle]
pub extern "C" fn kisi_cli_parse(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply((|| {
        let args: Vec<String> = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let o = kisi_core::commands::parse_args(&args)?;
        Ok(serde_json::json!({"opts": o, "needs_board": kisi_core::commands::needs_board(&o.cmd), "uses_field": kisi_core::commands::uses_field(&o.cmd), "help": kisi_core::commands::HELP}).to_string())
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
        let inp = kisi_core::service::Inputs {
            pcb_path: std::path::PathBuf::from(s("pcb_path").unwrap_or_default()),
            pcb: s("pcb").ok_or("pcb missing")?,
            pro: s("pro"),
            config,
            stackup: serde_json::from_value(v["stackup"].clone()).ok(),
            field: serde_json::from_value(v["field"].clone()).unwrap_or_default(),
        };
        let proj = kisi_core::service::Project::from_texts(inp)?;
        let plan = kisi_core::commands::field_plan(&proj, &o);
        CLI.with(|c| *c.borrow_mut() = Some(proj));
        Ok(serde_json::json!({"plan": plan}).to_string())
    })())
}

/// Run the command on the project. Input: {"opts", "tree": json|null, "report_path": str|null}.
/// Output: the result; for `report` {"result": summary, "write": {"path", "content"}}.
#[no_mangle]
pub extern "C" fn kisi_cli_run(p: *const u8, n: usize) -> i32 {
    let txt = input(p, n);
    reply((|| {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let o: kisi_core::commands::Opts = serde_json::from_value(v["opts"].clone()).map_err(|e| e.to_string())?;
        let tree = if v["tree"].is_null() { None } else { Some(v["tree"].clone()) };
        CLI.with(|c| {
            let c = c.borrow();
            let r = kisi_core::commands::run(&o, c.as_ref(), tree)?;
            if o.cmd == "report" {
                let path = v["report_path"].as_str().unwrap_or("analysis/kisi.json");
                return Ok(serde_json::json!({"result": kisi_core::commands::report_summary(path, &r), "write": {"path": path, "content": serde_json::to_string_pretty(&r).unwrap()}}).to_string());
            }
            Ok(r.to_string())
        })
    })())
}
