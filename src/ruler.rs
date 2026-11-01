//! Scale from a ruler without taps: the millimetre ticks are a periodic pattern along
//! the ruler, so the strongest frequency of the brightness profile is one tick per mm.
//! Averaging over dozens of ticks gives the spacing to a small fraction of a pixel.

use serde::Serialize;
use std::f64::consts::PI;

/// 8 bit greyscale image.
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Gray {
    /// Bilinear sample; `None` outside the image.
    pub fn at(&self, x: f64, y: f64) -> Option<f64> {
        if !(0.0..(self.w - 1) as f64).contains(&x) || !(0.0..(self.h - 1) as f64).contains(&y) {
            return None;
        }
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let p = |xx: usize, yy: usize| self.px[yy * self.w + xx] as f64;
        let top = p(x0, y0) * (1.0 - fx) + p(x0 + 1, y0) * fx;
        let bot = p(x0, y0 + 1) * (1.0 - fx) + p(x0 + 1, y0 + 1) * fx;
        Some(top * (1.0 - fy) + bot * fy)
    }
}

/// Decodes a JPEG to greyscale. Other formats give `None` (the user can still tap marks).
pub fn decode_jpeg_gray(bytes: &[u8]) -> Option<Gray> {
    let mut d = jpeg_decoder::Decoder::new(bytes);
    let data = d.decode().ok()?;
    let info = d.info()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let px = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => data,
        jpeg_decoder::PixelFormat::RGB24 => data
            .chunks_exact(3)
            .map(|c| ((c[0] as u32 * 77 + c[1] as u32 * 150 + c[2] as u32 * 29) >> 8) as u8)
            .collect(),
        _ => return None,
    };
    (px.len() == w * h).then_some(Gray { w, h, px })
}

/// Reads the ruler that SAM found: its mask gives the axis and the area to read.
/// `focus` is the image point where the scale matters (the shaft).
pub fn read_ruler(
    img: &Gray,
    o: &crate::sam::Object,
    focus: Option<(f64, f64)>,
) -> Option<RulerFit> {
    let m = crate::measure::moments(o)?;
    // For a filled rectangle the moment axes are 2/sqrt(3) times its sides.
    let k = 2.0 / 3f64.sqrt();
    read_ticks(
        img,
        m.cx,
        m.cy,
        m.angle,
        m.major / k / 2.0,
        m.minor / k / 2.0,
        focus,
    )
}

/// Smallest and largest tick spacing we look for, in px per mm.
const MIN_PERIOD: f64 = 3.0;
const MAX_PERIOD: f64 = 40.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RulerFit {
    pub px_per_mm: f64,
    /// A 10 mm stretch along the ticks, for drawing.
    pub p1: [f64; 2],
    pub p2: [f64; 2],
    /// Length of the profile that was read, px.
    pub span_px: f64,
    /// Peak power over the median power of the spectrum.
    pub peak_ratio: f64,
    /// The 5 mm or 10 mm marks show up too, as on a metric ruler.
    pub cm_marks: bool,
    /// Windows of about 12 ticks whose local spacing was measured.
    pub windows: usize,
    /// Relative rms of the local spacings around the fitted line.
    pub fit_rms: f64,
    /// How much the spacing changes from one end of the read ruler to the other,
    /// relative (perspective when the ruler is not parallel to the camera).
    pub scale_change: f64,
    pub confidence: f64,
}

/// Power of the profile at frequency `f` (cycles per px).
fn power(x: &[f64], f: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    let w = 2.0 * PI * f;
    let (c, s) = (w.cos(), w.sin());
    let (mut cr, mut ci) = (1.0, 0.0);
    for &v in x {
        re += v * cr;
        im -= v * ci;
        let n = cr * c - ci * s;
        ci = cr * s + ci * c;
        cr = n;
    }
    (re * re + im * im) / x.len() as f64
}

/// Removes slow brightness changes (glare, shadows): subtract a running mean.
fn detrend(x: &[f64], win: usize) -> Vec<f64> {
    let n = x.len();
    let mut pre = vec![0.0; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + x[i];
    }
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(win / 2);
            let b = (i + win / 2 + 1).min(n);
            x[i] - (pre[b] - pre[a]) / (b - a) as f64
        })
        .collect()
}

