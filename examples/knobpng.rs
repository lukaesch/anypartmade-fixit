//! Dev tool: render a knob spec (default knob without one) to a PNG.
//! cargo run --release --example knobpng -- out.png [spec.json]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let k: anypartmade_fixit::knob::KnobSpec = match a.get(2) {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
        None => anypartmade_fixit::knob::KnobSpec::around_shaft(6.0, 4.6),
    };
    let t = std::time::Instant::now();
    let rgb = anypartmade_fixit::render::shade(&k.mesh(), 480);
    let png = anypartmade_fixit::render::png(&rgb, 480, 480);
    println!("{} bytes in {:?}", png.len(), t.elapsed());
    std::fs::write(&a[1], png).unwrap();
}
