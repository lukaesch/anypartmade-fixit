//! Dev tool: read the ruler that SAM found, offline, from a saved SAM reply.
//! cargo run --release --example sam_ruler -- photo.jpg sam_ruler.txt [focus_x focus_y]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).expect("read photo");
    let img = anypartmade_fixit::ruler::decode_jpeg_gray(&bytes).expect("decode");
    let text = std::fs::read_to_string(&a[2]).expect("read SAM text");
    let objs = anypartmade_fixit::sam::parse(&text).expect("parse");
    let n = |i: usize| a[i].parse::<f64>().expect("number");
    let focus = (a.len() > 4).then(|| (n(3), n(4)));
    for o in &objs {
        let m = anypartmade_fixit::measure::moments(o);
        println!("object {} conf {:?} moments {m:?}", o.id, o.confidence);
        let t = std::time::Instant::now();
        let fit = anypartmade_fixit::ruler::read_ruler(&img, o, focus);
        println!("{fit:?} in {:?}", t.elapsed());
    }
}
