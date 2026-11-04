//! The CAD language Muse designs in: a flat list of nodes (shapes, combinations,
//! transforms, and fit features), as JSON. Rust evaluates it as a signed distance field
//! and meshes it with marching tetrahedra, so the result is always a closed solid.
//! Fit features (a D hole for a measured shaft, a pocket for a measured board, a USB C
//! opening) take their sizes from the recorded measurements, and the checks Any Part
//! Made needs are derived from them.

use crate::knob::Mesh;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub const MAX_NODES: usize = 80;
const LIMIT_MM: f64 = 200.0;

type V3 = [f64; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn len2(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

/// Distance in a 2D cross section combined with a slab: exact for prisms.
fn prism(d2: f64, dz: f64) -> f64 {
    len2(d2.max(0.0), dz.max(0.0)) + d2.max(dz).min(0.0)
}

/// Signed distance to a closed polygon, negative inside.
fn polygon(p: [f64; 2], v: &[[f64; 2]]) -> f64 {
    let n = v.len();
    let d0 = [p[0] - v[0][0], p[1] - v[0][1]];
    let mut d = d0[0] * d0[0] + d0[1] * d0[1];
    let mut s = 1.0;
    let mut j = n - 1;
    for i in 0..n {
        let e = [v[j][0] - v[i][0], v[j][1] - v[i][1]];
        let w = [p[0] - v[i][0], p[1] - v[i][1]];
        let t =
            ((w[0] * e[0] + w[1] * e[1]) / (e[0] * e[0] + e[1] * e[1]).max(1e-12)).clamp(0.0, 1.0);
        let b = [w[0] - e[0] * t, w[1] - e[1] * t];
        d = d.min(b[0] * b[0] + b[1] * b[1]);
        let c1 = p[1] >= v[i][1];
        let c2 = p[1] < v[j][1];
        let c3 = e[0] * w[1] > e[1] * w[0];
        if (c1 && c2 && c3) || (!c1 && !c2 && !c3) {
            s = -s;
        }
        j = i;
    }
    s * d.sqrt()
}

fn smin(a: f64, b: f64, k: f64) -> f64 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

/// Rotation about x, then y, then z (degrees), as a matrix.
fn rotation(deg: V3) -> [[f64; 3]; 3] {
    let (a, b, c) = (
        deg[0].to_radians(),
        deg[1].to_radians(),
        deg[2].to_radians(),
    );
    let rx = [
        [1.0, 0.0, 0.0],
        [0.0, a.cos(), -a.sin()],
        [0.0, a.sin(), a.cos()],
    ];
    let ry = [
        [b.cos(), 0.0, b.sin()],
        [0.0, 1.0, 0.0],
        [-b.sin(), 0.0, b.cos()],
    ];
    let rz = [
        [c.cos(), -c.sin(), 0.0],
        [c.sin(), c.cos(), 0.0],
        [0.0, 0.0, 1.0],
    ];
    let m = |x: [[f64; 3]; 3], y: [[f64; 3]; 3]| {
        let mut r = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                r[i][j] = (0..3).map(|k| x[i][k] * y[k][j]).sum();
            }
        }
        r
    };
    m(rz, m(ry, rx))
}

fn apply(m: &[[f64; 3]; 3], p: V3) -> V3 {
    [dot(m[0], p), dot(m[1], p), dot(m[2], p)]
}

fn transpose(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut t = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            t[i][j] = m[j][i];
        }
    }
    t
}

fn axis_vec(s: &str) -> Option<V3> {
    Some(match s {
        "+x" | "x" => [1.0, 0.0, 0.0],
        "-x" => [-1.0, 0.0, 0.0],
        "+y" | "y" => [0.0, 1.0, 0.0],
        "-y" => [0.0, -1.0, 0.0],
        "+z" | "z" => [0.0, 0.0, 1.0],
        "-z" => [0.0, 0.0, -1.0],
        _ => return None,
    })
}

/// A local frame for a feature: n points out of the part through the opening.
#[derive(Clone, Debug)]
struct Frame {
    at: V3,
    n: V3,
    u: V3,
    w: V3,
}

impl Frame {
    fn new(at: V3, n: V3, u: V3) -> Self {
        Frame {
            at,
            n,
            u,
            w: cross(n, u),
        }
    }
    /// (along u, along w, depth into the part from the opening).
    fn local(&self, p: V3) -> (f64, f64, f64) {
        let q = sub(p, self.at);
        (dot(q, self.u), dot(q, self.w), -dot(q, self.n))
    }
}

#[derive(Clone, Debug)]
enum Node {
    Box {
        c: V3,
        half: V3,
        r: f64,
    },
    Cylinder {
        at: V3,
        r1: f64,
        r2: f64,
        h: f64,
    },
    Sphere {
        c: V3,
        r: f64,
    },
    Extrude {
        at: V3,
        pts: Vec<[f64; 2]>,
        h: f64,
    },
    Revolve {
        at: V3,
        pts: Vec<[f64; 2]>,
    },
    Union {
        of: Vec<usize>,
        k: f64,
    },
    Difference {
        of: Vec<usize>,
        k: f64,
    },
    Intersection {
        of: Vec<usize>,
    },
    Translate {
        by: V3,
        node: usize,
    },
    Rotate {
        m: [[f64; 3]; 3],
        node: usize,
    },
    Mirror {
        axis: usize,
        node: usize,
    },
    Polar {
        count: usize,
        node: usize,
    },
    Linear {
        count: usize,
        step: V3,
        node: usize,
    },
    /// A D shaped hole: radius a, the flat at `flat` from the axis towards u.
    DHole {
        f: Frame,
        a: f64,
        flat: f64,
        depth: f64,
    },
    Hole {
        f: Frame,
        r: f64,
        depth: f64,
    },
    /// A rectangular pocket: half sizes along u and w.
    Pocket {
        f: Frame,
        hu: f64,
        hw: f64,
        depth: f64,
    },
    /// A rounded slot along u (width) and w (height).
    Slot {
        f: Frame,
        hu: f64,
        hw: f64,
        depth: f64,
    },
}

