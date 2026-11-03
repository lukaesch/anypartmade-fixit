//! Dev tool: write the default knob as a 3MF.
//! cargo run --example knob3mf -- out.3mf [spec.json]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let k = match a.get(2) {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
        None => anypartmade_fixit::knob::KnobSpec::around_shaft(6.0, 4.6),
    };
    let k: anypartmade_fixit::knob::KnobSpec = k;
    k.validate().unwrap();
    let m = k.mesh();
    let f = anypartmade_fixit::threemf::three_mf(&m, "Oven knob");
    println!(
        "{} vertices, {} triangles, {} bytes, volume {:.0} mm3",
        m.vertices.len(),
        m.triangles.len(),
        f.len(),
        m.volume()
    );
    std::fs::write(&a[1], f).unwrap();
}
