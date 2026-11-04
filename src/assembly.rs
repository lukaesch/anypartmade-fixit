//! A complete solution: several parts that fit the measured object and each other.
//! Muse writes a design with shared parameters (numbers may be expressions such as
//! "$board_length_mm + 2 * $wall"), parts in the CAD language, where each part sits
//! in the assembly, how parts slide together, and ghosts of the measured objects for
//! the hologram. Rust resolves the expressions, parses each part and derives the
//! transforms Any Part Made needs to check parts against each other (mates).

use crate::cad::{self, Part};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const MAX_PARTS: usize = 6;

/// Keys whose string values are names, not expressions.
const TEXT_KEYS: [&str; 12] = [
    "id", "op", "axis", "u", "node", "of", "root", "name", "material", "with", "kind", "label",
];

// ---------- Expressions: numbers, $names, + - * /, parentheses, min, max ----------

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
    vars: &'a BTreeMap<String, f64>,
}

impl Lexer<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        while let Some(c @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term()?;
            v = if c == b'+' { v + r } else { v - r };
        }
        Ok(v)
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.factor()?;
        while let Some(c @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let r = self.factor()?;
            if c == b'/' && r == 0.0 {
                return Err("division by zero".into());
            }
            v = if c == b'*' { v * r } else { v / r };
        }
        Ok(v)
    }
    fn factor(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(-self.factor()?)
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("a ) is missing".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(b'$') => {
                self.i += 1;
                let n = self.name();
                self.vars
                    .get(&n)
                    .copied()
                    .ok_or(format!("${n} is not a parameter or a recorded dimension"))
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let a = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.')
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[a..self.i])
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| "a number is malformed".into())
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let f = self.name();
                if self.peek() != Some(b'(') {
                    return Err(format!("{f} is not known; write parameters as ${f}"));
                }
                self.i += 1;
                let a = self.expr()?;
                if self.peek() != Some(b',') {
                    return Err(format!("{f} needs two values"));
                }
                self.i += 1;
                let b = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("a ) is missing".into());
                }
                self.i += 1;
                match f.as_str() {
                    "min" => Ok(a.min(b)),
                    "max" => Ok(a.max(b)),
                    _ => Err(format!("{f} is not known; use min or max")),
                }
            }
            _ => Err("a value is missing".into()),
        }
    }
    fn name(&mut self) -> String {
        let a = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_')
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[a..self.i]).into_owned()
    }
}

pub fn eval(text: &str, vars: &BTreeMap<String, f64>) -> Result<f64, String> {
    let mut l = Lexer {
        s: text.as_bytes(),
        i: 0,
        vars,
    };
    let v = l.expr().map_err(|e| format!("\"{text}\": {e}"))?;
    if l.peek().is_some() {
        return Err(format!("\"{text}\": unexpected text after the value"));
    }
    if !v.is_finite() {
        return Err(format!("\"{text}\" is not a finite number"));
    }
    Ok(v)
}

/// Replaces every expression string by its number, except under name keys.
fn resolve(v: &Value, key: &str, vars: &BTreeMap<String, f64>) -> Result<Value, String> {
    Ok(match v {
        Value::String(s) if !TEXT_KEYS.contains(&key) => json!(eval(s, vars)?),
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|x| resolve(x, key, vars))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(o) => {
            let mut m = Map::new();
            for (k, x) in o {
                m.insert(k.clone(), resolve(x, k, vars)?);
            }
            Value::Object(m)
        }
        _ => v.clone(),
    })
}

// ---------- Poses: where each part sits in the assembly ----------

pub type M4 = [[f64; 4]; 4];