/// A part ready to mesh, with the checks its fit features need.
#[derive(Debug)]
pub struct Part {
    nodes: Vec<Node>,
    root: usize,
    pub critical_dimensions: Vec<Value>,
    pub interfaces: Vec<Value>,
    /// What Rust filled in from the measurements, to show the user.
    pub fitted: Vec<Value>,
}

fn v3(v: &Value, what: &str) -> Result<V3, String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or(format!("{what} must be [x, y, z]"))?;
    let mut o = [0.0; 3];
    for i in 0..3 {
        o[i] = a[i].as_f64().ok_or(format!("{what} must be numbers"))?;
        if o[i].abs() > LIMIT_MM {
            return Err(format!("{what} is beyond {LIMIT_MM} mm"));
        }
    }
    Ok(o)
}

fn pos(v: &Value, what: &str, lo: f64, hi: f64) -> Result<f64, String> {
    let x = v.as_f64().ok_or(format!("{what} is missing"))?;
    if !(x >= lo && x <= hi) {
        return Err(format!("{what} must be from {lo} to {hi}, not {x}"));
    }
    Ok(x)
}

fn opt(v: &Value, default: f64, what: &str, lo: f64, hi: f64) -> Result<f64, String> {
    if v.is_null() {
        Ok(default)
    } else {
        pos(v, what, lo, hi)
    }
}

fn pts2(v: &Value, what: &str) -> Result<Vec<[f64; 2]>, String> {
    let a = v
        .as_array()
        .ok_or(format!("{what} must be a list of [x, y]"))?;
    if a.len() < 3 || a.len() > 96 {
        return Err(format!("{what} needs 3 to 96 points"));
    }
    a.iter()
        .map(|p| {
            let x = p[0].as_f64().ok_or(format!("{what} must be numbers"))?;
            let y = p[1].as_f64().ok_or(format!("{what} must be numbers"))?;
            Ok([x, y])
        })
        .collect()
}

const FIT_OPS: [&str; 4] = ["d_hole", "pocket", "usb_c_cutout", "hole"];