fn spectrum_peak(x: &[f64]) -> Option<(f64, f64, f64)> {
    let n = x.len() as f64;
    // Hann window: keeps side lobes from posing as peaks.
    let w: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, v)| v * (0.5 - 0.5 * (2.0 * PI * i as f64 / (n - 1.0)).cos()))
        .collect();
    let x = &w[..];
    let (f0, f1) = (1.0 / MAX_PERIOD, 1.0 / MIN_PERIOD);
    let step = 1.0 / n;
    let mut powers = Vec::new();
    let mut f = f0;
    while f <= f1 {
        powers.push((f, power(x, f)));
        f += step;
    }
    let &(fp, pp) = powers.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    let mut sorted: Vec<f64> = powers.iter().map(|p| p.1).collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2].max(1e-12);
    Some((fp, pp, median))
}

/// Golden section search for the exact peak frequency near `f`.
fn refine(x: &[f64], f: f64) -> f64 {
    let half = 1.0 / x.len() as f64;
    let (mut a, mut b) = (f - half, f + half);
    let g = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..40 {
        let c = b - g * (b - a);
        let d = a + g * (b - a);
        if power(x, c) > power(x, d) {
            b = d;
        } else {
            a = c;
        }
    }
    (a + b) / 2.0
}

/// Thin ticks have strong harmonics. If a half, third or quarter of the peak frequency
/// is also strong, that lower one is the tick spacing.
fn fundamental(x: &[f64], f: f64) -> f64 {
    let top = power(x, refine(x, f));
    let mut best = f;
    for k in 2..=4 {
        let g = f / k as f64;
        if 1.0 / g > MAX_PERIOD {
            break;
        }
        let g = refine(x, g);
        if power(x, g) > 0.25 * top {
            best = g;
        }
    }
    best
}

