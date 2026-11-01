//! Dev tool: read ruler ticks in a JPEG inside a hand drawn box.
//! cargo run --release --example ticks -- photo.jpg cx cy angle_deg half_len half_width [focus_x focus_y]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).expect("read photo");
    let img = anypartmade_fixit::ruler::decode_jpeg_gray(&bytes).expect("decode");
    let n = |i: usize| a[i].parse::<f64>().expect("number");
    let t = std::time::Instant::now();
    let fit = anypartmade_fixit::ruler::read_ticks(
        &img,
        n(2),
        n(3),
        n(4).to_radians(),
        n(5),
        n(6),
        (a.len() > 8).then(|| (n(7), n(8))),
    );
    println!("{fit:?} in {:?}", t.elapsed());
}
