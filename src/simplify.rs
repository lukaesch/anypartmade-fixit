//! Mesh simplification by edge collapse with quadric error (Garland and Heckbert, 1997).
//! Grid meshes have many tiny triangles on flat and round faces; this merges them while
//! the error stays under a bound, keeps the mesh closed (link condition) and refuses
//! collapses that would flip a triangle. Checkers then get a small file.

use crate::knob::Mesh;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

type Q = [f64; 10];

fn plane_quadric(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<Q> {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l < 1e-12 {
        return None;
    }
    // Weight by area so tiny slivers do not dominate.
    let area = l / 2.0;
    let (x, y, z) = (n[0] / l, n[1] / l, n[2] / l);
    let d = -(x * a[0] + y * a[1] + z * a[2]);
    Some([
        x * x * area,
        x * y * area,
        x * z * area,
        x * d * area,
        y * y * area,
        y * z * area,
        y * d * area,
        z * z * area,
        z * d * area,
        d * d * area,
    ])
}

fn qadd(a: &Q, b: &Q) -> Q {
    std::array::from_fn(|i| a[i] + b[i])
}

fn qerr(q: &Q, p: [f64; 3]) -> f64 {
    let [x, y, z] = p;
    q[0] * x * x
        + 2.0 * q[1] * x * y
        + 2.0 * q[2] * x * z
        + 2.0 * q[3] * x
        + q[4] * y * y
        + 2.0 * q[5] * y * z
        + 2.0 * q[6] * y
        + q[7] * z * z
        + 2.0 * q[8] * z
        + q[9]
}

/// The point minimising the error, if the quadric is well conditioned.
fn qsolve(q: &Q) -> Option<[f64; 3]> {
    let m = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
    let b = [-q[3], -q[6], -q[8]];
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let scale = (q[0] + q[4] + q[7]).max(1e-12);
    if det.abs() < 1e-6 * scale * scale * scale {
        return None;
    }
    let solve = |col: usize| {
        let mut a = m;
        for r in 0..3 {
            a[r][col] = b[r];
        }
        a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
            - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
    };
    Some([solve(0) / det, solve(1) / det, solve(2) / det])
}

#[derive(PartialEq)]
struct Item {
    cost: f64,
    a: u32,
    b: u32,
    stamp: (u32, u32),
    at: [f64; 3],
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost)
    }
}

fn normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