/// Reads the ticks of a ruler whose long axis runs through (cx, cy) at `angle`
/// (radians, image axes), `half_len` px each way, `half_width` px across.
pub fn read_ticks(
    img: &Gray,
    cx: f64,
    cy: f64,
    angle: f64,
    half_len: f64,
    half_width: f64,
    focus: Option<(f64, f64)>,
) -> Option<RulerFit> {
    let u = (angle.cos(), angle.sin());
    let v = (-u.1, u.0);
    let profile = |s: f64| -> Option<(f64, Vec<f64>)> {
        let mut out = Vec::new();
        let mut first = None;
        let mut t = -half_len;
        while t <= half_len {
            let mut sum = 0.0;
            let mut ok = true;
            for d in [-1.0, 0.0, 1.0] {
                match img.at(cx + t * u.0 + (s + d) * v.0, cy + t * u.1 + (s + d) * v.1) {
                    Some(val) => sum += val,
                    None => ok = false,
                }
            }
            if ok {
                first.get_or_insert(t);
                out.push(sum / 3.0);
            } else if first.is_some() {
                break;
            }
            t += 1.0;
        }
        (out.len() as f64 >= 8.0 * MAX_PERIOD).then(|| (first.unwrap_or(0.0), out))
    };
    // Scan bands across the ruler, keep the one with the clearest periodic pattern.
    #[allow(clippy::type_complexity)]
    let mut bands: Vec<(f64, f64, f64, f64, f64, Vec<f64>, Vec<f64>)> = Vec::new();
    let mut s = -half_width;
    while s <= half_width {
        if let Some((t0, p)) = profile(s) {
            let x = detrend(&p, (3.0 * MAX_PERIOD) as usize | 1);
            // Gentler detrend for the 5 and 10 mm marks, whose period is long.
            let slow = detrend(&p, (25.0 * MAX_PERIOD) as usize | 1);
            if let Some((f, pw, med)) = spectrum_peak(&x) {
                let f = fundamental(&x, f);
                bands.push((s, f, pw / med, med, t0, x, slow));
            }
        }
        s += 4.0;
    }
    // A ruler can carry two scales (mm on one edge, inches on the other). Group the
    // bands by their tick frequency and keep the pattern that has metric marks: the
    // 5 mm and 10 mm marks reach further across than the 1 mm ticks, so some band shows
    // power at a fifth or a tenth of the tick frequency. Inch scales show halves and
    // quarters instead.
    let mut order: Vec<usize> = (0..bands.len()).collect();
    order.sort_by(|&a, &b| bands[b].2.total_cmp(&bands[a].2));
    let mut clusters: Vec<usize> = Vec::new();
    for &i in &order {
        if bands[i].2 < 20.0 || clusters.len() >= 4 {
            continue;
        }
        if clusters
            .iter()
            .all(|&c| (bands[c].1 - bands[i].1).abs() > 0.01 * bands[c].1)
        {
            clusters.push(i);
        }
    }
    // How much a frequency stands out from its neighbours in the spectrum.
    let prominence = |x: &[f64], g: f64| -> f64 {
        let around = [0.8, 0.86, 1.14, 1.2]
            .iter()
            .map(|k| power(x, g * k))
            .sum::<f64>()
            / 4.0;
        power(x, g) / around.max(1e-12)
    };
    let score = |f: f64, divs: [f64; 2]| -> f64 {
        bands
            .iter()
            .map(|b| {
                divs.iter()
                    .filter(|&&d| f / d * b.6.len() as f64 > 3.0)
                    .map(|&d| prominence(&b.6, f / d))
                    .fold(0.0, f64::max)
            })
            .fold(0.0, f64::max)
    };
    let metric: Vec<(usize, f64)> = clusters
        .iter()
        .map(|&c| (c, bands[c].1))
        .filter(|&(_, f)| {
            let m = score(f, [5.0, 10.0]);
            m > 4.0 && m > 1.3 * score(f, [4.0, 8.0])
        })
        .collect();
    let marks = !metric.is_empty();
    let pick = metric
        .first()
        .map(|m| m.0)
        .or_else(|| order.first().copied())?;
    let (s, f, ratio, _, t0, x, _) = bands[pick].clone();
    let f = refine(&x, f);
    let period = 1.0 / f;
    // Perspective changes the spacing along the ruler. Measure the spacing in windows of
    // about 12 ticks, fit spacing against position with a straight line, and read it
    // where it matters (the point of the ruler nearest the shaft).
    let local = local_periods(&x, period);
    let (alpha, beta, rms_rel) = fit_line_weighted(&local)?;
    let span = x.len() as f64;
    let t_focus = focus
        .map(|(fx, fy)| (fx - cx) * u.0 + (fy - cy) * u.1 - t0)
        .unwrap_or(span / 2.0)
        .clamp(local[0].0, local[local.len() - 1].0);
    let px_per_mm = alpha + beta * t_focus;
    let change = (beta * (local[local.len() - 1].0 - local[0].0)).abs() / px_per_mm;
    let ratio_q = ((ratio - 10.0) / 40.0).clamp(0.0, 1.0);
    let count_q = (span / px_per_mm / 30.0).clamp(0.0, 1.0);
    // Windows overlap three times, so about a third of them are independent.
    let se = rms_rel / (local.len() as f64 / 3.0).max(1.0).sqrt();
    let fit_q = (1.0 - se / 0.02).clamp(0.0, 1.0);
    let tilt_q = (1.0 - change / 0.15).clamp(0.3, 1.0);
    let confidence = 0.95 * ratio_q * count_q * fit_q * tilt_q * if marks { 1.0 } else { 0.7 };
    let t_a = t_focus - 5.0 * px_per_mm;
    let pt = |t: f64| [cx + (t0 + t) * u.0 + s * v.0, cy + (t0 + t) * u.1 + s * v.1];
    Some(RulerFit {
        px_per_mm,
        p1: pt(t_a),
        p2: pt(t_a + 10.0 * px_per_mm),
        span_px: span,
        peak_ratio: ratio,
        cm_marks: marks,
        windows: local.len(),
        fit_rms: rms_rel,
        scale_change: change,
        confidence,
    })
}

/// Local tick spacing in sliding windows: (window centre, spacing, weight).
fn local_periods(x: &[f64], p0: f64) -> Vec<(f64, f64, f64)> {
    let win = ((12.0 * p0) as usize).clamp(48, x.len());
    let step = (win / 3).max(1);
    let mut out = Vec::new();
    let mut start = 0;
    while start + win <= x.len() {
        let seg = &x[start..start + win];
        let n = win as f64;
        let w: Vec<f64> = seg
            .iter()
            .enumerate()
            .map(|(i, v)| v * (0.5 - 0.5 * (2.0 * PI * i as f64 / (n - 1.0)).cos()))
            .collect();
        // Search near the overall spacing only, so harmonics cannot win.
        let (f0, f1) = (1.0 / (1.2 * p0), 1.0 / (0.83 * p0));
        let mut f = f0;
        let mut best = (f0, 0.0);
        let mut all = Vec::new();
        while f <= f1 {
            let pw = power(&w, f);
            all.push(pw);
            if pw > best.1 {
                best = (f, pw);
            }
            f += 1.0 / (4.0 * n);
        }
        // Golden section inside the main lobe.
        let (mut a, mut b) = (best.0 - 1.0 / n, best.0 + 1.0 / n);
        let g = (5f64.sqrt() - 1.0) / 2.0;
        for _ in 0..40 {
            let c = b - g * (b - a);
            let d = a + g * (b - a);
            if power(&w, c) > power(&w, d) {
                b = d;
            } else {
                a = c;
            }
        }
        let fpk = (a + b) / 2.0;
        all.sort_by(f64::total_cmp);
        let floor = all[all.len() / 4].max(1e-12);
        let weight = (power(&w, fpk) / floor).ln().max(0.0);
        if weight > 1.0 {
            out.push((start as f64 + n / 2.0, 1.0 / fpk, weight));
        }
        start += step;
    }
    out
}