/// Reads Muse's part, fills the fit features from the measured dimensions, and derives
/// the checks. `dims` holds values like shaft_diameter_mm.
pub fn parse(part: &Value, dims: &BTreeMap<String, f64>) -> Result<Part, String> {
    let list = part["nodes"]
        .as_array()
        .ok_or("part.nodes must be a list")?;
    if list.is_empty() || list.len() > MAX_NODES {
        return Err(format!("part.nodes needs 1 to {MAX_NODES} nodes"));
    }
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut nodes = Vec::new();
    let mut crit = Vec::new();
    let mut ifaces = Vec::new();
    let mut fitted = Vec::new();
    let mut ops = Vec::new();
    for (i, n) in list.iter().enumerate() {
        let id = n["id"]
            .as_str()
            .ok_or(format!("node {} needs an id", i + 1))?
            .to_string();
        let op = n["op"].as_str().ok_or(format!("node {id} needs an op"))?;
        let at = if n["at"].is_null() {
            [0.0; 3]
        } else {
            v3(&n["at"], &format!("{id}.at"))?
        };
        let refs = |key: &str| -> Result<Vec<usize>, String> {
            let a = n[key]
                .as_array()
                .ok_or(format!("{id}.{key} must list earlier node ids"))?;
            a.iter()
                .map(|r| {
                    let r = r.as_str().unwrap_or("");
                    ids.get(r)
                        .copied()
                        .ok_or(format!("{id}.{key}: {r} is not an earlier node"))
                })
                .collect()
        };
        let one = |key: &str| -> Result<usize, String> {
            let r = n[key].as_str().unwrap_or("");
            ids.get(r)
                .copied()
                .ok_or(format!("{id}.{key}: {r} is not an earlier node"))
        };
        let frame = |default_u: Option<&str>| -> Result<Frame, String> {
            let nv = axis_vec(n["axis"].as_str().unwrap_or("")).ok_or(format!(
                "{id}.axis must be one of +x, -x, +y, -y, +z, -z (pointing out of the part through the opening)"
            ))?;
            let us =
                n["u"]
                    .as_str()
                    .or(default_u)
                    .unwrap_or(if nv[2] != 0.0 { "+x" } else { "+z" });
            let u = axis_vec(us).ok_or(format!("{id}.u must be an axis like +x"))?;
            if dot(u, nv).abs() > 1e-9 {
                return Err(format!("{id}.u must be at right angles to {id}.axis"));
            }
            Ok(Frame::new(at, nv, u))
        };
        let node = match op {
            "box" => {
                let s = v3(&n["size"], &format!("{id}.size"))?;
                if s.iter().any(|v| *v <= 0.0) {
                    return Err(format!("{id}.size must be positive"));
                }
                let r = opt(
                    &n["r"],
                    0.0,
                    &format!("{id}.r"),
                    0.0,
                    s[0].min(s[1]).min(s[2]) / 2.0,
                )?;
                Node::Box {
                    c: add(at, [0.0, 0.0, s[2] / 2.0]),
                    half: mul(s, 0.5),
                    r,
                }
            }
            "cylinder" => {
                let d = pos(&n["d"], &format!("{id}.d"), 0.2, 2.0 * LIMIT_MM)?;
                let d2 = opt(&n["d2"], d, &format!("{id}.d2"), 0.0, 2.0 * LIMIT_MM)?;
                let h = pos(&n["h"], &format!("{id}.h"), 0.1, LIMIT_MM)?;
                Node::Cylinder {
                    at,
                    r1: d / 2.0,
                    r2: d2 / 2.0,
                    h,
                }
            }
            "sphere" => Node::Sphere {
                c: at,
                r: pos(&n["d"], &format!("{id}.d"), 0.2, 2.0 * LIMIT_MM)? / 2.0,
            },
            "extrude" => Node::Extrude {
                at,
                pts: pts2(&n["points"], &format!("{id}.points"))?,
                h: pos(&n["h"], &format!("{id}.h"), 0.1, LIMIT_MM)?,
            },
            "revolve" => {
                let pts = pts2(&n["points"], &format!("{id}.points"))?;
                if pts.iter().any(|p| p[0] < 0.0) {
                    return Err(format!(
                        "{id}.points: the radius (first value) must not be negative"
                    ));
                }
                Node::Revolve { at, pts }
            }
            "union" => Node::Union {
                of: refs("of")?,
                k: opt(&n["smooth"], 0.0, &format!("{id}.smooth"), 0.0, 5.0)?,
            },
            "difference" => Node::Difference {
                of: refs("of")?,
                k: opt(&n["smooth"], 0.0, &format!("{id}.smooth"), 0.0, 5.0)?,
            },
            "intersection" => Node::Intersection { of: refs("of")? },
            "translate" => Node::Translate {
                by: v3(&n["by"], &format!("{id}.by"))?,
                node: one("node")?,
            },
            "rotate" => Node::Rotate {
                m: rotation(v3(&n["deg"], &format!("{id}.deg"))?),
                node: one("node")?,
            },
            "mirror" => {
                let a = match n["axis"].as_str() {
                    Some("x") | Some("+x") => 0,
                    Some("y") | Some("+y") => 1,
                    Some("z") | Some("+z") => 2,
                    _ => return Err(format!("{id}.axis must be x, y or z")),
                };
                Node::Mirror {
                    axis: a,
                    node: one("node")?,
                }
            }
            "repeat_polar" => Node::Polar {
                count: pos(&n["count"], &format!("{id}.count"), 2.0, 180.0)? as usize,
                node: one("node")?,
            },
            "repeat_linear" => Node::Linear {
                count: pos(&n["count"], &format!("{id}.count"), 2.0, 50.0)? as usize,
                step: v3(&n["step"], &format!("{id}.step"))?,
                node: one("node")?,
            },
            "d_hole" => {
                let f = frame(None)?;
                let (Some(d), Some(fl)) =
                    (dims.get("shaft_diameter_mm"), dims.get("flat_to_round_mm"))
                else {
                    return Err(format!("{id}: record shaft_diameter_mm and flat_to_round_mm first; a d_hole takes its size from them"));
                };
                let c = opt(&n["clearance"], 0.2, &format!("{id}.clearance"), 0.0, 1.0)?;
                let mut depth = pos(&n["depth"], &format!("{id}.depth"), 1.0, LIMIT_MM)?;
                if let Some(l) = dims.get("shaft_length_mm") {
                    depth = depth.min(l - 1.0).max(1.0);
                }
                let a = d / 2.0 + c / 2.0;
                let flat = fl + c - a;
                if flat <= 0.0 || flat >= a {
                    return Err(format!("{id}: the measured flat does not make a D shape"));
                }
                let mid = sub(f.at, mul(f.n, depth / 2.0));
                let r2 = |v: f64| (v * 100.0).round() / 100.0;
                let p = |q: V3| json!([r2(q[0]), r2(q[1]), r2(q[2])]);
                crit.push(json!({"name": format!("{id}_round"), "feature": "D hole, round diameter", "nominal_mm": r2(2.0 * a),
                    "locator": {"kind": "distance", "point_mm": p(sub(mid, mul(f.w, a))), "to_point_mm": p(add(mid, mul(f.w, a)))}}));
                crit.push(json!({"name": format!("{id}_flat"), "feature": "D hole, flat to round", "nominal_mm": r2(a + flat),
                    "locator": {"kind": "distance", "point_mm": p(sub(mid, mul(f.u, a))), "to_point_mm": p(add(mid, mul(f.u, flat)))}}));
                fitted.push(json!({"node": id, "diameter_mm": r2(2.0 * a), "flat_to_round_mm": r2(a + flat), "depth_mm": r2(depth), "clearance_mm": c}));
                Node::DHole { f, a, flat, depth }
            }
            "hole" => {
                let f = frame(None)?;
                Node::Hole {
                    f,
                    r: pos(&n["d"], &format!("{id}.d"), 0.4, 2.0 * LIMIT_MM)? / 2.0,
                    depth: pos(&n["depth"], &format!("{id}.depth"), 0.2, LIMIT_MM)?,
                }
            }
            "pocket" => {
                let f = frame(Some("+x"))?;
                let (Some(l), Some(w)) = (dims.get("board_length_mm"), dims.get("board_width_mm"))
                else {
                    return Err(format!("{id}: record board_length_mm and board_width_mm first; a pocket takes its size from them"));
                };
                let c = opt(&n["clearance"], 0.3, &format!("{id}.clearance"), 0.1, 2.0)?;
                let depth = pos(&n["depth"], &format!("{id}.depth"), 0.5, LIMIT_MM)?;
                let (hu, hw) = (l / 2.0 + c, w / 2.0 + c);
                let mid = sub(f.at, mul(f.n, depth / 2.0));
                let r2 = |v: f64| (v * 100.0).round() / 100.0;
                let p = |q: V3| json!([r2(q[0]), r2(q[1]), r2(q[2])]);
                crit.push(json!({"name": format!("{id}_length"), "feature": "pocket, inner length", "nominal_mm": r2(2.0 * hu),
                    "locator": {"kind": "distance", "point_mm": p(sub(mid, mul(f.u, hu))), "to_point_mm": p(add(mid, mul(f.u, hu)))}}));
                crit.push(json!({"name": format!("{id}_width"), "feature": "pocket, inner width", "nominal_mm": r2(2.0 * hw),
                    "locator": {"kind": "distance", "point_mm": p(sub(mid, mul(f.w, hw))), "to_point_mm": p(add(mid, mul(f.w, hw)))}}));
                fitted.push(json!({"node": id, "length_mm": r2(2.0 * hu), "width_mm": r2(2.0 * hw), "depth_mm": depth, "clearance_mm": c}));
                Node::Pocket { f, hu, hw, depth }
            }
            "usb_c_cutout" => {
                let f = frame(None)?;
                if f.n[2] != 0.0 {
                    return Err(format!("{id}.axis must be horizontal (+x, -x, +y or -y)"));
                }
                let size = if n["size"].is_null() {
                    [12.5, 7.0]
                } else {
                    let a = n["size"]
                        .as_array()
                        .filter(|a| a.len() == 2)
                        .ok_or(format!("{id}.size must be [width, height]"))?;
                    [
                        pos(&a[0], &format!("{id}.size[0]"), 9.0, 20.0)?,
                        pos(&a[1], &format!("{id}.size[1]"), 3.2, 12.0)?,
                    ]
                };
                let depth = opt(&n["depth"], 4.0, &format!("{id}.depth"), 0.5, 30.0)?;
                let inset = pos(
                    &n["inset"],
                    &format!("{id}.inset (outer surface to the USB C socket face)"),
                    0.0,
                    30.0,
                )?;
                let pt = sub(f.at, mul(f.n, inset));
                let r2 = |v: f64| (v * 100.0).round() / 100.0;
                ifaces.push(json!({"id": "usb_c_receptacle", "point_mm": [r2(pt[0]), r2(pt[1]), r2(pt[2])], "axis": f.n}));
                Node::Slot {
                    f,
                    hu: size[0] / 2.0,
                    hw: size[1] / 2.0,
                    depth,
                }
            }
            other => return Err(format!("{id}.op {other} is not known")),
        };
        if ids.insert(id.clone(), nodes.len()).is_some() {
            return Err(format!("node id {id} is used twice"));
        }
        nodes.push(node);
        ops.push(op.to_string());
    }
    let root = match part["root"].as_str() {
        Some(r) => *ids.get(r).ok_or(format!("root {r} is not a node"))?,
        None => nodes.len() - 1,
    };
    // Fit features are cutters in world coordinates: only as the 2nd and later parts of a
    // difference, and never under a move, so their checks land where they are.
    let mut moved = vec![false; nodes.len()];
    let mut cutter = vec![false; nodes.len()];
    let mut used = vec![false; nodes.len()];
    let mut stack = vec![(root, false)];
    while let Some((i, under)) = stack.pop() {
        used[i] = true;
        moved[i] |= under;
        match &nodes[i] {
            Node::Union { of, .. } | Node::Intersection { of } => {
                stack.extend(of.iter().map(|&c| (c, under)))
            }
            Node::Difference { of, .. } => {
                for (k, &c) in of.iter().enumerate() {
                    if k > 0 {
                        cutter[c] = true;
                    }
                    stack.push((c, under));
                }
            }
            Node::Translate { node, .. }
            | Node::Rotate { node, .. }
            | Node::Mirror { node, .. }
            | Node::Polar { node, .. }
            | Node::Linear { node, .. } => stack.push((*node, true)),
            _ => {}
        }
    }
    for (i, op) in ops.iter().enumerate() {
        if FIT_OPS.contains(&op.as_str()) && used[i] {
            if moved[i] && op != "hole" {
                return Err(format!(
                    "{op} {} sits under a move or repeat; place it with at and axis instead",
                    list[i]["id"]
                ));
            }
            if !cutter[i] {
                return Err(format!(
                    "{op} {} must be cut away: put it after the body in a difference",
                    list[i]["id"]
                ));
            }
        }
    }
    Ok(Part {
        nodes,
        root,
        critical_dimensions: crit,
        interfaces: ifaces,
        fitted,
    })
}

