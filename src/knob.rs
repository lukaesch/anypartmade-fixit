//! KnobSpec, the feature language Muse writes, and the Rust that turns it into a mesh.
//! Muse never writes geometry: only this JSON. Rust checks the bounds and builds.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobSpec {
    pub body: Body,
    pub grip: Grip,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Pointer>,
    pub bore: Bore,
    pub shell: Shell,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    pub outer_diameter_mm: f64,
    pub height_mm: f64,
    pub top: Top,
    #[serde(default)]
    pub top_size_mm: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skirt: Option<Skirt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Top {
    Flat,
    Dome,
    Chamfer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skirt {
    pub diameter_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grip {
    pub kind: GripKind,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub depth_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GripKind {
    None,
    Ribs,
    Knurl,
    Flats,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pointer {
    pub kind: PointerKind,
    pub width_mm: f64,
    pub depth_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerKind {
    None,
    Groove,
    Ridge,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bore {
    pub shape: BoreShape,
    pub diameter_mm: f64,
    pub flat_to_round_mm: f64,
    pub depth_mm: f64,
    #[serde(default)]
    pub clearance_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BoreShape {
    D,
    Round,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shell {
    pub hollow: bool,
    pub wall_mm: f64,
}

fn within(name: &str, v: f64, lo: f64, hi: f64) -> Result<(), String> {
    if v.is_finite() && v >= lo && v <= hi {
        Ok(())
    } else {
        Err(format!("{name} must be from {lo} to {hi} mm, not {v}."))
    }
}

impl KnobSpec {
    /// A plain knob around the measured shaft, the start before Muse designs.
    pub fn around_shaft(diameter_mm: f64, flat_to_round_mm: f64) -> Self {
        KnobSpec {
            body: Body {
                outer_diameter_mm: 36.0,
                height_mm: 20.0,
                top: Top::Chamfer,
                top_size_mm: 1.5,
                skirt: None,
            },
            grip: Grip {
                kind: GripKind::Ribs,
                count: 18,
                depth_mm: 0.8,
            },
            pointer: Some(Pointer {
                kind: PointerKind::Groove,
                width_mm: 1.5,
                depth_mm: 0.8,
            }),
            bore: Bore {
                shape: BoreShape::D,
                diameter_mm,
                flat_to_round_mm,
                depth_mm: 12.0,
                clearance_mm: 0.0,
            },
            shell: Shell {
                hollow: true,
                wall_mm: 2.0,
            },
        }
    }

    /// The bounds from the spec, plus what the mesh needs to stay a valid solid.
    pub fn validate(&self) -> Result<(), String> {
        let b = &self.body;
        within("body.outer_diameter_mm", b.outer_diameter_mm, 15.0, 80.0)?;
        within("body.height_mm", b.height_mm, 8.0, 50.0)?;
        within("bore.diameter_mm", self.bore.diameter_mm, 3.0, 12.0)?;
        within("bore.clearance_mm", self.bore.clearance_mm, 0.0, 1.0)?;
        within("shell.wall_mm", self.shell.wall_mm, 1.6, 10.0)?;
        within("bore.depth_mm", self.bore.depth_mm, 3.0, b.height_mm - 2.0)?;
        if self.bore.shape == BoreShape::D {
            within(
                "bore.flat_to_round_mm",
                self.bore.flat_to_round_mm,
                0.55 * self.bore.diameter_mm,
                0.95 * self.bore.diameter_mm,
            )?;
        }
        let r = b.outer_diameter_mm / 2.0;
        match b.top {
            Top::Flat => {}
            Top::Dome => within("body.top_size_mm", b.top_size_mm, 0.5, b.height_mm / 3.0)?,
            Top::Chamfer => within("body.top_size_mm", b.top_size_mm, 0.3, (r / 4.0).min(5.0))?,
        }
        if let Some(s) = &b.skirt {
            within(
                "body.skirt.diameter_mm",
                s.diameter_mm,
                b.outer_diameter_mm + 1.0,
                90.0,
            )?;
            within("body.skirt.height_mm", s.height_mm, 1.0, b.height_mm / 2.0)?;
        }
        let g = &self.grip;
        if g.kind != GripKind::None {
            let (lo, hi) = match g.kind {
                GripKind::Flats => (3, 12),
                GripKind::Knurl => (20, 90),
                _ => (6, 60),
            };
            if g.count < lo || g.count > hi {
                return Err(format!(
                    "grip.count for {:?} must be from {lo} to {hi}.",
                    g.kind
                ));
            }
            if g.kind != GripKind::Flats {
                within("grip.depth_mm", g.depth_mm, 0.3, 2.0)?;
            }
        }
        if let Some(p) = &self.pointer {
            if p.kind != PointerKind::None {
                within("pointer.width_mm", p.width_mm, 0.8, 4.0)?;
                within("pointer.depth_mm", p.depth_mm, 0.3, 2.0)?;
            }
        }
        // The solid between the bore and the outside must hold the wall.
        let inner = self.bore.diameter_mm / 2.0 + self.bore.clearance_mm / 2.0;
        if self.min_outer_radius() - inner < self.shell.wall_mm + 1.0 {
            return Err("The knob is too small around the bore for this wall.".into());
        }
        Ok(())
    }

    /// Smallest radius of the outside, after grip and pointer.
    fn min_outer_radius(&self) -> f64 {
        let r = self.body.outer_diameter_mm / 2.0;
        let grip = match self.grip.kind {
            GripKind::None => 0.0,
            GripKind::Flats => r * (1.0 - (PI / self.grip.count.max(3) as f64).cos()),
            _ => self.grip.depth_mm,
        };
        let pointer = match &self.pointer {
            Some(p) if p.kind == PointerKind::Groove => p.depth_mm,
            _ => 0.0,
        };
        r - grip.max(pointer)
    }

    /// Radius of the bore at angle `t`. The D flat faces +x, where the pointer is.
    pub fn bore_radius(&self, t: f64) -> f64 {
        let a = self.bore.diameter_mm / 2.0 + self.bore.clearance_mm / 2.0;
        if self.bore.shape == BoreShape::Round {
            return a;
        }
        let flat = self.bore.flat_to_round_mm + self.bore.clearance_mm - a;
        let c = t.cos();
        if c > 1e-9 && flat / c < a {
            flat / c
        } else {
            a
        }
    }

    /// Radius of the outside at angle `t`: grip and pointer cut into the round body.
    pub fn outer_radius(&self, t: f64) -> f64 {
        let r = self.body.outer_diameter_mm / 2.0;
        let g = &self.grip;
        let n = g.count.max(1) as f64;
        let mut out = match g.kind {
            GripKind::None => r,
            GripKind::Ribs | GripKind::Knurl => r - g.depth_mm * (0.5 - 0.5 * (n * t).cos()),
            GripKind::Flats => {
                let seg = 2.0 * PI / n;
                let u = (t + seg / 2.0).rem_euclid(seg) - seg / 2.0;
                r * (PI / n).cos() / u.cos()
            }
        };
        if let Some(p) = &self.pointer {
            let half = p.width_mm / 2.0 / r;
            let d = (t + PI).rem_euclid(2.0 * PI) - PI;
            if d.abs() <= half {
                match p.kind {
                    PointerKind::Groove => out = out.min(r - p.depth_mm),
                    PointerKind::Ridge => out = out.max(r + p.depth_mm),
                    PointerKind::None => {}
                }
            }
        }
        out
    }

    /// Angles to sample around the axis: fine enough for the grip, with the exact
    /// corners of the D flat and the pointer so their edges are sharp.
    fn angles(&self) -> Vec<f64> {
        let n = match self.grip.kind {
            GripKind::Ribs | GripKind::Knurl => (self.grip.count as usize * 12).max(240),
            _ => 240,
        };
        let mut a: Vec<f64> = (0..n).map(|i| 2.0 * PI * i as f64 / n as f64).collect();
        let r = self.body.outer_diameter_mm / 2.0;
        if self.bore.shape == BoreShape::D {
            let ra = self.bore.diameter_mm / 2.0 + self.bore.clearance_mm / 2.0;
            let flat = self.bore.flat_to_round_mm + self.bore.clearance_mm - ra;
            let c = (flat / ra).acos();
            a.extend([c, 2.0 * PI - c]);
        }
        if let Some(p) = &self.pointer {
            if p.kind != PointerKind::None {
                let h = p.width_mm / 2.0 / r;
                // Both sides of each pointer edge, so the wall drops straight down.
                for e in [h, 2.0 * PI - h] {
                    a.extend([e - 1e-4, e + 1e-4]);
                }
            }
        }
        if self.grip.kind == GripKind::Flats {
            let n = self.grip.count as f64;
            a.extend((0..self.grip.count).map(|i| (2.0 * i as f64 + 1.0) * PI / n));
        }
        a.sort_by(f64::total_cmp);
        a.dedup_by(|x, y| (*x - *y).abs() < 1e-6);
        a
    }

    /// The cross section from the top centre to the bore top, as (radius kind, r, z).
    /// Radius kinds follow the angle: outside, bore, or fixed.
    fn profile(&self) -> Vec<(Ring, f64, f64)> {
        let b = &self.body;
        let (h, s) = (b.height_mm, b.top_size_mm);
        let mut p = vec![(Ring::Axis, 0.0, h)];
        match b.top {
            Top::Flat => p.push((Ring::Outer(1.0), 0.0, h)),
            Top::Chamfer => {
                p.push((Ring::OuterLess(s), 0.0, h));
                p.push((Ring::Outer(1.0), 0.0, h - s));
            }
            Top::Dome => {
                for k in 1..=8 {
                    let f = k as f64 / 8.0;
                    p.push((Ring::Outer(f), 0.0, h - s * f * f));
                }
            }
        }
        let foot = match &b.skirt {
            Some(sk) => {
                p.push((Ring::Outer(1.0), 0.0, sk.height_mm));
                p.push((Ring::Fixed, sk.diameter_mm / 2.0, sk.height_mm));
                p.push((Ring::Fixed, sk.diameter_mm / 2.0, 0.0));
                sk.diameter_mm / 2.0
            }
            None => {
                p.push((Ring::Outer(1.0), 0.0, 0.0));
                self.min_outer_radius()
            }
        };
        let w = self.shell.wall_mm;
        let a = self.bore.diameter_mm / 2.0 + self.bore.clearance_mm / 2.0;
        let cavity_out = foot.min(self.min_outer_radius()) - w;
        let boss = a + w;
        let roof = h - w - if b.top == Top::Dome { s } else { 0.0 };
        if self.shell.hollow && cavity_out - boss >= 1.0 && roof >= 2.0 {
            p.push((Ring::Fixed, cavity_out, 0.0));
            p.push((Ring::Fixed, cavity_out, roof));
            p.push((Ring::Fixed, boss, roof));
            p.push((Ring::Fixed, boss, 0.0));
        }
        p.push((Ring::Bore, 0.0, 0.0));
        p.push((Ring::Bore, 0.0, self.bore.depth_mm));
        p.push((Ring::Axis, 0.0, self.bore.depth_mm));
        p
    }

    fn ring_radius(&self, ring: Ring, r: f64, t: f64) -> f64 {
        match ring {
            Ring::Axis => 0.0,
            Ring::Fixed => r,
            Ring::Bore => self.bore_radius(t),
            Ring::Outer(f) => self.outer_radius(t) * f,
            Ring::OuterLess(d) => self.outer_radius(t) - d,
        }
    }

    /// Watertight triangle mesh: the profile swept around the z axis.
    /// The knob stands on z = 0 with the bore opening down, axis along z.
    pub fn mesh(&self) -> Mesh {
        let angles = self.angles();
        let prof = self.profile();
        let n = angles.len();
        let mut m = Mesh::default();
        // Vertex index per (profile point, angle); poles are one vertex.
        let mut idx: Vec<Vec<u32>> = Vec::new();
        for &(ring, r, z) in &prof {
            if ring == Ring::Axis {
                m.vertices.push([0.0, 0.0, z]);
                idx.push(vec![(m.vertices.len() - 1) as u32; n]);
                continue;
            }
            let mut row = Vec::with_capacity(n);
            for &t in &angles {
                let rr = self.ring_radius(ring, r, t);
                m.vertices.push([rr * t.cos(), rr * t.sin(), z]);
                row.push((m.vertices.len() - 1) as u32);
            }
            idx.push(row);
        }
        for k in 0..prof.len() - 1 {
            for i in 0..n {
                let j = (i + 1) % n;
                let (a, b, c, d) = (idx[k][i], idx[k][j], idx[k + 1][i], idx[k + 1][j]);
                if a != b {
                    m.triangles.push([a, c, b]);
                }
                if c != d {
                    m.triangles.push([b, c, d]);
                }
            }
        }
        if m.volume() < 0.0 {
            for t in &mut m.triangles {
                t.swap(1, 2);
            }
        }
        m
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ring {
    Axis,
    Fixed,
    Bore,
    /// The outside scaled by a factor (dome rings).
    Outer(f64),
    /// The outside less a fixed amount (chamfer top edge).
    OuterLess(f64),
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

impl Mesh {
    /// Signed volume, mm³. Positive when the triangles face outwards.
    pub fn volume(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.vertices[i as usize]);
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum()
    }

    /// Every edge is used exactly twice, once in each direction, and no triangle is flat.
    pub fn is_watertight(&self) -> bool {
        use std::collections::HashMap;
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &self.triangles {
            if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
                return false;
            }
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *edges.entry((a, b)).or_default() += 1;
            }
        }
        edges
            .iter()
            .all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knob() -> KnobSpec {
        KnobSpec::around_shaft(6.0, 4.6)
    }

    #[test]
    fn default_knob_is_valid_and_watertight() {
        let k = knob();
        k.validate().unwrap();
        let m = k.mesh();
        assert!(m.is_watertight());
        let v = m.volume();
        // Between the solid cylinder and a thin shell.
        let full = PI * 18.0 * 18.0 * 20.0;
        assert!(v > 0.15 * full && v < full, "{v}");
    }

    #[test]
    fn d_bore_has_the_right_size() {
        let k = knob();
        // Round side at 3.0 mm, flat at 4.6 - 3.0 = 1.6 mm from the axis.
        assert!((k.bore_radius(PI) - 3.0).abs() < 1e-9);
        assert!((k.bore_radius(0.0) - 1.6).abs() < 1e-9);
        assert!((k.bore_radius(0.0) + k.bore_radius(PI) - 4.6).abs() < 1e-9);
        // The mesh vertices of the bore sit exactly there.
        let m = k.mesh();
        let bore: Vec<&[f64; 3]> = m
            .vertices
            .iter()
            .filter(|v| v[2] == 0.0 && (v[0] * v[0] + v[1] * v[1]).sqrt() < 3.01)
            .collect();
        let max_x = bore.iter().map(|v| v[0]).fold(f64::MIN, f64::max);
        let min_x = bore.iter().map(|v| v[0]).fold(f64::MAX, f64::min);
        assert!((max_x - 1.6).abs() < 1e-9 && (min_x + 3.0).abs() < 1e-6);
    }

    #[test]
    fn clearance_grows_the_bore() {
        let mut k = knob();
        k.bore.clearance_mm = 0.2;
        assert!((k.bore_radius(PI) - 3.1).abs() < 1e-9);
        assert!((k.bore_radius(0.0) - 1.7).abs() < 1e-9);
    }

    #[test]
    fn every_shape_is_watertight() {
        for top in [Top::Flat, Top::Dome, Top::Chamfer] {
            for kind in [
                GripKind::None,
                GripKind::Ribs,
                GripKind::Knurl,
                GripKind::Flats,
            ] {
                for skirt in [
                    None,
                    Some(Skirt {
                        diameter_mm: 42.0,
                        height_mm: 3.0,
                    }),
                ] {
                    for hollow in [true, false] {
                        let mut k = knob();
                        k.body.top = top;
                        k.body.top_size_mm = 2.0;
                        k.body.skirt = skirt.clone();
                        k.grip.kind = kind;
                        k.grip.count = match kind {
                            GripKind::Flats => 6,
                            GripKind::Knurl => 40,
                            _ => 18,
                        };
                        k.shell.hollow = hollow;
                        k.validate().unwrap();
                        let m = k.mesh();
                        assert!(m.is_watertight(), "{top:?} {kind:?} {hollow}");
                        assert!(m.volume() > 0.0);
                    }
                }
            }
        }
    }

    #[test]
    fn bounds_are_checked() {
        let mut k = knob();
        k.body.outer_diameter_mm = 90.0;
        assert!(k.validate().is_err());
        let mut k = knob();
        k.bore.depth_mm = 19.0;
        assert!(k.validate().is_err());
        let mut k = knob();
        k.shell.wall_mm = 1.0;
        assert!(k.validate().is_err());
        let mut k = knob();
        k.bore.flat_to_round_mm = 6.0;
        assert!(k.validate().is_err());
        let bad = r#"{"body":{"outer_diameter_mm":36,"height_mm":20,"top":"flat"},"grip":{"kind":"none"},"bore":{"shape":"d","diameter_mm":6,"flat_to_round_mm":4.6,"depth_mm":12},"shell":{"hollow":true,"wall_mm":2},"code":"x"}"#;
        assert!(serde_json::from_str::<KnobSpec>(bad).is_err());
    }
}
