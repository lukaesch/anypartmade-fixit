//! Measurements from SAM masks. Pure geometry: sub pixel edges from the mask coverage,
//! ellipse moments for the coin, a circle and flat fit for the D shaft, a radius profile
//! for the knob. No model guesses any number here.

use crate::sam::Object;
use serde::Serialize;
use std::f64::consts::PI;

pub type Pt = (f64, f64);

/// Edge points where coverage crosses one half, interpolated between pixel centres,
/// in image pixels. Scans rows and columns; outside the raster counts as empty.
pub fn edge_points(o: &Object) -> Vec<Pt> {
    let (w, h) = (o.mask.width as isize, o.mask.height as isize);
    let at = |c: isize, r: isize| -> f64 {
        if c < 0 || r < 0 || c >= w || r >= h {
            0.0
        } else {
            o.mask.coverage[(r * w + c) as usize] as f64
        }
    };
    let mut pts = Vec::new();
    let cross = |a: f64, b: f64| -> Option<f64> {
        ((a - 127.5) * (b - 127.5) < 0.0).then(|| (127.5 - a) / (b - a))
    };
    for r in 0..h {
        for c in -1..w {
            if let Some(t) = cross(at(c, r), at(c + 1, r)) {
                pts.push(o.to_image(c as f64 + 0.5 + t, r as f64 + 0.5));
            }
        }
    }
    for c in 0..w {
        for r in -1..h {
            if let Some(t) = cross(at(c, r), at(c, r + 1)) {
                pts.push(o.to_image(c as f64 + 0.5, r as f64 + 0.5 + t));
            }
        }
    }
    pts
}

/// Coverage weighted second moments in image pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moments {
    pub area: f64,
    pub cx: f64,
    pub cy: f64,
    /// Ellipse axes with the same second moments (full lengths, image pixels).
    pub major: f64,
    pub minor: f64,
    /// Direction of the major axis, radians, image axes.
    pub angle: f64,
}

