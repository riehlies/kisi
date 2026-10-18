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
    /// last analysis and its parameters (for the RF functions)
    an: Option<kisi_core::Analysis>,
    params: Option<Params>,
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
    let mut bufs: Option<(Vec<f32>, Vec<f32>)> = None;
    let rc = with_board(|b| {
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let setup: kisi_core::pi::RailSetup = serde_json::from_value(v["setup"].clone()).map_err(|e| e.to_string())?;
        let params: kisi_core::pi::PiParams = serde_json::from_value(v["params"].clone()).unwrap_or_default();
        let mut r = kisi_core::pi::solve_rail(b, &setup, &params)?;
        bufs = Some((std::mem::take(&mut r.v), std::mem::take(&mut r.j)));
        Ok(serde_json::to_string(&r).unwrap())
    });
    if let Some((v, j)) = bufs {
        ST.with(|st| {
            let mut st = st.borrow_mut();
            st.pi_v = v;
            st.pi_j = j;
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
    with_board(|b| {
        let spec: kisi_core::tree::TreeSpec = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let r = kisi_core::tree::solve_tree(&spec, Some(b))?;
        Ok(serde_json::to_string(&r).unwrap())
    })
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
        let mut rows = Vec::new();
        for item in v["items"].as_array().ok_or("items missing")? {
            let net = item["net"].as_u64().unwrap_or(0) as u32;
            let gbps = item["gbps"].as_f64().unwrap_or(1.0);
            let target = item["target"].as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_bool()?)));
            let end = |k: &str| item[k].as_object().and_then(|_| serde_json::from_value::<kisi_core::eye::EndModel>(item[k].clone()).ok());
            let rise = item["rise"].as_f64();
            rows.push(kisi_core::eye::check_net(b, an, &params.field, net, target, &rp, gbps, (end("tx"), end("rx"), rise)));
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

/// Channel and crosstalk of one net. Input:
/// {"net": id, "rf": RfParams, "target": [z, diff] | null, "bitrate": f | null, "prbs": 7}
/// Output: {"channel", "error", "crosstalk", "touchstone", "eye": Eye | null, "skew": {...} | null}
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
        let ch = kisi_core::rf::channel(b, an, net, &params.field, target, &rp);
        let xt = kisi_core::rf::crosstalk(b, an, net, &params.field, &rp);
        let mut eye = serde_json::Value::Null;
        let mut skew = serde_json::Value::Null;
        let (chv, err, ts) = match ch {
            Ok(c) => {
                let ts = kisi_core::rf::touchstone(&c);
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
                if let (true, Some(pn)) = (c.diff, c.partner) {
                    let rq = kisi_core::rf::RfParams { npts: 10, ..rp.clone() };
                    if let Ok(pc) = kisi_core::rf::channel(b, an, pn, &params.field, target, &rq) {
                        skew = serde_json::json!({"partner": pc.name, "delay_ps": pc.delay_ps, "length_mm": pc.length_mm,
                            "skew_ps": c.delay_ps - pc.delay_ps, "skew_mm": c.length_mm - pc.length_mm,
                            "uncertain": c.warnings.iter().chain(&pc.warnings).any(|w| w.starts_with("net branches") || w.starts_with("net splits"))});
                    }
                }
                (serde_json::to_value(&c).unwrap(), serde_json::Value::Null, ts)
            }
            Err(e) => (serde_json::Value::Null, serde_json::Value::String(e), String::new()),
        };
        Ok(serde_json::json!({"channel": chv, "error": err, "crosstalk": xt, "touchstone": ts, "eye": eye, "skew": skew}).to_string())
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