impl Part {
    fn eval(&self, i: usize, p: V3) -> f64 {
        match &self.nodes[i] {
            Node::Box { c, half, r } => {
                let q = [
                    (p[0] - c[0]).abs() - half[0] + r,
                    (p[1] - c[1]).abs() - half[1] + r,
                    (p[2] - c[2]).abs() - half[2] + r,
                ];
                let o = [q[0].max(0.0), q[1].max(0.0), q[2].max(0.0)];
                dot(o, o).sqrt() + q[0].max(q[1]).max(q[2]).min(0.0) - r
            }
            Node::Cylinder { at, r1, r2, h } => {
                let z = p[2] - at[2];
                let rr = r1 + (r2 - r1) * (z / h).clamp(0.0, 1.0);
                let slope = ((r2 - r1) / h).atan().cos();
                prism(
                    (len2(p[0] - at[0], p[1] - at[1]) - rr) * slope,
                    (z - h / 2.0).abs() - h / 2.0,
                )
            }
            Node::Sphere { c, r } => dot(sub(p, *c), sub(p, *c)).sqrt() - r,
            Node::Extrude { at, pts, h } => {
                let z = p[2] - at[2];
                prism(
                    polygon([p[0] - at[0], p[1] - at[1]], pts),
                    (z - h / 2.0).abs() - h / 2.0,
                )
            }
            Node::Revolve { at, pts } => {
                polygon([len2(p[0] - at[0], p[1] - at[1]), p[2] - at[2]], pts)
            }
            Node::Union { of, k } => of
                .iter()
                .map(|&c| self.eval(c, p))
                .fold(f64::MAX, |a, b| smin(a, b, *k)),
            Node::Difference { of, k } => {
                let mut d = self.eval(of[0], p);
                for &c in &of[1..] {
                    d = -smin(-d, self.eval(c, p), *k);
                }
                d
            }
            Node::Intersection { of } => {
                of.iter().map(|&c| self.eval(c, p)).fold(f64::MIN, f64::max)
            }
            Node::Translate { by, node } => self.eval(*node, sub(p, *by)),
            Node::Rotate { m, node } => self.eval(*node, apply(&transpose(m), p)),
            Node::Mirror { axis, node } => {
                let mut q = p;
                q[*axis] = -q[*axis];
                self.eval(*node, q)
            }
            Node::Polar { count, node } => {
                let seg = std::f64::consts::TAU / *count as f64;
                let a = p[1].atan2(p[0]);
                let k = (a / seg).round();
                let (s, c) = (-k * seg).sin_cos();
                self.eval(*node, [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]])
            }
            Node::Linear { count, step, node } => (0..*count)
                .map(|i| self.eval(*node, sub(p, mul(*step, i as f64))))
                .fold(f64::MAX, f64::min),
            Node::DHole { f, a, flat, depth } => {
                let (x, y, t) = f.local(p);
                let cap = (-(t + 1.0)).max(t - depth);
                prism((len2(x, y) - a).max(x - flat), cap)
            }
            Node::Hole { f, r, depth } => {
                let (x, y, t) = f.local(p);
                prism(len2(x, y) - r, (-(t + 1.0)).max(t - depth))
            }
            Node::Pocket { f, hu, hw, depth } => {
                let (x, y, t) = f.local(p);
                let q = [x.abs() - hu, y.abs() - hw];
                let d2 = len2(q[0].max(0.0), q[1].max(0.0)) + q[0].max(q[1]).min(0.0);
                prism(d2, (-(t + 1.0)).max(t - depth))
            }
            Node::Slot { f, hu, hw, depth } => {
                let (x, y, t) = f.local(p);
                // A stadium: straight along u, round ends of radius hw.
                let d2 = len2((x.abs() - (hu - hw)).max(0.0), y) - hw;
                prism(d2, (-(t + 1.0)).max(t - depth))
            }
        }
    }

    pub fn distance(&self, p: V3) -> f64 {
        self.eval(self.root, p)
    }

    /// Axis aligned bounds of node i (min, max).
    fn bounds(&self, i: usize) -> (V3, V3) {
        let corners = |lo: V3, hi: V3| -> Vec<V3> {
            (0..8)
                .map(|k| {
                    [
                        if k & 1 == 0 { lo[0] } else { hi[0] },
                        if k & 2 == 0 { lo[1] } else { hi[1] },
                        if k & 4 == 0 { lo[2] } else { hi[2] },
                    ]
                })
                .collect()
        };
        let span = |pts: Vec<V3>| -> (V3, V3) {
            let mut lo = [f64::MAX; 3];
            let mut hi = [f64::MIN; 3];
            for p in pts {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            (lo, hi)
        };
        let feature = |f: &Frame, ru: f64, rw: f64, depth: f64| -> (V3, V3) {
            let mut pts = Vec::new();
            for su in [-1.0, 1.0] {
                for sw in [-1.0, 1.0] {
                    for t in [-1.0, depth] {
                        pts.push(add(
                            add(add(f.at, mul(f.u, su * ru)), mul(f.w, sw * rw)),
                            mul(f.n, -t),
                        ));
                    }
                }
            }
            span(pts)
        };
        match &self.nodes[i] {
            Node::Box { c, half, .. } => (sub(*c, *half), add(*c, *half)),
            Node::Cylinder { at, r1, r2, h } => {
                let r = r1.max(*r2);
                (
                    [at[0] - r, at[1] - r, at[2]],
                    [at[0] + r, at[1] + r, at[2] + h],
                )
            }
            Node::Sphere { c, r } => (sub(*c, [*r; 3]), add(*c, [*r; 3])),
            Node::Extrude { at, pts, h } => {
                let (lo, hi) = span(pts.iter().map(|q| [q[0], q[1], 0.0]).collect());
                (add(*at, lo), add(*at, [hi[0], hi[1], *h]))
            }
            Node::Revolve { at, pts } => {
                let r = pts.iter().map(|q| q[0]).fold(0.0, f64::max);
                let z0 = pts.iter().map(|q| q[1]).fold(f64::MAX, f64::min);
                let z1 = pts.iter().map(|q| q[1]).fold(f64::MIN, f64::max);
                (add(*at, [-r, -r, z0]), add(*at, [r, r, z1]))
            }
            Node::Union { of, .. } => {
                let bs: Vec<(V3, V3)> = of.iter().map(|&c| self.bounds(c)).collect();
                span(bs.iter().flat_map(|b| [b.0, b.1]).collect())
            }
            Node::Difference { of, .. } => self.bounds(of[0]),
            Node::Intersection { of } => {
                let mut lo = [f64::MIN; 3];
                let mut hi = [f64::MAX; 3];
                for &c in of {
                    let b = self.bounds(c);
                    for k in 0..3 {
                        lo[k] = lo[k].max(b.0[k]);
                        hi[k] = hi[k].min(b.1[k]);
                    }
                }
                (lo, hi)
            }
            Node::Translate { by, node } => {
                let b = self.bounds(*node);
                (add(b.0, *by), add(b.1, *by))
            }
            Node::Rotate { m, node } => {
                let b = self.bounds(*node);
                span(corners(b.0, b.1).into_iter().map(|q| apply(m, q)).collect())
            }
            Node::Mirror { axis, node } => {
                let (mut lo, mut hi) = self.bounds(*node);
                let (a, b) = (-hi[*axis], -lo[*axis]);
                lo[*axis] = a;
                hi[*axis] = b;
                (lo, hi)
            }
            Node::Polar { node, .. } => {
                let b = self.bounds(*node);
                let r = corners(b.0, b.1)
                    .iter()
                    .map(|q| len2(q[0], q[1]))
                    .fold(0.0, f64::max);
                ([-r, -r, b.0[2]], [r, r, b.1[2]])
            }
            Node::Linear { count, step, node } => {
                let b = self.bounds(*node);
                let e = mul(*step, (*count - 1) as f64);
                span(vec![b.0, b.1, add(b.0, e), add(b.1, e)])
            }
            Node::DHole { f, a, depth, .. } => feature(f, *a, *a, *depth),
            Node::Hole { f, r, depth } => feature(f, *r, *r, *depth),
            Node::Pocket { f, hu, hw, depth } => feature(f, *hu, *hw, *depth),
            Node::Slot { f, hu, hw, depth } => feature(f, *hu, *hw, *depth),
        }
    }

    /// Meshes at `step` mm, and once more on a coarser grid when the mesh has far more
    /// than `target` triangles, so checkers and phones handle it. Flat and round faces
    /// stay on the true surface; only sharp edges round off by about the grid size.
    pub fn mesh_for(&self, step: f64, target: usize) -> Result<Mesh, String> {
        let m = self.mesh(step)?;
        if m.triangles.len() <= target * 5 / 4 {
            return Ok(m);
        }
        let k = (m.triangles.len() as f64 / target as f64).sqrt();
        self.mesh(step * k * 1.03)
    }

    /// Meshes the part with marching tetrahedra on a grid of about `step` mm.
    /// The cube split is the same everywhere, so neighbouring cells share their faces and
    /// the mesh is closed.
    pub fn mesh(&self, step: f64) -> Result<Mesh, String> {
        let (lo, hi) = self.bounds(self.root);
        let size = sub(hi, lo);
        if size.iter().any(|v| !v.is_finite() || *v <= 0.5) {
            return Err("The part is empty or smaller than half a millimetre.".into());
        }
        if size.iter().any(|v| *v > LIMIT_MM) {
            return Err(format!("The part is larger than {LIMIT_MM} mm."));
        }
        // Keep the grid under about 4 million points.
        let vol = (size[0] + 2.0) * (size[1] + 2.0) * (size[2] + 2.0);
        let h = step.max((vol / 4.0e6).cbrt());
        let o = sub(lo, [h * 1.5; 3]);
        let n: Vec<usize> = (0..3)
            .map(|k| ((size[k] + 3.0 * h) / h).ceil() as usize + 1)
            .collect();
        let idx = |i: usize, j: usize, k: usize| (k * n[1] + j) * n[0] + i;
        let mut val = vec![0f32; n[0] * n[1] * n[2]];
        for k in 0..n[2] {
            for j in 0..n[1] {
                for i in 0..n[0] {
                    let d = self.distance([
                        o[0] + i as f64 * h,
                        o[1] + j as f64 * h,
                        o[2] + k as f64 * h,
                    ]);
                    // Exactly zero would make a corner both inside and outside.
                    val[idx(i, j, k)] = if d == 0.0 { 1e-7 } else { d as f32 };
                }
            }
        }
        let point = |g: usize| -> V3 {
            let i = g % n[0];
            let j = (g / n[0]) % n[1];
            let k = g / (n[0] * n[1]);
            [
                o[0] + i as f64 * h,
                o[1] + j as f64 * h,
                o[2] + k as f64 * h,
            ]
        };
        const TETS: [[usize; 4]; 6] = [
            [0, 1, 3, 7],
            [0, 3, 2, 7],
            [0, 2, 6, 7],
            [0, 6, 4, 7],
            [0, 4, 5, 7],
            [0, 5, 1, 7],
        ];
        let mut mesh = Mesh::default();
        let mut edge_vertex: HashMap<(usize, usize), u32> = HashMap::new();
        let mut vertex = |a: usize, b: usize, mesh: &mut Mesh| -> u32 {
            let key = (a.min(b), a.max(b));
            if let Some(&v) = edge_vertex.get(&key) {
                return v;
            }
            let (va, vb) = (val[key.0] as f64, val[key.1] as f64);
            let t = (va / (va - vb)).clamp(0.02, 0.98);
            let (pa, pb) = (point(key.0), point(key.1));
            mesh.vertices.push(add(pa, mul(sub(pb, pa), t)));
            let v = (mesh.vertices.len() - 1) as u32;
            edge_vertex.insert(key, v);
            v
        };
        for k in 0..n[2] - 1 {
            for j in 0..n[1] - 1 {
                for i in 0..n[0] - 1 {
                    let c: [usize; 8] = std::array::from_fn(|q| {
                        idx(i + (q & 1), j + ((q >> 1) & 1), k + ((q >> 2) & 1))
                    });
                    let inside: Vec<bool> = c.iter().map(|&g| val[g] < 0.0).collect();
                    if inside.iter().all(|&b| b) || inside.iter().all(|&b| !b) {
                        continue;
                    }
                    for t in TETS {
                        let g = t.map(|q| c[q]);
                        let ins: Vec<usize> = (0..4).filter(|&q| inside[t[q]]).collect();
                        let outs: Vec<usize> = (0..4).filter(|&q| !inside[t[q]]).collect();
                        let mut tris: Vec<[u32; 3]> = Vec::new();
                        match ins.len() {
                            1 | 3 => {
                                let (a, others) = if ins.len() == 1 {
                                    (ins[0], &outs)
                                } else {
                                    (outs[0], &ins)
                                };
                                tris.push([
                                    vertex(g[a], g[others[0]], &mut mesh),
                                    vertex(g[a], g[others[1]], &mut mesh),
                                    vertex(g[a], g[others[2]], &mut mesh),
                                ]);
                            }
                            2 => {
                                let (a, b) = (ins[0], ins[1]);
                                let (c0, d0) = (outs[0], outs[1]);
                                let p = vertex(g[a], g[c0], &mut mesh);
                                let q = vertex(g[a], g[d0], &mut mesh);
                                let r = vertex(g[b], g[d0], &mut mesh);
                                let s = vertex(g[b], g[c0], &mut mesh);
                                tris.push([p, q, r]);
                                tris.push([p, r, s]);
                            }
                            _ => {}
                        }
                        // Face outwards: from the inside corners towards the outside ones.
                        let cin = ins.iter().fold([0.0; 3], |acc, &q| add(acc, point(g[q])));
                        let cout = outs.iter().fold([0.0; 3], |acc, &q| add(acc, point(g[q])));
                        let dir = sub(
                            mul(cout, 1.0 / outs.len() as f64),
                            mul(cin, 1.0 / ins.len() as f64),
                        );
                        for mut tr in tris {
                            let [a, b, c] = tr.map(|v| mesh.vertices[v as usize]);
                            if dot(cross(sub(b, a), sub(c, a)), dir) < 0.0 {
                                tr.swap(1, 2);
                            }
                            mesh.triangles.push(tr);
                        }
                    }
                }
            }
        }
        if mesh.triangles.is_empty() {
            return Err(
                "The part has no solid: check the differences do not cut everything away.".into(),
            );
        }
        Ok(mesh)
    }
}