/// Collapses edges while the squared distance error stays under `max_error_mm`² per
/// unit area and there are more than `target` triangles. Returns a closed mesh.
pub fn simplify(m: &Mesh, target: usize, max_error_mm: f64) -> Mesh {
    let nv = m.vertices.len();
    let mut pos = m.vertices.clone();
    let mut tris: Vec<[u32; 3]> = m.triangles.clone();
    let mut alive = vec![true; tris.len()];
    let mut vf: Vec<Vec<u32>> = vec![Vec::new(); nv];
    let mut q: Vec<Q> = vec![[0.0; 10]; nv];
    for (f, t) in tris.iter().enumerate() {
        if let Some(pq) = plane_quadric(pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize])
        {
            for &v in t {
                q[v as usize] = qadd(&q[v as usize], &pq);
            }
        }
        for &v in t {
            vf[v as usize].push(f as u32);
        }
    }
    let mut stamp = vec![0u32; nv];
    let mut removed_v = vec![false; nv];
    let max_cost = max_error_mm * max_error_mm;
    let candidate = |a: u32, b: u32, q: &[Q], pos: &[[f64; 3]], stamp: &[u32]| -> Item {
        let qq = qadd(&q[a as usize], &q[b as usize]);
        let (pa, pb) = (pos[a as usize], pos[b as usize]);
        let mid = [
            (pa[0] + pb[0]) / 2.0,
            (pa[1] + pb[1]) / 2.0,
            (pa[2] + pb[2]) / 2.0,
        ];
        let mut best = (qerr(&qq, mid), mid);
        for p in [pa, pb] {
            let e = qerr(&qq, p);
            if e < best.0 {
                best = (e, p);
            }
        }
        if let Some(p) = qsolve(&qq) {
            // Only trust the optimum near the edge.
            let near = |c: usize| p[c] >= pa[c].min(pb[c]) - 0.5 && p[c] <= pa[c].max(pb[c]) + 0.5;
            if near(0) && near(1) && near(2) {
                let e = qerr(&qq, p);
                if e < best.0 {
                    best = (e, p);
                }
            }
        }
        Item {
            cost: best.0.max(0.0),
            a,
            b,
            stamp: (stamp[a as usize], stamp[b as usize]),
            at: best.1,
        }
    };
    let mut heap = BinaryHeap::new();
    let mut seen = HashSet::new();
    for t in &tris {
        for k in 0..3 {
            let (a, b) = (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]));
            if seen.insert((a, b)) {
                heap.push(candidate(a, b, &q, &pos, &stamp));
            }
        }
    }
    drop(seen);
    let mut count = tris.len();
    let neighbours = |v: u32, vf: &[Vec<u32>], tris: &[[u32; 3]], alive: &[bool]| -> HashSet<u32> {
        let mut s = HashSet::new();
        for &f in &vf[v as usize] {
            if alive[f as usize] {
                for &w in &tris[f as usize] {
                    if w != v {
                        s.insert(w);
                    }
                }
            }
        }
        s
    };
    while let Some(it) = heap.pop() {
        if count <= target || it.cost > max_cost {
            break;
        }
        let (a, b) = (it.a, it.b);
        if removed_v[a as usize]
            || removed_v[b as usize]
            || it.stamp != (stamp[a as usize], stamp[b as usize])
        {
            continue;
        }
        let na = neighbours(a, &vf, &tris, &alive);
        if !na.contains(&b) {
            continue;
        }
        let nb = neighbours(b, &vf, &tris, &alive);
        // Link condition: on a closed mesh the edge's two ends share exactly the two
        // vertices of the triangles on either side of it.
        if na.intersection(&nb).count() != 2 {
            continue;
        }
        // No triangle that stays may flip or collapse.
        let mut ok = true;
        for &v in [a, b].iter() {
            for &f in &vf[v as usize] {
                let f = f as usize;
                if !alive[f] {
                    continue;
                }
                let t = tris[f];
                if t.contains(&a) && t.contains(&b) {
                    continue;
                }
                let before = normal(pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
                let p = t.map(|w| {
                    if w == a || w == b {
                        it.at
                    } else {
                        pos[w as usize]
                    }
                });
                let after = normal(p[0], p[1], p[2]);
                let (lb, la) = (
                    (before.iter().map(|x| x * x).sum::<f64>()).sqrt(),
                    (after.iter().map(|x| x * x).sum::<f64>()).sqrt(),
                );
                if la < 1e-10
                    || (before[0] * after[0] + before[1] * after[1] + before[2] * after[2])
                        < 0.3 * la * lb
                {
                    ok = false;
                    break;
                }
            }
            if !ok {
                break;
            }
        }
        if !ok {
            continue;
        }
        // Collapse b into a.
        pos[a as usize] = it.at;
        q[a as usize] = qadd(&q[a as usize], &q[b as usize]);
        removed_v[b as usize] = true;
        let bf = std::mem::take(&mut vf[b as usize]);
        for f in bf {
            let fi = f as usize;
            if !alive[fi] {
                continue;
            }
            if tris[fi].contains(&a) {
                alive[fi] = false;
                count -= 1;
            } else {
                for w in tris[fi].iter_mut() {
                    if *w == b {
                        *w = a;
                    }
                }
                vf[a as usize].push(f);
            }
        }
        vf[a as usize].retain(|&f| alive[f as usize]);
        stamp[a as usize] += 1;
        for w in neighbours(a, &vf, &tris, &alive) {
            let (x, y) = (a.min(w), a.max(w));
            heap.push(candidate(x, y, &q, &pos, &stamp));
        }
    }
    // Compact.
    let mut map = vec![u32::MAX; nv];
    let mut out = Mesh::default();
    for (f, t) in tris.iter().enumerate() {
        if !alive[f] {
            continue;
        }
        let t = t.map(|v| {
            if map[v as usize] == u32::MAX {
                map[v as usize] = out.vertices.len() as u32;
                out.vertices.push(pos[v as usize]);
            }
            map[v as usize]
        });
        out.triangles.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn a_box_with_a_pocket_shrinks_and_stays_closed() {
        let dims = BTreeMap::from([
            ("board_length_mm".to_string(), 21.0),
            ("board_width_mm".to_string(), 17.8),
        ]);
        let part = json!({"nodes": [
            {"id": "shell", "op": "box", "size": [27, 24, 10], "r": 1.5},
            {"id": "seat", "op": "pocket", "at": [0, 0, 10], "axis": "+z", "u": "+x", "depth": 8},
            {"id": "case", "op": "difference", "of": ["shell", "seat"]}
        ]});
        let p = crate::cad::parse(&part, &dims).unwrap();
        let m = p.mesh(0.3).unwrap();
        let s = simplify(&m, 2000, 0.02);
        assert!(s.is_watertight());
        assert!(
            s.triangles.len() * 10 < m.triangles.len(),
            "{} from {}",
            s.triangles.len(),
            m.triangles.len()
        );
        assert!((s.volume() - m.volume()).abs() / m.volume() < 0.01);
        // The pocket walls are still where they were: inner length 21.6 mm.
        // Vertices on the inner wall (|x| near 10.8) must sit on it, not drift.
        // No surface inside the wall material, and the wall itself still has vertices.
        let mid = |v: &&[f64; 3]| v[2] > 2.5 && v[2] < 9.5;
        assert!(!s
            .vertices
            .iter()
            .filter(mid)
            .any(|v| v[0].abs() > 10.85 && v[0].abs() < 11.5 && v[1].abs() < 9.0));
        assert!(s.vertices.iter().any(|v| (v[0].abs() - 10.8).abs() < 0.01));
        println!("{} triangles from {}", s.triangles.len(), m.triangles.len());
    }
}
