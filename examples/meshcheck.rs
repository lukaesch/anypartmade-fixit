//! Dev tool: build a part like the browser does and report what a strict checker sees:
//! zero area triangles, and open edges after welding vertices at the same position.
//! cargo run --release --example meshcheck -- input.json
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let dims = v["dims"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, x)| (k.clone(), x.as_f64().unwrap()))
        .collect();
    let p = anypartmade_fixit::cad::parse(&v["part"], &dims).unwrap();
    for k in 0..3 {
        let fine = p.mesh_for(0.4 * (1.0 + 0.037 * k as f64), 150_000).unwrap();
        let m = anypartmade_fixit::simplify::simplify(&fine, 8000, 0.02);
        let r = anypartmade_fixit::simplify::report(&m);
        println!(
            "try {k}: fine {} -> {} triangles, closed by index {}, {r:?}",
            fine.triangles.len(),
            m.triangles.len(),
            m.is_watertight()
        );
    }
}