/// The node schema for Muse. Recursive schemas are not allowed, so a part is a flat list
/// of nodes that refer to earlier nodes by id.
pub fn schema() -> Value {
    // Numbers may be expression strings ("$wall * 2") in an assembly; Rust resolves them.
    let n3 = json!({"type": "array", "items": {"type": ["number", "string"]}, "minItems": 3, "maxItems": 3});
    let pts = json!({"type": "array", "items": {"type": "array", "items": {"type": ["number", "string"]}, "minItems": 2, "maxItems": 2}});
    let ids = json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "properties": {
            "nodes": {"type": "array", "maxItems": MAX_NODES, "items": {"type": "object", "properties": {
                "id": {"type": "string"},
                "op": {"type": "string", "enum": ["box", "cylinder", "sphere", "extrude", "revolve", "union", "difference",
                    "intersection", "translate", "rotate", "mirror", "repeat_polar", "repeat_linear", "d_hole", "hole", "pocket", "usb_c_cutout"]},
                "at": n3.clone(), "size": {"type": "array", "items": {"type": ["number", "string"]}},
                "d": {"type": ["number", "string"]}, "d2": {"type": ["number", "string"]}, "h": {"type": ["number", "string"]}, "r": {"type": ["number", "string"]},
                "points": pts, "of": ids, "node": {"type": "string"}, "smooth": {"type": ["number", "string"]},
                "by": n3.clone(), "deg": n3.clone(), "step": n3,
                "axis": {"type": "string"}, "u": {"type": "string"}, "count": {"type": ["integer", "string"]},
                "depth": {"type": ["number", "string"]}, "clearance": {"type": ["number", "string"]}, "inset": {"type": ["number", "string"]}
            }, "required": ["id", "op"]}},
            "root": {"type": "string"}
        },
        "required": ["nodes"]
    })
}

