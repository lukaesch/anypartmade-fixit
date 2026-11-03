//! The "see" step: from SAM objects for coin, shaft and knob to millimetres, with a
//! confidence for each number and outlines for the page.

use crate::measure::{self, DFit, KnobFit};
use crate::sam::Object;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// (key, SAM concept prompt). One short noun phrase per request, as the docs ask.
pub const CONCEPTS: [(&str, &str); 4] = [
    ("coin", "coin"),
    ("shaft", "shaft"),
    ("knob", "knob"),
    ("ruler", "ruler"),
];

/// Other names for the shaft, tried one at a time when SAM finds no "shaft".
/// A bare shaft seen end on often reads as a pin or a rod.
pub const SHAFT_FALLBACKS: [(&str, &str); 3] = [
    ("shaft_pin", "metal pin"),
    ("shaft_rod", "rod"),
    ("shaft_stem", "knob shaft"),
];

fn conf(o: &Object) -> f64 {
    o.confidence.unwrap_or(0.5).clamp(0.0, 1.0)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Shaft numbers typed from calipers. They replace the photo numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManualShaft {
    pub diameter_mm: f64,
    pub flat_to_round_mm: f64,
}

impl ManualShaft {
    pub fn validate(diameter_mm: f64, flat_to_round_mm: f64) -> Result<Self, String> {
        if !(2.0..=12.0).contains(&diameter_mm) {
            return Err("The shaft diameter must be between 2 and 12 mm.".into());
        }
        if !(diameter_mm * 0.5..=diameter_mm).contains(&flat_to_round_mm) {
            return Err(
                "Flat to round must be between half the diameter and the diameter. Use the diameter for a round shaft."
                    .into(),
            );
        }
        Ok(ManualShaft {
            diameter_mm: round2(diameter_mm),
            flat_to_round_mm: round2(flat_to_round_mm),
        })
    }
}

/// Scale from a ruler: two points the user tapped on the photo and the distance
/// between them in mm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RulerScale {
    pub p1: [f64; 2],
    pub p2: [f64; 2],
    pub mm: f64,
}

impl RulerScale {
    pub fn validate(p1: [f64; 2], p2: [f64; 2], mm: f64, w: u32, h: u32) -> Result<Self, String> {
        let inside = |p: [f64; 2]| {
            p.iter().all(|v| v.is_finite())
                && (0.0..=w as f64).contains(&p[0])
                && (0.0..=h as f64).contains(&p[1])
        };
        if !inside(p1) || !inside(p2) {
            return Err("Tap both marks on the photo.".into());
        }
        if !(1.0..=300.0).contains(&mm) {
            return Err("The distance between the marks must be between 1 and 300 mm.".into());
        }
        let r = RulerScale {
            p1,
            p2,
            mm: round2(mm),
        };
        if r.px() < 60.0 {
            return Err("The marks are too close together. Pick two marks further apart.".into());
        }
        Ok(r)
    }

    pub fn px(&self) -> f64 {
        (self.p2[0] - self.p1[0]).hypot(self.p2[1] - self.p1[1])
    }

    pub fn mm_per_px(&self) -> f64 {
        self.mm / self.px()
    }

    /// A tap is good to a few pixels, so longer spans give a surer scale.
    pub fn confidence(&self) -> f64 {
        0.95 * (self.px() / 400.0).clamp(0.0, 1.0)
    }
}

/// Where the scale comes from: a coin found by SAM, or two ruler marks.
pub enum ScaleSource<'a> {
    Coin {
        diameter_mm: f64,
        objects: &'a [Object],
    },
    Ruler(&'a RulerScale),
    /// Ruler found by SAM and read from its ticks; `None` when that failed.
    RulerFound {
        object: Option<&'a Object>,
        fit: Option<&'a crate::ruler::RulerFit>,
    },
}

/// Centre of the most confident shaft, where the scale matters most.
pub fn shaft_centre(objs: &[Object]) -> Option<(f64, f64)> {
    let o = best(objs)?;
    measure::fit_d(&measure::edge_points(o)).map(|d| (d.cx, d.cy))
}

pub fn best_object(objs: &[Object]) -> Option<&Object> {
    best(objs)
}

/// Best coin: confident and round.
fn pick_coin(objs: &[Object]) -> Option<(&Object, measure::Moments)> {
    objs.iter()
        .filter_map(|o| measure::moments(o).map(|m| (o, m)))
        .max_by(|a, b| {
            let s = |(o, m): &(&Object, measure::Moments)| conf(o) * m.minor / m.major.max(1e-9);
            s(a).total_cmp(&s(b))
        })
}