pub fn moments(o: &Object) -> Option<Moments> {
    let (sx, sy) = o.scale();
    let (mut m0, mut mx, mut my) = (0.0, 0.0, 0.0);
    let w = o.mask.width;
    for (i, &v) in o.mask.coverage.iter().enumerate() {
        if v == 0 {
            continue;
        }
        let k = v as f64 / 255.0;
        let (x, y) = o.to_image((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        m0 += k;
        mx += k * x;
        my += k * y;
    }
    if m0 < 4.0 {
        return None;
    }
    let (cx, cy) = (mx / m0, my / m0);
    let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
    for (i, &v) in o.mask.coverage.iter().enumerate() {
        if v == 0 {
            continue;
        }
        let k = v as f64 / 255.0;
        let (x, y) = o.to_image((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        xx += k * (x - cx) * (x - cx);
        yy += k * (y - cy) * (y - cy);
        xy += k * (x - cx) * (y - cy);
    }
    // Each pixel is a small square, not a point: add its own variance.
    let (xx, yy, xy) = (xx / m0 + sx * sx / 12.0, yy / m0 + sy * sy / 12.0, xy / m0);
    let tr = xx + yy;
    let det = xx * yy - xy * xy;
    let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
    let (l1, l2) = (tr / 2.0 + disc, (tr / 2.0 - disc).max(0.0));
    Some(Moments {
        area: m0 * sx * sy,
        cx,
        cy,
        major: 4.0 * l1.sqrt(),
        minor: 4.0 * l2.sqrt(),
        angle: 0.5 * (2.0 * xy).atan2(xx - yy),
    })
}

/// Least squares circle (Kasa fit). Returns centre and radius.
pub fn fit_circle(pts: &[Pt]) -> Option<(f64, f64, f64)> {
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let (mx, my) = (
        pts.iter().map(|p| p.0).sum::<f64>() / n,
        pts.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let (mut suu, mut svv, mut suv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for &(x, y) in pts {
        let (u, v) = (x - mx, y - my);
        suu += u * u;
        svv += v * v;
        suv += u * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-9 {
        return None;
    }
    let a = 0.5 * (suuu + suvv);
    let b = 0.5 * (svvv + svuu);
    let uc = (a * svv - b * suv) / det;
    let vc = (b * suu - a * suv) / det;
    let r = (uc * uc + vc * vc + (suu + svv) / n).sqrt();
    Some((uc + mx, vc + my, r))
}

fn rms(pts: &[Pt], c: (f64, f64, f64)) -> f64 {
    let s: f64 = pts
        .iter()
        .map(|&(x, y)| ((x - c.0).hypot(y - c.1) - c.2).powi(2))
        .sum();
    (s / pts.len().max(1) as f64).sqrt()
}

/// Line through points by principal axis: a point on it and its unit direction.
fn fit_line(pts: &[Pt]) -> Option<(Pt, Pt)> {
    if pts.len() < 2 {
        return None;
    }
    let n = pts.len() as f64;
    let (mx, my) = (
        pts.iter().map(|p| p.0).sum::<f64>() / n,
        pts.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
    for &(x, y) in pts {
        xx += (x - mx) * (x - mx);
        yy += (y - my) * (y - my);
        xy += (x - mx) * (y - my);
    }
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    Some(((mx, my), (angle.cos(), angle.sin())))
}

/// A D shaft seen end on: round diameter and the flat to round distance, in image px.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DFit {
    pub cx: f64,
    pub cy: f64,
    pub diameter_px: f64,
    /// Equal to the diameter when no flat was found.
    pub flat_to_round_px: f64,
    pub has_flat: bool,
    /// Angle of the flat's outward normal, radians, image axes.
    pub flat_angle: f64,
    pub rms_px: f64,
    pub flat_points: usize,
    pub round_points: usize,
}

/// Circle fit that drops points lying clearly inside the circle (the flat), refits on
/// the round part, then fits a line to the dropped points.
pub fn fit_d(pts: &[Pt]) -> Option<DFit> {
    let mut c = fit_circle(pts)?;
    let mut round: Vec<Pt> = pts.to_vec();
    let mut flat: Vec<Pt> = Vec::new();
    for _ in 0..8 {
        let tol = (0.04 * c.2).max(0.6);
        let (r, f): (Vec<Pt>, Vec<Pt>) = pts
            .iter()
            .partition(|&&(x, y)| (x - c.0).hypot(y - c.1) - c.2 > -tol);
        if r.len() < 8 {
            break;
        }
        let next = fit_circle(&r)?;
        let done = (next.2 - c.2).abs() < 1e-4 && r.len() == round.len();
        c = next;
        round = r;
        flat = f;
        if done {
            break;
        }
    }
    let mut out = DFit {
        cx: c.0,
        cy: c.1,
        diameter_px: 2.0 * c.2,
        flat_to_round_px: 2.0 * c.2,
        has_flat: false,
        flat_angle: 0.0,
        rms_px: rms(&round, c),
        flat_points: flat.len(),
        round_points: round.len(),
    };
    // A real flat is a straight run that covers a fair share of the outline.
    if flat.len() >= 6 && flat.len() * 12 >= pts.len() {
        let (p0, d) = fit_line(&flat)?;
        // Unit normal pointing from the line towards the circle centre.
        let mut nrm = (-d.1, d.0);
        let to_c = (c.0 - p0.0, c.1 - p0.1);
        let mut dist = to_c.0 * nrm.0 + to_c.1 * nrm.1;
        if dist < 0.0 {
            nrm = (-nrm.0, -nrm.1);
            dist = -dist;
        }
        out.flat_to_round_px = c.2 + dist;
        out.has_flat = true;
        out.flat_angle = (-nrm.1).atan2(-nrm.0);
    }
    Some(out)
}

/// Knob seen from the front: outer diameter and grip ribs from the radius profile.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct KnobFit {
    pub cx: f64,
    pub cy: f64,
    pub outer_diameter_px: f64,
    pub rib_count: u32,
    /// Peak to valley rib depth in image px; zero when no ribs were found.
    pub rib_depth_px: f64,
    /// How clearly the rib frequency stands out, 0 to 1.
    pub rib_clarity: f64,
}

pub fn fit_knob(o: &Object) -> Option<KnobFit> {
    let m = moments(o)?;
    let pts = edge_points(o);
    const BINS: usize = 720;
    let mut prof = vec![f64::NAN; BINS];
    for &(x, y) in &pts {
        let (dx, dy) = (x - m.cx, y - m.cy);
        let a = dy.atan2(dx).rem_euclid(2.0 * PI);
        let b = ((a / (2.0 * PI)) * BINS as f64) as usize % BINS;
        let r = dx.hypot(dy);
        if prof[b].is_nan() || r > prof[b] {
            prof[b] = r;
        }
    }
    let known: Vec<usize> = (0..BINS).filter(|&i| !prof[i].is_nan()).collect();
    if known.len() < BINS / 4 {
        return None;
    }
    // Fill empty bins from the nearest known bin.
    let filled: Vec<f64> = (0..BINS)
        .map(|i| {
            if !prof[i].is_nan() {
                return prof[i];
            }
            let j = known
                .iter()
                .min_by_key(|&&k| {
                    let d = k.abs_diff(i);
                    d.min(BINS - d)
                })
                .copied()
                .unwrap_or(i);
            prof[j]
        })
        .collect();
    let mut sorted = filled.clone();
    sorted.sort_by(f64::total_cmp);
    let r_outer = sorted[(BINS as f64 * 0.95) as usize];
    let mean = filled.iter().sum::<f64>() / BINS as f64;
    let amp = |k: usize| -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, r) in filled.iter().enumerate() {
            let t = 2.0 * PI * (k * i) as f64 / BINS as f64;
            re += (r - mean) * t.cos();
            im += (r - mean) * t.sin();
        }
        2.0 * re.hypot(im) / BINS as f64
    };
    let amps: Vec<(usize, f64)> = (6..=72).map(|k| (k, amp(k))).collect();
    let &(k, a) = amps.iter().max_by(|x, y| x.1.total_cmp(&y.1))?;
    let others = amps
        .iter()
        .filter(|(j, _)| *j != k)
        .map(|x| x.1)
        .sum::<f64>()
        / (amps.len() - 1) as f64;
    let clarity = (1.0 - others / a.max(1e-9)).clamp(0.0, 1.0);
    // Ribs must be at least a tenth of a pixel and clearly periodic.
    let ribs = a > 0.1 && a > 0.004 * mean && clarity > 0.6;
    Some(KnobFit {
        cx: m.cx,
        cy: m.cy,
        outer_diameter_px: 2.0 * r_outer,
        rib_count: if ribs { k as u32 } else { 0 },
        rib_depth_px: if ribs { 2.0 * a } else { 0.0 },
        rib_clarity: clarity,
    })
}

/// The outline as a closed polygon around a centre, at most `max` points, for drawing.
pub fn outline(pts: &[Pt], cx: f64, cy: f64, max: usize) -> Vec<[f64; 2]> {
    let mut v: Vec<(f64, Pt)> = pts
        .iter()
        .map(|&(x, y)| ((y - cy).atan2(x - cx), (x, y)))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let step = (v.len() as f64 / max.max(1) as f64).max(1.0);
    let mut out = Vec::new();
    let mut i = 0.0;
    while (i as usize) < v.len() {
        let p = v[i as usize].1;
        out.push([(p.0 * 10.0).round() / 10.0, (p.1 * 10.0).round() / 10.0]);
        i += step;
    }
    out
}

/// Any object's size from its mask: the sides of the rectangle with the same second
/// moments, and a circle fit of its outline (for round things).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectSize {
    pub cx: f64,
    pub cy: f64,
    pub length_px: f64,
    pub width_px: f64,
    pub angle: f64,
    pub circle_px: Option<f64>,
    /// Mask area over length times width: near 1 for a rectangle, 0.785 for a disc.
    pub fill: f64,
}

/// Convex hull (monotone chain), counter clockwise.
fn hull(mut p: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    p.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut h: Vec<(f64, f64)> = Vec::new();
    for pass in 0..2 {
        let start = h.len();
        let pts: Box<dyn Iterator<Item = &(f64, f64)>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for &q in pts {
            while h.len() >= start + 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

pub fn object_size(o: &Object) -> Option<ObjectSize> {
    let m = moments(o)?;
    // The smallest rotated rectangle around the outline (rotating calipers over the
    // convex hull of the covered pixels' corners): exact for a board even with cut
    // corners, a label or a connector, where moments alone tilt or undercount.
    let w = o.mask.width;
    let h = o.mask.height;
    let on = |c: usize, r: usize| o.mask.coverage[r * w + c] >= 128;
    let mut pts = Vec::new();
    for r in 0..h {
        for c in 0..w {
            if !on(c, r) {
                continue;
            }
            let edge = c == 0
                || r == 0
                || c + 1 == w
                || r + 1 == h
                || !on(c - 1, r)
                || !on(c + 1, r)
                || !on(c, r - 1)
                || !on(c, r + 1);
            if edge {
                for (dc, dr) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    pts.push(o.to_image(c as f64 + dc, r as f64 + dr));
                }
            }
        }
    }
    let hl = hull(pts);
    if hl.len() < 3 {
        return None;
    }
    let mut best: Option<(f64, f64, f64, f64, f64, f64)> = None; // area, angle, len, wid, cx, cy
    for i in 0..hl.len() {
        let (p, q) = (hl[i], hl[(i + 1) % hl.len()]);
        let ang = (q.1 - p.1).atan2(q.0 - p.0);
        let (ux, uy) = (ang.cos(), ang.sin());
        let (mut a0, mut a1, mut b0, mut b1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &(x, y) in &hl {
            let (a, b) = (x * ux + y * uy, -x * uy + y * ux);
            a0 = a0.min(a);
            a1 = a1.max(a);
            b0 = b0.min(b);
            b1 = b1.max(b);
        }
        let area = (a1 - a0) * (b1 - b0);
        if best.is_none_or(|b| area < b.0) {
            let (ca, cb) = ((a0 + a1) / 2.0, (b0 + b1) / 2.0);
            best = Some((
                area,
                ang,
                a1 - a0,
                b1 - b0,
                ca * ux - cb * uy,
                ca * uy + cb * ux,
            ));
        }
    }
    let (area, mut ang, mut l, mut wd, cx, cy) = best?;
    if wd > l {
        std::mem::swap(&mut l, &mut wd);
        ang += std::f64::consts::FRAC_PI_2;
    }
    let circle = fit_circle(&edge_points(o)).map(|c| 2.0 * c.2);
    Some(ObjectSize {
        cx,
        cy,
        length_px: l,
        width_px: wd,
        angle: ang,
        circle_px: circle,
        fill: m.area / area.max(1e-9),
    })
}

/// A shaft seen from the side: an elongated outline. Its diameter is the width across
/// the long axis, the median over the middle of its length (the ends are rounded or hidden).
#[derive(Clone, Debug, PartialEq)]
pub struct SideFit {
    pub cx: f64,
    pub cy: f64,
    pub width_px: f64,
    pub length_px: f64,
    /// Spread of the width over the middle, px (low for a straight pin).
    pub spread_px: f64,
}

/// Outlines longer than this times their width are read as side views.
pub const SIDE_VIEW_ELONGATION: f64 = 1.3;

pub fn side_width(o: &Object) -> Option<SideFit> {
    let m = moments(o)?;
    let (sx, sy) = o.scale();
    let (ux, uy) = (m.angle.cos(), m.angle.sin());
    let w = o.mask.width;
    // Covered area per 1 px slice along the axis is the width at that slice.
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for (i, &v) in o.mask.coverage.iter().enumerate() {
        if v == 0 {
            continue;
        }
        let (x, y) = o.to_image((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        pts.push((
            (x - m.cx) * ux + (y - m.cy) * uy,
            v as f64 / 255.0 * sx * sy,
        ));
    }
    let lo = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let hi = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
    let length = hi - lo;
    if length < 8.0 {
        return None;
    }
    let bins = length.ceil() as usize + 1;
    let mut area = vec![0.0; bins];
    for (t, a) in &pts {
        area[((t - lo).floor() as usize).min(bins - 1)] += a;
    }
    let (a, b) = ((bins as f64 * 0.25) as usize, (bins as f64 * 0.75) as usize);
    let mut mid: Vec<f64> = area[a..b.max(a + 1)].to_vec();
    mid.sort_by(f64::total_cmp);
    let width = mid[mid.len() / 2];
    let spread = mid[mid.len() * 3 / 4] - mid[mid.len() / 4];
    Some(SideFit {
        cx: m.cx,
        cy: m.cy,
        width_px: width,
        length_px: length,
        spread_px: spread,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::sam::{Mask, Object};

    /// A synthetic mask with anti aliased coverage from a point in shape test,
    /// 8 x 8 supersampled, placed with its box at (x0, y0) in a 2048 x 1536 image.
    #[test]
    fn rectangle_sides_from_the_mask() {
        // A board 84 x 71 px, tilted 10 degrees.
        let a = 10f64.to_radians();
        let o = synth(140, 140, 300, 300, |x, y| {
            let (dx, dy) = (x - 70.0, y - 70.0);
            (dx * a.cos() + dy * a.sin()).abs() <= 42.0
                && (-dx * a.sin() + dy * a.cos()).abs() <= 35.5
        });
        let s = object_size(&o).unwrap();
        // Within about a pixel, which is 0.1 mm at a typical scale.
        assert!(
            (s.length_px - 84.0).abs() < 1.5 && (s.width_px - 71.0).abs() < 1.5,
            "{s:?}"
        );
        assert!(s.fill > 0.95);
        // A label or a cut corner must not shrink it: the same board with one corner cut.
        let cut = synth(140, 140, 300, 300, |x, y| {
            let (dx, dy) = (x - 70.0, y - 70.0);
            let (p, q) = (dx * a.cos() + dy * a.sin(), -dx * a.sin() + dy * a.cos());
            p.abs() <= 42.0 && q.abs() <= 35.5 && !(p > 30.0 && q > 23.0)
        });
        let c = object_size(&cut).unwrap();
        assert!(
            (c.length_px - 84.0).abs() < 1.5 && (c.width_px - 71.0).abs() < 1.5,
            "{c:?}"
        );
    }

    #[test]
    fn side_view_width_of_a_tilted_pin() {
        // A pin 70 px wide and 110 px long, tilted 20 degrees, round at one end.
        let (a, c) = (20f64.to_radians(), (60.0, 70.0));
        let o = synth(140, 140, 500, 1100, |x, y| {
            let (dx, dy) = (x - c.0, y - c.1);
            let t = dx * a.sin() + dy * a.cos();
            let n = dx * a.cos() - dy * a.sin();
            (t.abs() <= 55.0 && n.abs() <= 35.0) || (t - 55.0).hypot(n) <= 35.0
        });
        let m = moments(&o).unwrap();
        assert!(m.major / m.minor > SIDE_VIEW_ELONGATION);
        let f = side_width(&o).unwrap();
        assert!((f.width_px - 70.0).abs() < 1.0, "{f:?}");
    }

    pub fn synth(
        w: usize,
        h: usize,
        x0: u32,
        y0: u32,
        inside: impl Fn(f64, f64) -> bool,
    ) -> Object {
        let mut cov = vec![0u8; w * h];
        for r in 0..h {
            for c in 0..w {
                let mut n = 0;
                for i in 0..8 {
                    for j in 0..8 {
                        let x = c as f64 + (i as f64 + 0.5) / 8.0;
                        let y = r as f64 + (j as f64 + 0.5) / 8.0;
                        n += inside(x, y) as u32;
                    }
                }
                cov[r * w + c] = ((n * 255 + 32) / 64) as u8;
            }
        }
        Object {
            id: "0".into(),
            x1: x0,
            y1: y0,
            x2: x0 + w as u32 - 1,
            y2: y0 + h as u32 - 1,
            image_w: 2048,
            image_h: 1536,
            confidence: Some(0.9),
            mask: Mask {
                width: w,
                height: h,
                coverage: cov,
            },
        }
    }

    #[test]
    fn coin_diameter_from_moments_and_circle() {
        let o = synth(160, 160, 300, 200, |x, y| (x - 80.3).hypot(y - 79.6) < 76.4);
        let m = moments(&o).expect("moments");
        assert!((m.major - 152.8).abs() < 0.3, "{m:?}");
        assert!((m.cx - 380.3).abs() < 0.05 && (m.cy - 279.6).abs() < 0.05);
        let c = fit_circle(&edge_points(&o)).expect("circle");
        assert!((2.0 * c.2 - 152.8).abs() < 0.3, "{c:?}");
    }

    #[test]
    fn tilted_coin_keeps_its_major_axis() {
        let o = synth(170, 140, 0, 0, |x, y| {
            let (u, v) = (x - 85.0, y - 70.0);
            (u / 80.0).powi(2) + (v / 64.0).powi(2) < 1.0
        });
        let m = moments(&o).expect("moments");
        assert!(
            (m.major - 160.0).abs() < 0.5 && (m.minor - 128.0).abs() < 0.5,
            "{m:?}"
        );
    }

    #[test]
    fn d_shaft_diameter_and_flat() {
        // 6 mm shaft with a 4.6 mm flat to round at 10 px per mm, flat facing up and right.
        let (r, f) = (30.0, 46.0);
        let (nx, ny) = ((0.5f64).cos(), -(0.5f64).sin());
        let o = synth(80, 80, 1000, 700, |x, y| {
            let (u, v) = (x - 40.0, y - 40.0);
            u.hypot(v) < r && u * nx + v * ny < f - r
        });
        let d = fit_d(&edge_points(&o)).expect("fit");
        assert!(d.has_flat, "{d:?}");
        assert!((d.diameter_px - 60.0).abs() < 0.3, "{d:?}");
        assert!((d.flat_to_round_px - 46.0).abs() < 0.3, "{d:?}");
        assert!(
            (d.flat_angle - (-0.5f64)).abs() < 0.05 || (d.flat_angle + 0.5).abs() < 0.05,
            "{d:?}"
        );
    }

    #[test]
    fn round_shaft_has_no_flat() {
        let o = synth(70, 70, 10, 10, |x, y| (x - 35.0).hypot(y - 35.0) < 31.0);
        let d = fit_d(&edge_points(&o)).expect("fit");
        assert!(!d.has_flat, "{d:?}");
        assert!((d.diameter_px - 62.0).abs() < 0.3);
    }

    #[test]
    fn knob_ribs_are_counted() {
        let o = synth(300, 300, 500, 400, |x, y| {
            let (u, v) = (x - 150.0, y - 150.0);
            let a = v.atan2(u);
            u.hypot(v) < 140.0 - 3.0 * (0.5 - 0.5 * (18.0 * a).cos())
        });
        let k = fit_knob(&o).expect("knob");
        assert_eq!(k.rib_count, 18, "{k:?}");
        assert!((k.outer_diameter_px - 280.0).abs() < 1.5, "{k:?}");
        assert!((k.rib_depth_px - 3.0).abs() < 1.0, "{k:?}");
        let plain = synth(200, 200, 0, 0, |x, y| (x - 100.0).hypot(y - 100.0) < 90.0);
        assert_eq!(fit_knob(&plain).expect("plain").rib_count, 0);
    }

    #[test]
    fn outline_is_capped() {
        let o = synth(100, 100, 0, 0, |x, y| (x - 50.0).hypot(y - 50.0) < 40.0);
        let pts = edge_points(&o);
        let ol = outline(&pts, 50.0, 50.0, 64);
        assert!(ol.len() <= 65 && ol.len() >= 60);
    }
}