/// Weighted least squares spacing = alpha + beta * t, dropping windows more than 3 %
/// off once. Returns (alpha, beta, relative rms).
fn fit_line_weighted(pts: &[(f64, f64, f64)]) -> Option<(f64, f64, f64)> {
    let solve = |pts: &[(f64, f64, f64)]| -> Option<(f64, f64)> {
        let sw: f64 = pts.iter().map(|p| p.2).sum();
        if pts.len() < 2 || sw <= 0.0 {
            return None;
        }
        let mt = pts.iter().map(|p| p.2 * p.0).sum::<f64>() / sw;
        let mp = pts.iter().map(|p| p.2 * p.1).sum::<f64>() / sw;
        let stt: f64 = pts.iter().map(|p| p.2 * (p.0 - mt).powi(2)).sum();
        let stp: f64 = pts.iter().map(|p| p.2 * (p.0 - mt) * (p.1 - mp)).sum();
        let beta = if stt > 1e-9 { stp / stt } else { 0.0 };
        Some((mp - beta * mt, beta))
    };
    if pts.len() < 3 {
        return None;
    }
    let (a, b) = solve(pts)?;
    let kept: Vec<(f64, f64, f64)> = pts
        .iter()
        .copied()
        .filter(|p| ((p.1 - (a + b * p.0)) / p.1).abs() < 0.03)
        .collect();
    let (a, b) = solve(if kept.len() >= 3 { &kept } else { pts })?;
    let used = if kept.len() >= 3 { &kept[..] } else { pts };
    let sw: f64 = used.iter().map(|p| p.2).sum();
    let rms = (used
        .iter()
        .map(|p| p.2 * ((p.1 - (a + b * p.0)) / p.1).powi(2))
        .sum::<f64>()
        / sw)
        .sqrt();
    Some((a, b, rms))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A synthetic ruler: light body, dark ticks every `period` px (longer every 5 and 10),
    /// rotated by `angle`, with a glare gradient and pixel noise.
    pub fn synth_ruler(period: f64, angle: f64) -> Gray {
        let (w, h) = (1200usize, 500usize);
        let (cx, cy) = (600.0, 250.0);
        let (c, s) = (angle.cos(), angle.sin());
        let mut px = vec![0u8; w * h];
        let mut seed: u32 = 7;
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f64 - cx, y as f64 - cy);
                let t = dx * c + dy * s;
                let n = -dx * s + dy * c;
                let mut v = 60.0 + 0.05 * x as f64;
                if n.abs() < 120.0 {
                    v = 200.0 - 0.06 * x as f64;
                    let k = (t / period).round();
                    let len = if k as i64 % 10 == 0 {
                        70.0
                    } else if k as i64 % 5 == 0 {
                        50.0
                    } else {
                        35.0
                    };
                    if (t - k * period).abs() < 0.9 && n > 120.0 - len - 10.0 && n < 110.0 {
                        v = 40.0;
                    }
                }
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                v += ((seed >> 16) % 21) as f64 - 10.0;
                px[y * w + x] = v.clamp(0.0, 255.0) as u8;
            }
        }
        Gray { w, h, px }
    }

    #[test]
    fn reads_the_tick_spacing() {
        for (period, angle) in [(14.94, 0.0), (7.3, 0.05), (22.1, -0.08)] {
            let img = synth_ruler(period, angle);
            let fit = read_ticks(&img, 600.0, 250.0, angle, 520.0, 118.0, None).expect("fit");
            let err = (fit.px_per_mm - period).abs() / period;
            assert!(err < 0.002, "period {period}: {fit:?}");
            assert!(fit.cm_marks, "{fit:?}");
            assert!(fit.confidence > 0.6, "{fit:?}");
        }
    }

    #[test]
    fn plain_surface_has_no_ticks() {
        let mut img = synth_ruler(10.0, 0.0);
        img.px.iter_mut().for_each(|p| *p = 128);
        let fit = read_ticks(&img, 600.0, 250.0, 0.0, 520.0, 100.0, None);
        assert!(fit.as_ref().is_none_or(|f| f.confidence < 0.2), "{fit:?}");
    }
}
