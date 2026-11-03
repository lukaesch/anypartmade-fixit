//! Dev tool: mesh a CAD part as a 3MF and a PNG.
//! cargo run --release --example cad3mf -- part.json out.3mf out.png [step_mm]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let part: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let dims = std::collections::BTreeMap::from([
        ("shaft_diameter_mm".to_string(), 5.96),
        ("flat_to_round_mm".to_string(), 4.37),
        ("board_length_mm".to_string(), 21.0),
        ("board_width_mm".to_string(), 17.8),
    ]);
    let step = a.get(4).map(|s| s.parse().unwrap()).unwrap_or(0.3);
    let t = std::time::Instant::now();
    let p = anypartmade_fixit::cad::parse(&part, &dims).unwrap();
    let m = p.mesh(step).unwrap();
    let t1 = t.elapsed();
    let f = anypartmade_fixit::threemf::three_mf(&m, "part");
    let t2 = t.elapsed();
    println!(
        "{} triangles, closed {}, {:.0} mm3, 3MF {} KB, mesh {:?}, 3mf {:?}",
        m.triangles.len(),
        m.is_watertight(),
        m.volume(),
        f.len() / 1024,
        t1,
        t2
    );
    std::fs::write(&a[2], f).unwrap();
    let png = anypartmade_fixit::render::png(&anypartmade_fixit::render::shade(&m, 480), 480, 480);
    std::fs::write(&a[3], png).unwrap();
    println!("{}", serde_json::to_string(&p.critical_dimensions).unwrap());
}