fn best(objs: &[Object]) -> Option<&Object> {
    objs.iter().max_by(|a, b| conf(a).total_cmp(&conf(b)))
}

fn overlay(label: &str, o: &Object, cx: f64, cy: f64, text: String) -> Value {
    let pts = measure::edge_points(o);
    json!({
        "label": label,
        "confidence": round2(conf(o)),
        "box": [o.x1, o.y1, o.x2 + 1, o.y2 + 1],
        "outline": measure::outline(&pts, cx, cy, 160),
        "text": text,
    })
}

/// Turns the three SAM results into measurements. `coin_mm` is the chosen coin.
/// The shaft seen from the side: the diameter is its width; the D flat cannot be seen.
fn side_shaft(
    o: &Object,
    mmpp: f64,
    scale_c: f64,
    overlays: &mut Vec<Value>,
    notes: &mut Vec<String>,
) -> Option<Value> {
    let f = measure::side_width(o)?;
    let size_q = (f.width_px / 40.0).clamp(0.0, 1.0);
    let straight_q = (1.0 - f.spread_px / (0.1 * f.width_px).max(1.0)).clamp(0.0, 1.0);
    let c = conf(o) * scale_c * size_q * (0.5 + 0.5 * straight_q);
    let text = if mmpp > 0.0 {
        format!("{:.2} mm across", f.width_px * mmpp)
    } else {
        "shaft".into()
    };
    overlays.push(overlay("shaft", o, f.cx, f.cy, text));
    notes.push(
        "The shaft is seen from the side, so its width gives the diameter but the D flat is hidden. Type the flat from calipers, or take a photo straight at the shaft end."
            .into(),
    );
    Some(json!({
        "view": "side",
        "diameter_mm": (mmpp > 0.0).then(|| round2(f.width_px * mmpp)),
        "flat_to_round_mm": Value::Null,
        "has_flat": false,
        "diameter_px": round2(f.width_px),
        "length_mm": (mmpp > 0.0).then(|| round2(f.length_px * mmpp)),
        "plus_minus_mm": (mmpp > 0.0).then(|| round2(mmpp * (1.0 + f.spread_px))),
        "confidence": round2(c),
    }))
}

