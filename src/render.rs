//! A shaded picture of the mesh as a PNG, so the page can show the knob without WebGL
//! or external assets: orthographic view from above the front, z buffer, flat shading.

use crate::knob::Mesh;

const BG: [f64; 3] = [246.0, 247.0, 248.0];
const BODY: [f64; 3] = [70.0, 78.0, 92.0];

fn norm(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// RGB pixels, `size` by `size`. The view looks down at about 35 degrees, the pointer
/// (+x) turned towards the viewer's right.
pub fn shade(m: &Mesh, size: usize) -> Vec<u8> {
    let ss = 2; // supersampling
    let n = size * ss;
    let az = 0.35f64; // radians from +x towards -y
    let el = 0.62f64;
    let view = norm([az.cos() * el.cos(), -az.sin() * el.cos(), el.sin()]); // towards the viewer
    let right = norm(cross([0.0, 0.0, 1.0], view));
    let up = cross(view, right);
    let light = norm([
        view[0] * 0.6 - right[0] * 0.5 + up[0] * 0.6,
        view[1] * 0.6 - right[1] * 0.5 + up[1] * 0.6,
        view[2] * 0.6 - right[2] * 0.5 + up[2] * 0.6,
    ]);
    let proj: Vec<[f64; 3]> = m
        .vertices
        .iter()
        .map(|&p| [dot(p, right), dot(p, up), dot(p, view)])
        .collect();
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &proj {
        x0 = x0.min(p[0]);
        x1 = x1.max(p[0]);
        y0 = y0.min(p[1]);
        y1 = y1.max(p[1]);
    }
    let scale = 0.86 * n as f64 / (x1 - x0).max(y1 - y0).max(1e-9);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let to_px = |p: &[f64; 3]| {
        [
            n as f64 / 2.0 + (p[0] - cx) * scale,
            n as f64 / 2.0 - (p[1] - cy) * scale,
            p[2],
        ]
    };
    let mut depth = vec![f64::MIN; n * n];
    let mut color = vec![BG; n * n];
    for t in &m.triangles {
        let [a, b, c] = t.map(|i| m.vertices[i as usize]);
        let normal = norm(cross(
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        ));
        if dot(normal, view) <= 0.0 {
            continue; // facing away
        }
        let lit = 0.5 + 0.55 * dot(normal, light).max(0.0);
        let spec = dot(
            normal,
            norm([light[0] + view[0], light[1] + view[1], light[2] + view[2]]),
        )
        .max(0.0)
        .powi(24)
            * 60.0;
        let col = BODY.map(|v| (v * lit * 1.6 + spec).min(255.0));
        let [p, q, r] = t.map(|i| to_px(&proj[i as usize]));
        let area = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let xa = p[0].min(q[0]).min(r[0]).floor().max(0.0) as usize;
        let xb = (p[0].max(q[0]).max(r[0]).ceil() as usize).min(n - 1);
        let ya = p[1].min(q[1]).min(r[1]).floor().max(0.0) as usize;
        let yb = (p[1].max(q[1]).max(r[1]).ceil() as usize).min(n - 1);
        for y in ya..=yb {
            for x in xa..=xb {
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((q[0] - fx) * (r[1] - fy) - (q[1] - fy) * (r[0] - fx)) / area;
                let w1 = ((r[0] - fx) * (p[1] - fy) - (r[1] - fy) * (p[0] - fx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                    continue;
                }
                let z = w0 * p[2] + w1 * q[2] + w2 * r[2];
                let k = y * n + x;
                if z > depth[k] {
                    depth[k] = z;
                    color[k] = col;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(size * size * 3);
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0; 3];
            for dy in 0..ss {
                for dx in 0..ss {
                    let c = color[(y * ss + dy) * n + x * ss + dx];
                    for i in 0..3 {
                        acc[i] += c[i];
                    }
                }
            }
            out.extend(acc.map(|v| (v / (ss * ss) as f64).round() as u8));
        }
    }
    out
}

/// RGB pixels as a PNG file.
pub fn png(rgb: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for row in rgb.chunks(w * 3) {
        raw.push(0); // no filter
        raw.extend_from_slice(row);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend(crate::threemf::crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]); // 8 bit RGB
    chunk(b"IHDR", &ihdr);
    chunk(
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_picture_is_a_png_with_the_knob_in_the_middle() {
        let m = crate::knob::KnobSpec::around_shaft(6.0, 4.6).mesh();
        let rgb = shade(&m, 160);
        let mid = (80 * 160 + 80) * 3;
        assert_ne!(&rgb[mid..mid + 3], &[246, 247, 248]);
        assert_eq!(&rgb[..3], &[246, 247, 248]);
        let p = png(&rgb, 160, 160);
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&p[p.len() - 8..p.len() - 4], b"IEND");
    }
}