/// The language, for the agent's instructions.
pub const GUIDE: &str = "CAD language (millimetres, z up, the part sits on z = 0 when printed). A part is {nodes: [...], root?: id}; each node has id and op and may only use earlier ids. \
Shapes: box {size [x,y,z], r edge radius, at}: centred on x and y, bottom at at.z. cylinder {d, h, d2 top diameter for a cone, at}: along z from at.z up. sphere {d, at centre}. \
extrude {points [[x,y]...] polygon, h, at}: from at.z up. revolve {points [[r,z]...] profile polygon with r >= 0, at}: around the z axis. \
Combine: union {of [ids], smooth k}, difference {of [body, cutter, ...], smooth k}, intersection {of}. Move: translate {by, node}, rotate {deg [x,y,z], node}, mirror {axis x|y|z, node}, \
repeat_polar {count, node} around the z axis, repeat_linear {count, step [x,y,z], node}. \
Fit features (cutters, used as later parts of a difference, placed with at and axis, never under a move): \
d_hole {at: centre of the opening, axis: direction out of the part through the opening, u: direction the D flat faces, depth, clearance 0.1 to 0.3}: its diameter and flat come from the recorded shaft. \
pocket {at: centre of the opening, axis, u: direction of the board length, depth, clearance}: its length and width come from the recorded board. \
usb_c_cutout {at: centre of the opening on the outer wall, axis: outwards (horizontal), u: horizontal along the wall, size [w,h] default 12.5 x 7, depth: wall to cut, inset: distance from the outer surface in to the USB C socket face}: Any Part Made checks the plug fits. The plug reaches only about 1 mm past the outer surface, so the board's USB C socket must stick into the wall opening with its face 0 to 1 mm behind the outer surface (inset 0 to 1): end the board pocket at the inner wall face and let the socket reach into the opening. The wall around the opening stays 2 mm thick. The opening may open into the board pocket only, never into another cavity. \
hole {d, at, axis, depth}: a plain round hole. \
Walls at least 2 mm, and at least 2 mm of solid between any two cavities or cutters; never let two cutters touch or nearly touch, that leaves paper thin skins. Keep the part in one piece.";