pub fn assemble(source: ScaleSource, shaft: &[Object], knob: &[Object]) -> Value {
    let mut notes: Vec<String> = Vec::new();
    let mut overlays = Vec::new();

    let scale = match source {
        ScaleSource::Coin {
            diameter_mm: coin_mm,
            objects: coin,
        } => {
            let s = pick_coin(coin).map(|(o, m)| {
                let roundness = m.minor / m.major.max(1e-9);
                let round_q = ((roundness - 0.85) / 0.12).clamp(0.0, 1.0);
                let size_q = (m.major / 120.0).clamp(0.0, 1.0);
                let c = conf(o) * round_q * size_q;
                let mm_per_px = coin_mm / m.major;
                overlays.push(overlay("coin", o, m.cx, m.cy, format!("{coin_mm} mm")));
                if roundness < 0.95 {
                    notes.push(
                        "The coin looks tilted. Hold the phone straight above for better numbers."
                            .into(),
                    );
                }
                if m.major < 120.0 {
                    notes.push("The coin is small in the photo. Step closer or zoom in.".into());
                }
                (
                    mm_per_px,
                    json!({
                        "source": "coin",
                        "mm_per_px": mm_per_px,
                        "coin_px": round2(m.major),
                        "coin_roundness": round3(roundness),
                        "confidence": round2(c),
                    }),
                    c,
                )
            });
            if s.is_none() {
                notes.push(
                    "No coin found. Put the coin flat next to the shaft and take the photo again."
                        .into(),
                );
            }
            s
        }
        ScaleSource::RulerFound { object, fit } => {
            if let Some(o) = object {
                if let Some(m) = measure::moments(o) {
                    overlays.push(overlay("ruler", o, m.cx, m.cy, "found".into()));
                }
            }
            match fit {
                Some(f) if f.confidence > 0.2 => {
                    overlays.push(json!({
                        "label": "ruler",
                        "confidence": round2(f.confidence),
                        "line": [f.p1, f.p2],
                        "text": "10 mm from the ticks",
                    }));
                    if f.scale_change > 0.04 {
                        notes.push(format!(
                            "The ruler is tilted: its scale changes {:.0}% along it. We used the scale next to the shaft.",
                            f.scale_change * 100.0
                        ));
                    }
                    notes.push(
                        "If the shaft end sits deeper or higher than the ruler, the numbers can be off by a few percent."
                            .into(),
                    );
                    Some((
                        1.0 / f.px_per_mm,
                        json!({
                            "source": "ruler_ticks",
                            "mm_per_px": 1.0 / f.px_per_mm,
                            "px_per_mm": round3(f.px_per_mm),
                            "scale_change": round3(f.scale_change),
                            "cm_marks": f.cm_marks,
                            "confidence": round2(f.confidence),
                        }),
                        f.confidence,
                    ))
                }
                _ => {
                    notes.push(if object.is_some() {
                        "We found the ruler but could not read its millimetre ticks. Tap two marks on the ruler instead.".into()
                    } else {
                        "No ruler found. Tap two marks on the ruler, or take the photo again with the ruler flat next to the shaft.".into()
                    });
                    None
                }
            }
        }
        ScaleSource::Ruler(r) => {
            let c = r.confidence();
            overlays.push(json!({
                "label": "ruler",
                "confidence": round2(c),
                "line": [r.p1, r.p2],
                "text": format!("{} mm", r.mm),
            }));
            notes.push(
                "The scale comes from the ruler. If the shaft end sits deeper or higher than the ruler, the numbers can be off by a few percent."
                    .into(),
            );
            Some((
                r.mm_per_px(),
                json!({
                    "source": "ruler",
                    "mm_per_px": r.mm_per_px(),
                    "ruler_px": round2(r.px()),
                    "ruler_mm": r.mm,
                    "confidence": round2(c),
                }),
                c,
            ))
        }
    };
    let (mmpp, scale_c) = scale.as_ref().map(|s| (s.0, s.2)).unwrap_or((0.0, 0.0));

    let shaft_v = best(shaft).and_then(|o| {
        let m = measure::moments(o)?;
        if m.major / m.minor.max(1e-9) > measure::SIDE_VIEW_ELONGATION {
            return side_shaft(o, mmpp, scale_c, &mut overlays, &mut notes);
        }
        let d: DFit = measure::fit_d(&measure::edge_points(o))?;
        let size_q = (d.diameter_px / 40.0).clamp(0.0, 1.0);
        let fit_q = (1.0 - d.rms_px / (0.04 * d.diameter_px).max(0.5)).clamp(0.0, 1.0);
        let c = conf(o) * scale_c * size_q * fit_q;
        let text = if mmpp > 0.0 {
            format!(
                "{:.2} mm, flat {:.2} mm",
                d.diameter_px * mmpp,
                d.flat_to_round_px * mmpp
            )
        } else {
            "shaft".into()
        };
        overlays.push(overlay("shaft", o, d.cx, d.cy, text));
        if d.diameter_px < 40.0 {
            notes.push("The shaft is small in the photo. Zoom in so it is at least 40 px wide, or type the calipers reading.".into());
        }
        Some(json!({
            "view": "end",
            "diameter_mm": (mmpp > 0.0).then(|| round2(d.diameter_px * mmpp)),
            "flat_to_round_mm": (mmpp > 0.0 && d.has_flat).then(|| round2(d.flat_to_round_px * mmpp)),
            "has_flat": d.has_flat,
            "diameter_px": round2(d.diameter_px),
            "flat_to_round_px": round2(d.flat_to_round_px),
            "plus_minus_mm": (mmpp > 0.0).then(|| round2(mmpp * (0.5 + d.rms_px))),
            "confidence": round2(c),
        }))
    });
    if shaft_v.is_none() {
        notes
            .push("No shaft found. Type the shaft diameter and flat from calipers instead.".into());
    }

    let knob_v = best(knob).and_then(|o| {
        let k: KnobFit = measure::fit_knob(o)?;
        let c = conf(o) * scale_c;
        let text = if mmpp > 0.0 {
            format!("{:.1} mm, {} ribs", k.outer_diameter_px * mmpp, k.rib_count)
        } else {
            "knob".into()
        };
        overlays.push(overlay("knob", o, k.cx, k.cy, text));
        Some(json!({
            "outer_diameter_mm": (mmpp > 0.0).then(|| round2(k.outer_diameter_px * mmpp)),
            "rib_count": k.rib_count,
            "rib_depth_mm": (mmpp > 0.0).then(|| round2(k.rib_depth_px * mmpp)),
            "rib_clarity": round2(k.rib_clarity),
            "confidence": round2(c),
        }))
    });
    if knob_v.is_none() {
        notes.push("No knob found. Muse will design from your description.".into());
    }

    json!({
        "scale": scale.map(|s| s.1),
        "shaft": shaft_v,
        "knob": knob_v,
        "overlays": overlays,
        "notes": notes,
        "found": {"shaft": shaft.len(), "knob": knob.len()},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::tests::synth;

    #[test]
    fn coin_and_d_shaft_give_millimetres() {
        // 1 euro at 6 px per mm (139.5 px), 6.0 x 4.6 mm D shaft (36 x 27.6 px).
        let coin = synth(150, 150, 200, 200, |x, y| {
            (x - 75.0).hypot(y - 75.0) < 69.75
        });
        let shaft = synth(50, 50, 800, 600, |x, y| {
            let (u, v) = (x - 25.0, y - 25.0);
            u.hypot(v) < 18.0 && v > -(27.6 - 18.0)
        });
        let out = assemble(
            ScaleSource::Coin {
                diameter_mm: 23.25,
                objects: &[coin],
            },
            &[shaft],
            &[],
        );
        let d = out["shaft"]["diameter_mm"].as_f64().expect("diameter");
        let f = out["shaft"]["flat_to_round_mm"].as_f64().expect("flat");
        assert!((d - 6.0).abs() < 0.1, "{out}");
        assert!((f - 4.6).abs() < 0.1, "{out}");
        assert_eq!(out["shaft"]["has_flat"], true);
        assert!(out["scale"]["confidence"].as_f64().unwrap() > 0.8);
        assert_eq!(out["overlays"].as_array().unwrap().len(), 2);
        assert!(out["notes"].to_string().contains("No knob found"));
    }

    #[test]
    fn nothing_found_explains_what_to_do() {
        let out = assemble(
            ScaleSource::Coin {
                diameter_mm: 23.25,
                objects: &[],
            },
            &[],
            &[],
        );
        assert!(out["scale"].is_null() && out["shaft"].is_null());
        assert!(out["notes"].to_string().contains("No coin found"));
        assert!(out["notes"].to_string().contains("calipers"));
    }

    #[test]
    fn ruler_gives_the_scale() {
        // 10 mm between the marks, 600 px apart: 0.0167 mm per px; shaft 360 px is 6 mm.
        let r =
            RulerScale::validate([100.0, 200.0], [700.0, 200.0], 10.0, 1450, 2576).expect("ruler");
        let shaft = synth(400, 400, 500, 900, |x, y| {
            let (u, v) = (x - 200.0, y - 200.0);
            u.hypot(v) < 180.0 && v > -(276.0 - 180.0)
        });
        let out = assemble(ScaleSource::Ruler(&r), &[shaft], &[]);
        assert_eq!(out["scale"]["source"], "ruler");
        let d = out["shaft"]["diameter_mm"].as_f64().expect("d");
        let f = out["shaft"]["flat_to_round_mm"].as_f64().expect("f");
        assert!((d - 6.0).abs() < 0.05 && (f - 4.6).abs() < 0.05, "{out}");
        assert_eq!(out["overlays"][0]["label"], "ruler");
        assert!(RulerScale::validate([0.0, 0.0], [20.0, 0.0], 10.0, 100, 100).is_err());
        assert!(RulerScale::validate([0.0, 0.0], [2000.0, 0.0], 10.0, 1450, 2576).is_err());
        assert!(RulerScale::validate([0.0, 0.0], [300.0, 0.0], 0.0, 1450, 2576).is_err());
    }

    #[test]
    fn unreadable_ruler_asks_for_taps() {
        let out = assemble(
            ScaleSource::RulerFound {
                object: None,
                fit: None,
            },
            &[],
            &[],
        );
        assert!(out["scale"].is_null());
        assert!(out["notes"].to_string().contains("Tap two marks"));
    }

    #[test]
    fn manual_shaft_is_bounded() {
        assert!(ManualShaft::validate(6.0, 4.6).is_ok());
        assert!(ManualShaft::validate(6.0, 6.0).is_ok());
        assert!(ManualShaft::validate(6.0, 7.0).is_err());
        assert!(ManualShaft::validate(40.0, 30.0).is_err());
    }
}
