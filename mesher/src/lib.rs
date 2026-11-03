//! The part builder for the browser: the same Rust geometry as the Worker, compiled to
//! WebAssembly, so the phone builds the part and the Worker stays light.
//! Plain exports, no bindings: the page writes JSON into memory from `alloc`, calls
//! `build`, and reads the outputs with `out_ptr` and `out_len`:
//! 0 the result JSON, 1 the 3MF file, 2 the PNG picture.

use anypartmade_fixit::{cad, render, simplify, threemf};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    static OUT: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

/// Memory for the page to write the input into.
#[no_mangle]
pub extern "C" fn alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

fn run(input: &[u8]) -> Result<(Value, Vec<u8>, Vec<u8>), String> {
    let v: Value = serde_json::from_slice(input).map_err(|e| e.to_string())?;
    let dims: BTreeMap<String, f64> = v["dims"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, x)| Some((k.clone(), x.as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    let part = cad::parse(&v["part"], &dims)?;
    // Fine grid, then merge the tiny triangles on flat and round faces: small files that
    // checkers handle, with the surface within max_error_mm of the fine mesh.
    let fine = part.mesh_for(v["step"].as_f64().unwrap_or(0.4), 400_000)?;
    let target = v["target_triangles"].as_u64().unwrap_or(8_000) as usize;
    let mesh = simplify::simplify(&fine, target, v["max_error_mm"].as_f64().unwrap_or(0.02));
    if !mesh.is_watertight() {
        return Err("The simplified mesh is not closed.".into());
    }
    let file = threemf::three_mf(&mesh, v["name"].as_str().unwrap_or("part"));
    let png = render::png(&render::shade(&mesh, 480), 480, 480);
    let meta = json!({
        "ok": true,
        "triangles": mesh.triangles.len(),
        "fine_triangles": fine.triangles.len(),
        "volume_mm3": mesh.volume().round(),
        "fitted": part.fitted,
    });
    Ok((meta, file, png))
}

/// Builds the part from the JSON at `ptr` (which this call frees). Returns the number of
/// outputs: 3 on success, 1 with an error JSON.
///
/// # Safety
/// `ptr` and `len` must come from `alloc` with the input written into it.
#[no_mangle]
pub unsafe extern "C" fn build(ptr: *mut u8, len: usize) -> usize {
    let input = Vec::from_raw_parts(ptr, len, len);
    let out = match run(&input) {
        Ok((meta, file, png)) => vec![meta.to_string().into_bytes(), file, png],
        Err(e) => vec![json!({"ok": false, "error": e}).to_string().into_bytes()],
    };
    let n = out.len();
    OUT.with(|o| *o.borrow_mut() = out);
    n
}

#[no_mangle]
pub extern "C" fn out_ptr(i: usize) -> *const u8 {
    OUT.with(|o| o.borrow().get(i).map_or(std::ptr::null(), |v| v.as_ptr()))
}

#[no_mangle]
pub extern "C" fn out_len(i: usize) -> usize {
    OUT.with(|o| o.borrow().get(i).map_or(0, |v| v.len()))
}
