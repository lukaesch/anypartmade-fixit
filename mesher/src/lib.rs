//! The part builder for the browser: the same Rust geometry as the Worker, compiled to
//! WebAssembly, so the phone builds the parts and the Worker stays light.
//! Plain exports, no bindings: the page writes JSON into memory from `alloc`, calls
//! `build`, and reads the outputs with `out_ptr` and `out_len`:
//! 0 the result JSON, then for each part its 3MF file and its PNG picture.

use anypartmade_fixit::{assembly, cad, render, simplify, threemf};
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

/// One part: fine grid, then merge the tiny triangles on flat and round faces, so
/// checkers get a small file with the surface within max_error_mm. A strict checker
/// merges points within about 0.01 mm; if that would leave a zero area triangle or a
/// hole, build again on a slightly shifted grid.
fn build_part(
    part: &cad::Part,
    name: &str,
    step: f64,
    target: usize,
    max_error: f64,
) -> Result<(Value, Vec<u8>, Vec<u8>), String> {
    let mut best = None;
    for k in 0..5 {
        let fine = part.mesh_for(step * (1.0 + 0.037 * k as f64), 150_000)?;
        let mesh = simplify::simplify(&fine, target, max_error);
        let r = simplify::report(&mesh);
        if mesh.is_watertight() && r.zero_area == 0 && r.open_edges == 0 {
            best = Some((fine.triangles.len(), mesh));
            break;
        }
    }
    // Only a clean mesh goes to the checker: a broken one would waste a round.
    let Some((fine_triangles, mesh)) = best else {
        return Err(format!("Part {name} could not be meshed cleanly: a wall or gap is thinner than about 0.5 mm. Thicken it."));
    };
    let file = threemf::three_mf(&mesh, name);
    let png = render::png(&render::shade(&mesh, 480), 480, 480);
    let meta = json!({
        "name": name,
        "triangles": mesh.triangles.len(),
        "fine_triangles": fine_triangles,
        "volume_mm3": mesh.volume().round(),
        "fitted": part.fitted,
    });
    Ok((meta, file, png))
}

fn run(input: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let v: Value = serde_json::from_slice(input).map_err(|e| e.to_string())?;
    let dims: BTreeMap<String, f64> = v["dims"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, x)| Some((k.clone(), x.as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    // The design: an assembly ({params, parts}) or one part ({nodes}).
    let design = if v["design"].is_object() {
        v["design"].clone()
    } else {
        v["part"].clone()
    };
    let a = assembly::parse(&design, &dims)?;
    let step = v["step"].as_f64().unwrap_or(0.4);
    let target = v["target_triangles"].as_u64().unwrap_or(8_000) as usize;
    let max_error = v["max_error_mm"].as_f64().unwrap_or(0.02);
    let mut out = vec![Vec::new()];
    let mut parts = Vec::new();
    for p in &a.parts {
        let (meta, file, png) = build_part(&p.part, &p.name, step, target, max_error)?;
        parts.push(meta);
        out.push(file);
        out.push(png);
    }
    out[0] = json!({"ok": true, "parts": parts, "view": a.view()})
        .to_string()
        .into_bytes();
    Ok(out)
}

/// Builds the design from the JSON at `ptr` (which this call frees). Returns the number
/// of outputs: 1 + 2 per part on success, 1 with an error JSON.
///
/// # Safety
/// `ptr` and `len` must come from `alloc` with the input written into it.
#[no_mangle]
pub unsafe extern "C" fn build(ptr: *mut u8, len: usize) -> usize {
    let input = Vec::from_raw_parts(ptr, len, len);
    let out = match run(&input) {
        Ok(o) => o,
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