#[cfg(test)]
mod tests {
    use super::*;

    fn dims() -> BTreeMap<String, f64> {
        BTreeMap::from([
            ("shaft_diameter_mm".to_string(), 6.0),
            ("flat_to_round_mm".to_string(), 4.6),
            ("board_length_mm".to_string(), 21.0),
            ("board_width_mm".to_string(), 17.8),
        ])
    }

    fn knob() -> Value {
        json!({"nodes": [
            {"id": "body", "op": "cylinder", "d": 36, "h": 20},
            {"id": "groove", "op": "cylinder", "d": 2, "h": 30, "at": [18.4, 0, -5]},
            {"id": "grooves", "op": "repeat_polar", "count": 18, "node": "groove"},
            {"id": "bore", "op": "d_hole", "at": [0, 0, 0], "axis": "-z", "u": "+x", "depth": 12, "clearance": 0.2},
            {"id": "knob", "op": "difference", "of": ["body", "grooves", "bore"], "smooth": 0.4}
        ]})
    }

    #[test]
    fn knob_is_closed_and_the_d_hole_is_measured() {
        let p = parse(&knob(), &dims()).unwrap();
        assert_eq!(p.critical_dimensions.len(), 2);
        assert_eq!(p.critical_dimensions[0]["nominal_mm"], 6.2);
        assert_eq!(p.critical_dimensions[1]["nominal_mm"], 4.8);
        // Inside the round part of the bore, inside the flat, and in the body.
        assert!(p.distance([-2.9, 0.0, 6.0]) > 0.0);
        // The flat sits 4.8 - 3.1 = 1.7 mm from the axis.
        assert!(p.distance([1.6, 0.0, 6.0]) > 0.0 && p.distance([1.8, 0.0, 6.0]) < 0.0);
        assert!(p.distance([10.0, 0.0, 10.0]) < 0.0);
        let m = p.mesh(0.4).unwrap();
        assert!(m.is_watertight(), "not closed");
        let small = p.mesh_for(0.3, 40_000).unwrap();
        assert!(
            small.is_watertight() && small.triangles.len() < 52_000,
            "{}",
            small.triangles.len()
        );
        let v = m.volume();
        let solid = std::f64::consts::PI * 18.0 * 18.0 * 20.0;
        assert!(v > 0.8 * solid && v < solid, "{v}");
    }