fn mmul(a: &M4, b: &M4) -> M4 {
    let mut r = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            r[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

/// Rotate about x, then y, then z (degrees), then translate: row major.
pub fn pose(at: [f64; 3], deg: [f64; 3]) -> M4 {
    let (a, b, c) = (
        deg[0].to_radians(),
        deg[1].to_radians(),
        deg[2].to_radians(),
    );
    let rx = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, a.cos(), -a.sin(), 0.0],
        [0.0, a.sin(), a.cos(), 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let ry = [
        [b.cos(), 0.0, b.sin(), 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [-b.sin(), 0.0, b.cos(), 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let rz = [
        [c.cos(), -c.sin(), 0.0, 0.0],
        [c.sin(), c.cos(), 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut m = mmul(&rz, &mmul(&ry, &rx));
    m[0][3] = at[0];
    m[1][3] = at[1];
    m[2][3] = at[2];
    m
}

/// Inverse of a rigid pose (rotation plus translation).
#[allow(clippy::needless_range_loop)]
pub fn inverse(m: &M4) -> M4 {
    let mut r = [[0.0; 4]; 4];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = m[j][i];
        }
    }
    for i in 0..3 {
        r[i][3] = -(0..3).map(|k| r[i][k] * m[k][3]).sum::<f64>();
    }
    r[3][3] = 1.0;
    r
}

fn rotate(m: &M4, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

#[derive(Debug, Clone)]
pub struct Mate {
    /// Index of the earlier part this one fits onto.
    pub with: usize,
    /// Direction this part moves to come off, in assembly coordinates.
    pub insertion: [f64; 3],
    pub min_clearance_mm: f64,
}

#[derive(Debug)]
pub struct AssemblyPart {
    pub name: String,
    pub material: String,
    pub part: Part,
    /// Part coordinates (as printed, standing on z = 0) to assembly coordinates.
    pub place: M4,
    pub mates: Vec<Mate>,
    /// The part as resolved JSON (numbers only), for the mesher.
    pub nodes: Value,
}

#[derive(Debug)]
pub struct Assembly {
    pub parts: Vec<AssemblyPart>,
    /// Measured objects drawn as ghosts in the hologram, sizes resolved.
    pub ghosts: Vec<Value>,
    pub params: BTreeMap<String, f64>,
}

impl Assembly {
    /// For checking part `i` with mate `m`: where the mate part sits in part i's own
    /// coordinates (16 numbers, row major), and the direction it leaves along.
    pub fn mate_for_check(&self, i: usize, m: &Mate) -> (Vec<f64>, [f64; 3]) {
        let rel = mmul(&inverse(&self.parts[i].place), &self.parts[m.with].place);
        let dir = rotate(
            &inverse(&self.parts[i].place),
            [-m.insertion[0], -m.insertion[1], -m.insertion[2]],
        );
        (
            rel.iter()
                .flatten()
                .map(|v| (v * 1e4).round() / 1e4)
                .collect(),
            dir.map(|v| (v * 1e4).round() / 1e4),
        )
    }

    /// What the page needs to build and show it.
    pub fn view(&self) -> Value {
        json!({
            "parts": self.parts.iter().map(|p| json!({
                "name": p.name,
                "part": p.nodes,
                "place": p.place,
                "mates": p.mates.iter().map(|m| json!({"with": m.with, "insertion": m.insertion, "min_clearance_mm": m.min_clearance_mm})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "ghosts": self.ghosts,
        })
    }
}

fn v3(v: &Value, what: &str) -> Result<[f64; 3], String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or(format!("{what} must be [x, y, z]"))?;
    let mut o = [0.0; 3];
    for i in 0..3 {
        o[i] = a[i].as_f64().ok_or(format!("{what} must be numbers"))?;
    }
    Ok(o)
}

/// Reads a design: one part ({nodes}) or an assembly ({params, parts, ghosts}).
pub fn parse(design: &Value, dims: &BTreeMap<String, f64>) -> Result<Assembly, String> {
    // Parameters may use recorded dimensions and each other, in any order.
    let mut vars = dims.clone();
    let raw: Vec<(String, Value)> = design["params"]
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let mut left = raw;
    for _ in 0..=left.len() {
        let mut next = Vec::new();
        for (k, v) in left {
            let x = match &v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => eval(s, &vars).ok(),
                _ => return Err(format!("param {k} must be a number or an expression")),
            };
            match x {
                Some(x) => {
                    if dims.contains_key(&k) {
                        return Err(format!(
                            "param {k} would hide the recorded dimension of the same name"
                        ));
                    }
                    vars.insert(k, x);
                }
                None => next.push((k, v)),
            }
        }
        left = next;
        if left.is_empty() {
            break;
        }
    }
    if let Some((k, v)) = left.first() {
        let e = v
            .as_str()
            .map(|s| eval(s, &vars).unwrap_err())
            .unwrap_or_default();
        return Err(format!("param {k}: {e}"));
    }
    let list: Vec<Value> = match design["parts"].as_array() {
        Some(a) => a.clone(),
        None if design["nodes"].is_array() => {
            vec![json!({"name": "part", "nodes": design["nodes"], "root": design["root"]})]
        }
        None => return Err("a design needs parts: [{name, nodes, place?, mates?}]".into()),
    };
    if list.is_empty() || list.len() > MAX_PARTS {
        return Err(format!("a design has 1 to {MAX_PARTS} parts"));
    }
    let mut names: Vec<String> = Vec::new();
    let mut parts = Vec::new();
    for (i, p) in list.iter().enumerate() {
        let name = p["name"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("part{}", i + 1));
        if names.contains(&name) {
            return Err(format!("part name {name} is used twice"));
        }
        let resolved = resolve(p, "", &vars).map_err(|e| format!("part {name}: {e}"))?;
        let nodes = json!({"nodes": resolved["nodes"], "root": resolved["root"]});
        let part = cad::parse(&nodes, &vars).map_err(|e| format!("part {name}: {e}"))?;
        let place = if resolved["place"].is_null() {
            pose([0.0; 3], [0.0; 3])
        } else {
            let at = if resolved["place"]["at"].is_null() {
                [0.0; 3]
            } else {
                v3(&resolved["place"]["at"], &format!("{name}.place.at"))?
            };
            let deg = if resolved["place"]["deg"].is_null() {
                [0.0; 3]
            } else {
                v3(&resolved["place"]["deg"], &format!("{name}.place.deg"))?
            };
            pose(at, deg)
        };
        let mut mates = Vec::new();
        for m in resolved["mates"].as_array().into_iter().flatten() {
            let with = m["with"].as_str().unwrap_or("");
            let Some(w) = names.iter().position(|n| n == with) else {
                return Err(format!(
                    "part {name}: mates.with {with} must name an earlier part"
                ));
            };
            let ins = if m["insertion"].is_null() {
                [0.0, 0.0, 1.0]
            } else {
                v3(&m["insertion"], &format!("{name}.mates.insertion"))?
            };
            let l = (ins[0] * ins[0] + ins[1] * ins[1] + ins[2] * ins[2]).sqrt();
            if l < 1e-9 {
                return Err(format!("part {name}: mates.insertion must point somewhere"));
            }
            mates.push(Mate {
                with: w,
                insertion: ins.map(|x| x / l),
                min_clearance_mm: m["min_clearance_mm"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .clamp(0.0, 5.0),
            });
        }
        parts.push(AssemblyPart {
            name: name.clone(),
            material: p["material"].as_str().unwrap_or("petg").to_string(),
            part,
            place,
            mates,
            nodes,
        });
        names.push(name);
    }
    let mut ghosts = Vec::new();
    for g in design["ghosts"].as_array().into_iter().flatten().take(6) {
        let r = resolve(g, "", &vars).map_err(|e| format!("ghost: {e}"))?;
        let kind = r["kind"].as_str().unwrap_or("box");
        if !["box", "cylinder"].contains(&kind) {
            return Err("ghost.kind must be box or cylinder".into());
        }
        ghosts.push(json!({
            "label": r["label"],
            "kind": kind,
            "size": r["size"],
            "d": r["d"],
            "h": r["h"],
            "at": if r["at"].is_null() { json!([0, 0, 0]) } else { r["at"].clone() },
        }));
    }
    Ok(Assembly {
        parts,
        ghosts,
        params: vars,
    })
}

/// The design schema for Muse: params, parts of CAD nodes, poses, mates, ghosts.
pub fn schema() -> Value {
    let n3 = json!({"type": "array", "items": {"type": ["number", "string"]}, "minItems": 3, "maxItems": 3});
    let nodes = cad::schema()["properties"]["nodes"].clone();
    json!({
        "type": "object",
        "properties": {
            "params": {"type": "object", "description": "Shared sizes: name to a number or an expression string", "additionalProperties": {"type": ["number", "string"]}},
            "parts": {"type": "array", "minItems": 1, "maxItems": MAX_PARTS, "items": {"type": "object", "properties": {
                "name": {"type": "string"},
                "material": {"type": "string", "enum": ["petg", "asa"]},
                "nodes": nodes,
                "root": {"type": "string"},
                "place": {"type": "object", "properties": {"at": n3.clone(), "deg": n3.clone()}},
                "mates": {"type": "array", "items": {"type": "object", "properties": {
                    "with": {"type": "string"}, "insertion": n3.clone(), "min_clearance_mm": {"type": ["number", "string"]}
                }, "required": ["with"]}}
            }, "required": ["name", "nodes"]}},
            "ghosts": {"type": "array", "maxItems": 6, "items": {"type": "object", "properties": {
                "label": {"type": "string"}, "kind": {"type": "string", "enum": ["box", "cylinder"]},
                "size": n3.clone(), "d": {"type": ["number", "string"]}, "h": {"type": ["number", "string"]}, "at": n3
            }, "required": ["label", "kind"]}}
        },
        "required": ["parts"]
    })
}

pub const GUIDE: &str = "Design a complete solution: design = {params: {name: number or expression}, parts: [{name, material: petg|asa, nodes: [CAD nodes], root?, place: {at: [x,y,z], deg: [x,y,z]}, mates: [{with: earlier part name, insertion: [x,y,z] direction this part moves to come off, min_clearance_mm}]}], ghosts: [{label, kind: box|cylinder, size: [x,y,z] or d and h, at: [x,y,z] bottom centre}]}. \
Use as many parts as the solution needs (for example a case and its lid, a knob and a spacer), each a closed solid, modelled standing as it prints on z = 0. place puts the part in the assembly (rotate about x, y, z, then move): a lid modelled upside down for printing gets deg [180, 0, 0] and its at. mates declares how a part slides onto an earlier one; Any Part Made checks the two do not overlap, keep min_clearance_mm and can slide along insertion. \
Any number may be an expression string using params and recorded dimensions, for example \"$board_length_mm + 2 * $wall + 0.6\", with + - * / ( ) min(a, b) max(a, b); share sizes through params so parts stay consistent. \
ghosts show the measured objects in the hologram (the board in its pocket, the shaft in its bore), in assembly coordinates, sized from the recorded dimensions. ";

#[cfg(test)]
mod tests {
    use super::*;

    fn dims() -> BTreeMap<String, f64> {
        BTreeMap::from([
            ("board_length_mm".to_string(), 21.5),
            ("board_width_mm".to_string(), 18.6),
            ("board_height_mm".to_string(), 4.5),
        ])
    }

    #[test]
    fn expressions() {
        let v = dims();
        assert_eq!(eval("$board_length_mm + 2 * 2", &v).unwrap(), 25.5);
        assert_eq!(eval("($board_width_mm - 0.6) / 2", &v).unwrap(), 9.0);
        assert_eq!(eval("max(3, $board_height_mm) - -1", &v).unwrap(), 5.5);
        assert!(eval("$nope + 1", &v)
            .unwrap_err()
            .contains("not a parameter"));
        assert!(eval("2 *", &v).is_err());
        assert!(eval("1 / 0", &v).is_err());
        assert!(eval("rm(1, 2)", &v).is_err());
    }

    fn case_and_lid() -> Value {
        json!({
            "params": {"wall": 2, "gap": "0.3", "ol": "$board_length_mm + 2 * $wall + 0.6", "ow": "$board_width_mm + 2 * $wall + 0.6", "oh": 12, "lip": "$oh - 2"},
            "parts": [
                {"name": "case", "nodes": [
                    {"id": "shell", "op": "box", "size": ["$ol", "$ow", "$oh"], "r": 1},
                    {"id": "seat", "op": "pocket", "at": [0, 0, "$oh"], "axis": "+z", "u": "+x", "depth": "$oh - 2"},
                    {"id": "c", "op": "difference", "of": ["shell", "seat"]}
                ]},
                {"name": "lid", "place": {"at": [0, 0, "$oh + 2"], "deg": [180, 0, 0]}, "mates": [{"with": "case", "insertion": [0, 0, 1], "min_clearance_mm": 0.2}], "nodes": [
                    {"id": "plate", "op": "box", "size": ["$ol", "$ow", 2], "r": 1},
                    {"id": "plug", "op": "box", "size": ["$board_length_mm", "$board_width_mm", 2], "at": [0, 0, 2]},
                    {"id": "l", "op": "union", "of": ["plate", "plug"]}
                ]}
            ],
            "ghosts": [{"label": "board", "kind": "box", "size": ["$board_length_mm", "$board_width_mm", "$board_height_mm"], "at": [0, 0, 2]}]
        })
    }

    #[test]
    fn a_case_with_a_lid_resolves_and_meshes() {
        let a = parse(&case_and_lid(), &dims()).unwrap();
        assert_eq!(a.parts.len(), 2);
        assert_eq!(a.params["ol"], 26.1);
        assert_eq!(a.params["lip"], 10.0);
        for p in &a.parts {
            let m = p.part.mesh(0.5).unwrap();
            assert!(m.is_watertight(), "{}", p.name);
        }
        assert_eq!(a.ghosts[0]["size"], json!([21.5, 18.6, 4.5]));
        // The case seen from the lid: the lid is flipped and 14 mm up, so the case sits
        // flipped below it; the case leaves the lid downwards in the lid's own frame,
        // which after the flip is +z.
        let (rel, dir) = a.mate_for_check(1, &a.parts[1].mates[0]);
        assert_eq!(rel.len(), 16);
        assert!((rel[11] - 14.0).abs() < 1e-6, "{rel:?}");
        assert!((dir[2] - 1.0).abs() < 1e-6, "{dir:?}");
    }

    #[test]
    fn single_part_designs_still_work_and_errors_are_clear() {
        let one = json!({"nodes": [{"id": "b", "op": "box", "size": [10, 10, 10]}]});
        assert_eq!(parse(&one, &dims()).unwrap().parts.len(), 1);
        let mut bad = case_and_lid();
        bad["parts"][1]["mates"][0]["with"] = json!("lid2");
        assert!(parse(&bad, &dims()).unwrap_err().contains("earlier part"));
        let cyc = json!({"params": {"a": "$b + 1", "b": "$a"}, "nodes": [{"id": "b", "op": "box", "size": [1, 1, 1]}]});
        assert!(parse(&cyc, &dims()).unwrap_err().contains("param"));
        let hide = json!({"params": {"board_length_mm": 30}, "nodes": [{"id": "b", "op": "box", "size": [1, 1, 1]}]});
        assert!(parse(&hide, &dims()).unwrap_err().contains("hide"));
    }
}
