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
    pro: Option<String>,
    out: Vec<u8>,
    zone_pts: Vec<f32>,
    zone_idx: Vec<u32>,
    pi_v: Vec<f32>,
    pi_j: Vec<f32>,
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
                    b.warnings.push(format!("Projektdatei: {e}"));
                }
            }
            ST.with(|st| st.borrow_mut().board = Some(b));
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
    let s = ST.with(|st| {
        let st = st.borrow();
        st.board.as_ref().map(|b| serde_json::to_string(&analyze(b, &params)).unwrap())
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