    #[test]
    fn enclosure_is_closed_and_declares_the_usb_c_check() {
        let part = json!({"nodes": [
            {"id": "shell", "op": "box", "size": [27, 24, 10], "r": 1.5},
            {"id": "seat", "op": "pocket", "at": [0, 0, 10], "axis": "+z", "u": "+x", "depth": 8, "clearance": 0.3},
            {"id": "usb", "op": "usb_c_cutout", "at": [13.5, 0, 4.5], "axis": "+x", "u": "+y", "inset": 3},
            {"id": "case", "op": "difference", "of": ["shell", "seat", "usb"]}
        ]});
        let p = parse(&part, &dims()).unwrap();
        assert_eq!(p.interfaces[0]["id"], "usb_c_receptacle");
        assert_eq!(p.interfaces[0]["point_mm"], json!([10.5, 0.0, 4.5]));
        assert_eq!(p.critical_dimensions[0]["nominal_mm"], 21.6);
        let m = p.mesh(0.4).unwrap();
        assert!(m.is_watertight());
        assert!(m.volume() > 0.0);
    }

    #[test]
    fn fit_features_must_be_cutters_in_place_and_measured() {
        let mut d = dims();
        let bad_move = json!({"nodes": [
            {"id": "b", "op": "cylinder", "d": 30, "h": 20},
            {"id": "h", "op": "d_hole", "at": [0, 0, 0], "axis": "-z", "depth": 10},
            {"id": "m", "op": "translate", "by": [1, 0, 0], "node": "h"},
            {"id": "k", "op": "difference", "of": ["b", "m"]}
        ]});
        assert!(parse(&bad_move, &d).unwrap_err().contains("under a move"));
        let not_cut = json!({"nodes": [
            {"id": "h", "op": "d_hole", "at": [0, 0, 0], "axis": "-z", "depth": 10},
            {"id": "b", "op": "cylinder", "d": 30, "h": 20},
            {"id": "k", "op": "union", "of": ["b", "h"]}
        ]});
        assert!(parse(&not_cut, &d).unwrap_err().contains("cut away"));
        let forward = json!({"nodes": [{"id": "k", "op": "union", "of": ["b"]}, {"id": "b", "op": "sphere", "d": 10}]});
        assert!(parse(&forward, &d)
            .unwrap_err()
            .contains("not an earlier node"));
        d.remove("shaft_diameter_mm");
        assert!(parse(&knob(), &d)
            .unwrap_err()
            .contains("record shaft_diameter_mm"));
    }

    #[test]
    fn transforms_and_profiles_mesh_closed() {
        let part = json!({"nodes": [
            {"id": "cap", "op": "revolve", "points": [[0, 0], [15, 0], [15, 8], [12, 12], [0, 12]]},
            {"id": "rib", "op": "box", "size": [2, 4, 6], "at": [15, 0, 1]},
            {"id": "ribs", "op": "repeat_polar", "count": 8, "node": "rib"},
            {"id": "tab", "op": "extrude", "points": [[0, 0], [10, 0], [10, 3], [0, 3]], "h": 2},
            {"id": "tab2", "op": "rotate", "deg": [0, 0, 45], "node": "tab"},
            {"id": "all", "op": "union", "of": ["cap", "ribs", "tab2"], "smooth": 0.5}
        ]});
        let p = parse(&part, &BTreeMap::new()).unwrap();
        let m = p.mesh(0.5).unwrap();
        assert!(m.is_watertight());
        assert!(m.volume() > 0.0);
    }
}
