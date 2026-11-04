//! The agent page: a chat with Muse Spark, styled as a design console. Photos get a scan
//! beam and drawn outlines; the part appears as a WebGL hologram that builds up, is
//! inspected by Any Part Made (issues pulse where the checker found them) and turns
//! solid when it passes. Everything is inline: no libraries, no external assets.

const SCRIPT: &str = r##"
const $ = (id) => document.getElementById(id);
const MAX_EDGE = 2048;
let current = null, pending = [], busy = false, viewing = null;
const COLORS = { coin: "#ffb000", shaft: "#38e8ff", knob: "#7dff9c", ruler: "#ff4fa3" };
const VERDICT = { ok: "PASSED", needs_changes: "NEEDS CHANGES", needs_review: "HUMAN REVIEW", refused: "REFUSED", needs_upload: "WAITING" };
const LEAD = { opt_express: "Express", opt_standard: "Standard", opt_economy: "Economy" };
const NAMES = { shaft_diameter_mm: "Shaft", flat_to_round_mm: "Flat", shaft_length_mm: "Shaft length",
  board_length_mm: "Board L", board_width_mm: "Board W", board_height_mm: "Board H" };

function h(tag, attrs, text) {
  const e = document.createElement(tag);
  for (const k in attrs || {}) e.setAttribute(k, attrs[k]);
  if (text) e.textContent = text;
  return e;
}
function svg(tag, attrs, text) {
  const e = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const k in attrs || {}) e.setAttribute(k, attrs[k]);
  if (text) e.textContent = text;
  return e;
}
function pct(c) { return Math.round((c || 0) * 100) + "%"; }

async function shrink(file) {
  let bmp;
  try { bmp = await createImageBitmap(file, { imageOrientation: "from-image" }); }
  catch (e) {
    bmp = await new Promise((ok, fail) => {
      const i = new Image();
      i.onload = () => ok(i);
      i.onerror = () => fail(new Error("This photo format cannot be read here. Try JPEG."));
      i.src = URL.createObjectURL(file);
    });
  }
  const k = Math.min(1, MAX_EDGE / Math.max(bmp.width, bmp.height));
  const c = document.createElement("canvas");
  c.width = Math.round(bmp.width * k); c.height = Math.round(bmp.height * k);
  c.getContext("2d").drawImage(bmp, 0, 0, c.width, c.height);
  return await new Promise((ok) => c.toBlob(ok, "image/jpeg", 0.9));
}

// Photos taken on this device are shown from memory: no wait for the server.
const localPhotos = {};
function photoUrl(n) { return localPhotos[n] || "/api/session/" + current.id + "/photo/" + n; }

// ---------- 3MF reading: zip entry, raw deflate, vertices and triangles ----------
async function read3mf(buf) {
  const d = new DataView(buf);
  let o = 0;
  while (o + 30 < buf.byteLength && d.getUint32(o, true) === 0x04034b50) {
    const method = d.getUint16(o + 8, true), size = d.getUint32(o + 18, true);
    const nl = d.getUint16(o + 26, true), xl = d.getUint16(o + 28, true);
    const name = new TextDecoder().decode(new Uint8Array(buf, o + 30, nl));
    const data = new Uint8Array(buf, o + 30 + nl + xl, size);
    if (name.endsWith(".model")) {
      let xml;
      if (method === 8) xml = await new Response(new Blob([data]).stream().pipeThrough(new DecompressionStream("deflate-raw"))).text();
      else xml = new TextDecoder().decode(data);
      const v = [], t = [];
      for (const m of xml.matchAll(/<vertex x="([-\d.e]+)" y="([-\d.e]+)" z="([-\d.e]+)"/g)) v.push(+m[1], +m[2], +m[3]);
      for (const m of xml.matchAll(/<triangle v1="(\d+)" v2="(\d+)" v3="(\d+)"/g)) t.push(+m[1], +m[2], +m[3]);
      return { v: new Float32Array(v), t: new Uint32Array(t) };
    }
    o += 30 + nl + xl + size;
  }
  throw new Error("no model in the file");
}

// ---------- The hologram: WebGL2, one live at a time ----------
const VS = `#version 300 es
in vec3 p; in vec3 n; in vec3 b;
uniform mat4 mvp; uniform mat4 rot;
out vec3 vn; out vec3 vb; out float vz;
void main() { vn = (rot * vec4(n, 0.0)).xyz; vb = b; vz = p.z; gl_Position = mvp * vec4(p, 1.0); }`;
const FS = `#version 300 es
precision highp float;
in vec3 vn; in vec3 vb; in float vz;
uniform float cut; uniform float solid; uniform float scan; uniform float flash; uniform vec3 tint;
out vec4 o;
void main() {
  if (vz > cut) discard;
  vec3 nn = normalize(vn);
  float e = min(min(vb.x, vb.y), vb.z);
  float wire = 1.0 - smoothstep(0.0, 1.2 * fwidth(e), e);
  float fres = pow(1.0 - abs(nn.z), 2.0);
  vec3 holo = tint * (0.10 + 0.9 * wire + 0.45 * fres);
  float lit = 0.35 + 0.65 * max(dot(nn, normalize(vec3(-0.4, 0.5, 0.8))), 0.0);
  vec3 body = vec3(0.62, 0.68, 0.78) * lit + tint * 0.25 * fres + vec3(0.9, 1.0, 1.0) * 0.12 * wire;
  vec3 c = mix(holo, body, solid);
  float edgeBand = 1.0 - smoothstep(0.0, 0.6, cut - vz);
  float scanBand = 1.0 - smoothstep(0.0, 0.5, abs(vz - scan));
  c += vec3(0.6, 1.0, 1.0) * (edgeBand * 0.9 + scanBand * 1.2) + vec3(flash);
  o = vec4(c, mix(0.30 + 0.70 * wire, 1.0, solid));
}`;
const PVS = `#version 300 es
in vec3 p; in vec3 col; uniform mat4 mvp; uniform float t; out vec3 vc;
void main() { vc = col; gl_Position = mvp * vec4(p, 1.0); gl_PointSize = 18.0 + 10.0 * sin(t * 6.0); }`;
const PFS = `#version 300 es
precision highp float; in vec3 vc; out vec4 o;
void main() { vec2 q = gl_PointCoord - 0.5; float r = length(q); if (r > 0.5) discard;
  float ring = smoothstep(0.5, 0.35, r) * smoothstep(0.15, 0.3, r) + smoothstep(0.14, 0.0, r);
  o = vec4(vc * 1.4, ring); }`;

const LVS = `#version 300 es
in vec3 p; uniform mat4 mvp; uniform float z; uniform float scale;
void main() { gl_Position = mvp * vec4(p.xy * scale, z, 1.0); }`;
const GVS = `#version 300 es
in vec3 p; uniform mat4 mvp; void main() { gl_Position = mvp * vec4(p, 1.0); }`;
const LFS = `#version 300 es
precision highp float; uniform vec4 col; out vec4 o; void main() { o = col; }`;

function mat(a, b) { const r = new Float32Array(16); for (let i = 0; i < 4; i++) for (let j = 0; j < 4; j++) {
  let s = 0; for (let k = 0; k < 4; k++) s += a[k * 4 + j] * b[i * 4 + k]; r[i * 4 + j] = s; } return r; }
function persp(f, asp, n, fa) { const t = 1 / Math.tan(f / 2); return new Float32Array([t / asp, 0, 0, 0, 0, t, 0, 0, 0, 0, (fa + n) / (n - fa), -1, 0, 0, 2 * fa * n / (n - fa), 0]); }
function rx(a) { const c = Math.cos(a), s = Math.sin(a); return new Float32Array([1, 0, 0, 0, 0, c, s, 0, 0, -s, c, 0, 0, 0, 0, 1]); }
function rz(a) { const c = Math.cos(a), s = Math.sin(a); return new Float32Array([c, s, 0, 0, -s, c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]); }
function tr(x, y, z, k) { return new Float32Array([k, 0, 0, 0, 0, k, 0, 0, 0, 0, k, 0, x * k, y * k, z * k, 1]); }

let live = null;
function prog(gl, vs, fs) {
  const p = gl.createProgram();
  for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
    const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); gl.attachShader(p, s);
  }
  gl.linkProgram(p); return p;
}
// mode: "build" (builds up, scanner loops), "check" (build, scan once, then markers or solid)
function hologram(canvas, mesh, opts) {
  if (live) live.stop();
  const gl = canvas.getContext("webgl2", { antialias: true, premultipliedAlpha: false });
  if (!gl) return null;
  const { v, t } = mesh, n = t.length;
  const pos = new Float32Array(n * 3), nor = new Float32Array(n * 3), bar = new Float32Array(n * 3);
  const lo = [1e9, 1e9, 1e9], hi = [-1e9, -1e9, -1e9];
  for (let i = 0; i < v.length; i += 3) for (let k = 0; k < 3; k++) { lo[k] = Math.min(lo[k], v[i + k]); hi[k] = Math.max(hi[k], v[i + k]); }
  // Face normals, then only real edges in the wireframe: where neighbouring faces meet
  // at more than 25 degrees. Flat faces from the simplifier stay clean.
  const fn = new Float32Array(n);
  for (let f = 0; f < n; f += 3) {
    const a = t[f] * 3, b = t[f + 1] * 3, c = t[f + 2] * 3;
    const ux = v[b] - v[a], uy = v[b + 1] - v[a + 1], uz = v[b + 2] - v[a + 2];
    const wx = v[c] - v[a], wy = v[c + 1] - v[a + 1], wz = v[c + 2] - v[a + 2];
    let nx = uy * wz - uz * wy, ny = uz * wx - ux * wz, nz = ux * wy - uy * wx; const l = Math.hypot(nx, ny, nz) || 1;
    fn[f] = nx / l; fn[f + 1] = ny / l; fn[f + 2] = nz / l;
  }
  const edges = new Map();
  for (let f = 0; f < n; f += 3) for (let k = 0; k < 3; k++) {
    const p = t[f + (k + 1) % 3], q = t[f + (k + 2) % 3];
    const key = p < q ? p + "," + q : q + "," + p;
    (edges.get(key) || edges.set(key, []).get(key)).push(f);
  }
  for (let f = 0; f < n; f += 3) {
    const hide = [0, 0, 0];
    for (let k = 0; k < 3; k++) {
      const p = t[f + (k + 1) % 3], q = t[f + (k + 2) % 3];
      const other = (edges.get(p < q ? p + "," + q : q + "," + p) || []).find((g) => g !== f);
      if (other !== undefined && fn[f] * fn[other] + fn[f + 1] * fn[other + 1] + fn[f + 2] * fn[other + 2] > 0.906) hide[k] = 1;
    }
    for (let k = 0; k < 3; k++) {
      const q = t[f + k] * 3;
      pos.set([v[q], v[q + 1], v[q + 2]], (f + k) * 3);
      nor.set([fn[f], fn[f + 1], fn[f + 2]], (f + k) * 3);
      bar.set([(k === 0 ? 1 : 0) + hide[0], (k === 1 ? 1 : 0) + hide[1], (k === 2 ? 1 : 0) + hide[2]], (f + k) * 3);
    }
  }
  const P = prog(gl, VS, FS), Q = prog(gl, PVS, PFS), R = prog(gl, LVS, LFS), G = prog(gl, GVS, LFS);
  const buf = (data, loc, p) => { const bb = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, bb); gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
    const L = gl.getAttribLocation(p, loc); gl.enableVertexAttribArray(L); gl.vertexAttribPointer(L, 3, gl.FLOAT, false, 0, 0); };
  const vao = gl.createVertexArray(); gl.bindVertexArray(vao);
  buf(pos, "p", P); buf(nor, "n", P); buf(bar, "b", P);
  const marks = opts.markers || [];
  const mvao = gl.createVertexArray(); gl.bindVertexArray(mvao);
  if (marks.length) { buf(new Float32Array(marks.flatMap((m) => m.p)), "p", Q); buf(new Float32Array(marks.flatMap((m) => m.c)), "col", Q); }
  // The scanner ring: a unit circle, scaled and lifted per draw.
  const ring = [];
  for (let i = 0; i < 128; i++) { const a = i / 128 * Math.PI * 2; ring.push(Math.cos(a), Math.sin(a), 0); }
  const rvao = gl.createVertexArray(); gl.bindVertexArray(rvao); buf(new Float32Array(ring), "p", R);
  // A measuring floor in millimetres: a 10 mm grid (every fifth line strong), dimension
  // lines along the box edges, and the x, y, z axes the checker reports positions in.
  const z0 = lo[2] - 0.05, pad = 10;
  const gx0 = Math.floor((lo[0] - pad) / 10) * 10, gx1 = Math.ceil((hi[0] + pad) / 10) * 10;
  const gy0 = Math.floor((lo[1] - pad) / 10) * 10, gy1 = Math.ceil((hi[1] + pad) / 10) * 10;
  const minor = [], major = [];
  for (let x = gx0; x <= gx1; x += 10) (x % 50 ? minor : major).push(x, gy0, z0, x, gy1, z0);
  for (let y = gy0; y <= gy1; y += 10) (y % 50 ? minor : major).push(gx0, y, z0, gx1, y, z0);
  const off = 4, tk = 2;
  // Dimension lines go on the edges that face the viewer, chosen again every frame.
  const dimsFor = (ys, xs, cxr, cyr) => {
    const dx = ys < 0 ? lo[1] - off : hi[1] + off, dy = xs > 0 ? hi[0] + off : lo[0] - off;
    const hx = cxr > 0 ? hi[0] + off : lo[0] - off, hy = cyr > 0 ? hi[1] + off : lo[1] - off;
    return { lines: [lo[0], dx, z0, hi[0], dx, z0, lo[0], dx - tk, z0, lo[0], dx + tk, z0, hi[0], dx - tk, z0, hi[0], dx + tk, z0,
      dy, lo[1], z0, dy, hi[1], z0, dy - tk, lo[1], z0, dy + tk, lo[1], z0, dy - tk, hi[1], z0, dy + tk, hi[1], z0,
      hx, hy, lo[2], hx, hy, hi[2], hx - tk, hy, lo[2], hx + tk, hy, lo[2], hx - tk, hy, hi[2], hx + tk, hy, hi[2]],
      at: [[(lo[0] + hi[0]) / 2, dx + (ys < 0 ? -4 : 4), z0], [dy + (xs > 0 ? 5 : -5), (lo[1] + hi[1]) / 2, z0], [hx + (cxr > 0 ? 5 : -5), hy, (lo[2] + hi[2]) / 2]] };
  };
  const ax = 10, axes = [[gx0, gy0, z0, gx0 + ax, gy0, z0], [gx0, gy0, z0, gx0, gy0 + ax, z0], [gx0, gy0, z0, gx0, gy0, z0 + ax]];
  const lines = (arr) => { const a = gl.createVertexArray(); gl.bindVertexArray(a); buf(new Float32Array(arr), "p", G); return [a, arr.length / 3]; };
  const gMinor = lines(minor), gMajor = lines(major), gAxes = axes.map(lines);
  const gDims = lines(dimsFor(-1, 1, 1, -1).lines);
  const dimBuf = gl.createBuffer();
  gl.bindVertexArray(gDims[0]); gl.bindBuffer(gl.ARRAY_BUFFER, dimBuf); gl.bufferData(gl.ARRAY_BUFFER, 36 * 3 * 4, gl.DYNAMIC_DRAW);
  { const L = gl.getAttribLocation(G, "p"); gl.enableVertexAttribArray(L); gl.vertexAttribPointer(L, 3, gl.FLOAT, false, 0, 0); }
  const holder = canvas.parentElement;
  const label = (text, cls) => { const e = document.createElement("span"); e.className = "lbl " + (cls || ""); e.textContent = text; holder.appendChild(e); return e; };
  const fmt = (v) => (Math.round(v * 10) / 10) + " mm";
  const dimLabels = [label(fmt(hi[0] - lo[0])), label(fmt(hi[1] - lo[1])), label(fmt(hi[2] - lo[2]))];
  const labels = [[label("X", "ax x"), [gx0 + ax + 3, gy0, z0]], [label("Y", "ax y"), [gx0, gy0 + ax + 3, z0]], [label("Z", "ax z"), [gx0, gy0, z0 + ax + 3]]];
  holder.appendChild(Object.assign(document.createElement("span"), { className: "legend", textContent: "GRID 10 MM · X Y Z AS THE CHECKER" }));
  const size = Math.max(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) || 1;
  const k = 1.25 / size, cx = -(lo[0] + hi[0]) / 2, cy = -(lo[1] + hi[1]) / 2, cz = -(lo[2] + hi[2]) / 2;
  let yaw = 0.6, pitch = -0.85, drag = null, running = true;
  canvas.onpointerdown = (e) => { drag = [e.clientX, e.clientY, yaw, pitch]; canvas.setPointerCapture(e.pointerId); };
  canvas.onpointermove = (e) => { if (drag) { yaw = drag[2] + (e.clientX - drag[0]) * 0.01; pitch = Math.max(-1.6, Math.min(0, drag[3] + (e.clientY - drag[1]) * 0.01)); } };
  canvas.onpointerup = () => { drag = null; };
  const t0 = performance.now();
  function frame(now) {
    if (!running) return;
    const s = (now - t0) / 1000;
    const W = Math.round(canvas.clientWidth * devicePixelRatio), H = Math.round(canvas.clientHeight * devicePixelRatio);
    if (!W || !H) { requestAnimationFrame(frame); return; }
    if (canvas.width !== W || canvas.height !== H) { canvas.width = W; canvas.height = H; }
    gl.viewport(0, 0, W, H); gl.clearColor(0, 0, 0, 0); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.enable(gl.DEPTH_TEST); gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    if (!drag) yaw += 0.004;
    const rot = mat(rx(pitch), rz(yaw));
    // Step back in portrait so the whole part stays in view.
    const dist = 3.1 / Math.min(1, W / H);
    const mvp = mat(persp(0.75, W / H, 0.1, 40), mat(tr(0, 0, -dist, 1), mat(rot, tr(cx, cy, cz, k))));
    const zr = hi[2] - lo[2];
    const build = Math.min(1, s / 1.6);
    const scanT = opts.mode === "build" ? (s % 2.2) / 2.2 : Math.min(1, Math.max(0, (s - 1.7) / 1.4));
    const after = opts.mode === "check" ? Math.max(0, s - 3.2) : 0;
    const solid = opts.ok ? Math.min(1, after / 0.8) : 0;
    const flash = opts.ok ? Math.max(0, 0.5 - Math.abs(after - 0.25) * 2) * 0.6 : 0;
    gl.useProgram(P); gl.bindVertexArray(vao);
    gl.uniformMatrix4fv(gl.getUniformLocation(P, "mvp"), false, mvp);
    gl.uniformMatrix4fv(gl.getUniformLocation(P, "rot"), false, rot);
    gl.uniform1f(gl.getUniformLocation(P, "cut"), lo[2] + zr * build * 1.02 + 0.01);
    gl.uniform1f(gl.getUniformLocation(P, "scan"), build < 1 ? -1e6 : (scanT > 0 && scanT < 1 ? lo[2] + zr * (1 - scanT) : -1e6));
    gl.uniform1f(gl.getUniformLocation(P, "solid"), solid);
    gl.uniform1f(gl.getUniformLocation(P, "flash"), flash);
    gl.uniform3fv(gl.getUniformLocation(P, "tint"), opts.ok && solid > 0 ? [0.45, 1.0, 0.7] : [0.22, 0.91, 1.0]);
    gl.drawArrays(gl.TRIANGLES, 0, n);
    // The measuring floor, dimension lines and axes, in the part's own millimetres.
    gl.useProgram(G);
    gl.uniformMatrix4fv(gl.getUniformLocation(G, "mvp"), false, mvp);
    const gc = gl.getUniformLocation(G, "col");
    const glow = opts.ok && solid > 0 ? [0.45, 1.0, 0.7] : [0.22, 0.91, 1.0];
    const draw = (g, c) => { gl.bindVertexArray(g[0]); gl.uniform4f(gc, c[0], c[1], c[2], c[3]); gl.drawArrays(gl.LINES, 0, g[1]); };
    draw(gMinor, [glow[0], glow[1], glow[2], 0.32]); draw(gMajor, [glow[0], glow[1], glow[2], 0.7]);
    // Which sides face the viewer: the clip depth w of points on each side.
    const wOf = (q) => mvp[3] * q[0] + mvp[7] * q[1] + mvp[11] * q[2] + mvp[15];
    const mx = (lo[0] + hi[0]) / 2, my = (lo[1] + hi[1]) / 2;
    const ys = wOf([mx, lo[1], z0]) < wOf([mx, hi[1], z0]) ? -1 : 1, xs = wOf([hi[0], my, z0]) < wOf([lo[0], my, z0]) ? 1 : -1;
    const d = dimsFor(ys, xs, xs, ys);
    gl.bindBuffer(gl.ARRAY_BUFFER, dimBuf); gl.bufferSubData(gl.ARRAY_BUFFER, 0, new Float32Array(d.lines));
    draw(gDims, [1.0, 1.0, 1.0, 0.8]);
    draw(gAxes[0], [1.0, 0.35, 0.35, 1]); draw(gAxes[1], [0.4, 1.0, 0.45, 1]); draw(gAxes[2], [0.4, 0.6, 1.0, 1]);
    // The inspection ring slides up around the part while Any Part Made scans it.
    if (build >= 1 && scanT > 0 && scanT < 1) {
      const footR = Math.hypot(hi[0] - lo[0], hi[1] - lo[1]) / 2 * 1.1;
      gl.useProgram(R);
      gl.uniformMatrix4fv(gl.getUniformLocation(R, "mvp"), false, mat(persp(0.75, W / H, 0.1, 40), mat(tr(0, 0, -dist, 1), mat(rot, tr(0, 0, cz, k)))));
      gl.uniform1f(gl.getUniformLocation(R, "scale"), footR);
      gl.uniform1f(gl.getUniformLocation(R, "z"), lo[2] + zr * (1 - scanT));
      gl.uniform4f(gl.getUniformLocation(R, "col"), 0.7, 1.0, 1.0, 0.95);
      gl.bindVertexArray(rvao); gl.drawArrays(gl.LINE_LOOP, 0, 128);
    }
    // Labels follow their 3D points.
    const cw = canvas.clientWidth, ch = canvas.clientHeight;
    for (const [el, q] of [...labels, ...dimLabels.map((e, i) => [e, d.at[i]])]) {
      const x = mvp[0] * q[0] + mvp[4] * q[1] + mvp[8] * q[2] + mvp[12], y = mvp[1] * q[0] + mvp[5] * q[1] + mvp[9] * q[2] + mvp[13];
      const w = mvp[3] * q[0] + mvp[7] * q[1] + mvp[11] * q[2] + mvp[15];
      el.style.transform = "translate(" + ((x / w * 0.5 + 0.5) * cw) + "px," + ((0.5 - y / w * 0.5) * ch) + "px) translate(-50%,-50%)";
      el.style.opacity = build >= 1 ? 1 : 0;
    }
    if (marks.length && opts.mode === "check" && s > 3.0) {
      gl.disable(gl.DEPTH_TEST);
      gl.useProgram(Q); gl.bindVertexArray(mvao);
      gl.uniformMatrix4fv(gl.getUniformLocation(Q, "mvp"), false, mvp);
      gl.uniform1f(gl.getUniformLocation(Q, "t"), s);
      gl.drawArrays(gl.POINTS, 0, marks.length);
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
  live = { stop() { running = false; const x = gl.getExtension("WEBGL_lose_context"); if (x) x.loseContext(); } };
  return live;
}

// Issue positions from the checker's text: "near (x, y, z) mm".
function markers(issues) {
  const out = [];
  for (const i of issues || []) {
    const m = String(i.message || "").match(/\((-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\)/);
    if (m) out.push({ p: [+m[1], +m[2], +m[3]], c: i.severity === "error" ? [1.0, 0.25, 0.3] : [1.0, 0.7, 0.1] });
  }
  return out;
}

function holoBox(meshPromise, opts, caption) {
  const box = h("div", { class: "holo" });
  const cv = h("canvas", { class: "gl" });
  box.append(cv, h("div", { class: "hud" }, caption || ""));
  const fallback = () => { if (opts.png) box.replaceChildren(h("img", { class: "knob", src: opts.png, alt: "The part" })); };
  meshPromise.then((mesh) => { requestAnimationFrame(() => { if (!hologram(cv, mesh, opts)) fallback(); }); }).catch(fallback);
  return box;
}
const meshes = {};
function meshFrom(url) { if (!meshes[url]) meshes[url] = fetch(url).then((r) => r.arrayBuffer()).then(read3mf); return meshes[url]; }

// ---------- Cards ----------
function shot(n, overlays, cls, scan) {
  const p = current.photos[n - 1];
  const wrap = h("div", { class: "shot " + (cls || "") + (scan ? " scanning" : "") });
  // Big photos fit the stage height, keeping their outline overlay aligned.
  if (cls === "small") wrap.style.width = "min(100%, " + (50 * p.width / p.height).toFixed(1) + "dvh)";
  const img = h("img", { src: photoUrl(n), alt: "Photo " + n, width: p.width, height: p.height });
  // The scan beam and the outline start once the photo is there.
  const ready = () => wrap.classList.add("ready");
  img.addEventListener("load", ready);
  if (img.complete && img.naturalWidth) ready();
  wrap.appendChild(img);
  if (overlays && overlays.length) {
    const s = svg("svg", { viewBox: "0 0 " + p.width + " " + p.height, class: "overlay" });
    const lw = Math.max(3, Math.max(p.width, p.height) / 260);
    for (const o of overlays) {
      const c = COLORS[o.label] || "#38e8ff";
      if (o.line) { s.appendChild(svg("line", { x1: o.line[0][0], y1: o.line[0][1], x2: o.line[1][0], y2: o.line[1][1], stroke: c, "stroke-width": lw, class: "draw", pathLength: "1" })); continue; }
      if (o.outline) s.appendChild(svg("polygon", { points: o.outline.map((q) => q.join(",")).join(" "), fill: c, "fill-opacity": "0.18",
        stroke: c, "stroke-width": lw, class: "draw", pathLength: "1" }));
    }
    wrap.appendChild(s);
  }
  wrap.appendChild(h("span", { class: "pn" }, "PHOTO " + n));
  wrap.addEventListener("click", () => openViewer(n));
  return wrap;
}
function card(title, cls) {
  const c = h("div", { class: "tool " + (cls || "") });
  c.appendChild(h("div", { class: "tt" }, title));
  return c;
}
function confirmRow(photo) {
  const row = h("div", { class: "chips" });
  const ok = h("button", { class: "chip-btn", type: "button" }, "Looks right");
  ok.addEventListener("click", () => send("Looks right."));
  const again = h("button", { class: "chip-btn ghost", type: "button" }, "Mark it again");
  again.addEventListener("click", () => openViewer(photo));
  row.append(ok, again);
  return row;
}
function measureCard(args, res, latest) {
  if (res.error) { const c = card("SCAN FAILED · PHOTO " + args.photo, "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const r = res.result || {}, sc = r.scale || {}, sh = r.shaft || {};
  const c = card("SAM 3.1 SCAN · PHOTO " + res.photo + " · " + res.id);
  const m = (current.measurements || []).find((x) => x.id === res.id);
  c.appendChild(shot(res.photo, m ? m.overlays : [], "small", true));
  const g = h("div", { class: "readout" });
  if (sh.diameter_mm) g.appendChild(h("div", { class: "big" }, "⌀ " + sh.diameter_mm.toFixed(2) + " mm" + (sh.flat_to_round_mm ? "   flat " + sh.flat_to_round_mm.toFixed(2) : "") +
    (sh.view === "side" && sh.length_mm ? "   L " + sh.length_mm.toFixed(1) : "")));
  else g.appendChild(h("div", { class: "big dim" }, "no shaft locked"));
  g.appendChild(h("div", { class: "small" }, sc.px_per_mm ? "scale " + sc.px_per_mm.toFixed(2) + " px/mm · " + (sc.source === "coin" ? "coin" : "ruler ticks") + " · " + pct(sc.confidence) : "no scale"));
  c.appendChild(g);
  return c;
}
function objectCard(args, res, latest) {
  if (res.error) { const c = card("SCAN FAILED · PHOTO " + args.photo, "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const r = res.result || {};
  const c = card("SAM 3.1 SCAN · " + (args.object || (args.box ? "MARKED BOX" : "TAPPED POINT")).toUpperCase() + " · PHOTO " + res.photo + " · " + res.id);
  const m = (current.measurements || []).find((x) => x.id === res.id);
  c.appendChild(shot(res.photo, m ? m.overlays : [], "small", true));
  const g = h("div", { class: "readout" });
  if (!r.found) g.appendChild(h("div", { class: "big dim" }, r.note || "nothing found"));
  else if (r.length_mm) g.appendChild(h("div", { class: "big" }, r.length_mm.toFixed(2) + " × " + r.width_mm.toFixed(2) + " mm"));
  if (r.scale && r.scale.px_per_mm) g.appendChild(h("div", { class: "small" }, "scale " + r.scale.px_per_mm.toFixed(2) + " px/mm · " + pct(r.scale.confidence)));
  c.appendChild(g);
  return c;
}
function issueList(issues) {
  const ul = h("ul", { class: "issues" });
  for (const i of issues) {
    const li = h("li", { class: i.severity }, i.code + ": " + i.message);
    if (i.fix) li.appendChild(h("div", { class: "fix" }, "→ " + i.fix));
    ul.appendChild(li);
  }
  return ul;
}
function roundCard(res, latest) {
  if (res.error) { const c = card("BUILD REJECTED", "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const r = (current.rounds || [])[res.round - 1];
  const v = res.verdict, ok = v === "ok";
  const issues = res.issues || [];
  const ch = (r && r.changes) || {};
  const errs = issues.filter((i) => i.severity === "error");
  if (!latest) {
    const d = h("details", { class: "round-line " + (ok ? "good" : "") });
    d.appendChild(h("summary", null, "ROUND " + res.round + " · " + (VERDICT[v] || v) + (errs.length ? " · " + errs.map((i) => i.code).join(", ") : "")));
    if (r) d.appendChild(h("img", { class: "thumb3d", src: r.picture_url, alt: "Round " + r.n }));
    if (ch.summary) d.appendChild(h("p", { class: "muse" }, ch.summary));
    if (issues.length) d.appendChild(issueList(issues));
    return d;
  }
  const c = card("ROUND " + res.round + " · ANY PART MADE INSPECTION", ok ? "good" : "");
  if (r) c.appendChild(holoBox(meshFrom(r.model_url), { mode: "check", ok: ok, markers: markers(issues), png: r.picture_url },
    ok ? "VERDICT: PASSED · MATERIALIZED" : "VERDICT: " + (VERDICT[v] || v) + " · " + errs.length + " TO FIX"));
  const changes = (ch.changes || []).filter((x) => x.from !== x.to);
  if (issues.length || changes.length) {
    const d = h("details", { class: "more" });
    d.appendChild(h("summary", null, (errs.length ? errs.length + " to fix" : "no errors") + (issues.length - errs.length ? " · " + (issues.length - errs.length) + " warnings" : "") +
      (changes.length ? " · " + changes.length + " changes by Muse" : "") + " · tap for details"));
    if (changes.length) { const ul = h("ul", { class: "changes" }); for (const x of changes) ul.appendChild(h("li", null, x.field + ": " + x.from + " → " + x.to)); d.appendChild(ul); }
    if (issues.length) d.appendChild(issueList(issues));
    c.appendChild(d);
  }
  if (ok && res.guarantee) {
    const cert = h("div", { class: "cert" });
    cert.appendChild(h("div", { class: "tt" }, "GUARANTEED BY ANY PART MADE"));
    for (const d of res.guarantee) cert.appendChild(h("div", null, d.label + "  " + d.nominal_mm + " mm  " + d.tolerance_mm));
    for (const i of res.interfaces || []) cert.appendChild(h("div", null, "fits " + i.replace(/_/g, " ")));
    c.appendChild(cert);
  }
  if (r) c.appendChild(h("a", { href: r.model_url, download: "", class: "dl" }, "DOWNLOAD 3MF · " + Math.round(r.file_bytes / 1024) + " KB"));
  return c;
}
function quoteCard(res) {
  if (res.error) { const c = card("PRICE", "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const c = card("ANY PART MADE · QUOTE");
  const row = h("div", { class: "opts3" });
  const short = (d) => String(d || "").slice(5).replace("-", "/");
  for (const o of res.options || []) {
    const d = o.delivery_estimate || {};
    const b = h("button", { class: "opt3", type: "button" });
    b.append(h("span", { class: "lead" }, LEAD[o.option_id] || o.lead_time), h("span", { class: "price" }, o.price_display), h("span", { class: "when" }, short(d.earliest) + " to " + short(d.latest)));
    b.addEventListener("click", () => send("I take " + (LEAD[o.option_id] || o.lead_time) + " (" + o.option_id + ")."));
    row.appendChild(b);
  }
  c.appendChild(row);
  c.appendChild(h("p", { class: "note" }, "Test quote from a simulated partner. Shipping included."));
  return c;
}
function orderCard(res) {
  if (res.error) { const c = card("APPROVAL", "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const c = card("APPROVAL ON ANYPARTMADE.COM", "good approve-card");
  c.appendChild(h("p", null, res.price + ". Nothing is ordered until you approve it there."));
  c.appendChild(h("a", { class: "btn primary", href: res.order_url, target: "_blank", rel: "noopener" }, "Review and approve"));
  return c;
}
function statusCard(res) {
  const c = card("ORDER STATUS", res.error ? "bad" : "good");
  c.appendChild(h("p", null, res.error || (res.status_label || res.status) + (res.test_mode ? " · test mode" : "")));
  return c;
}

// ---------- Rendering: only new or changed items, so animations keep running ----------
const shown = new Map();
function items(s) {
  const out = [], results = {};
  for (const m of s.chat || []) if (m.role === "tool_result") results[m.id] = m.result || {};
  const chat = s.chat || [];
  const lastMeasure = [...chat].reverse().find((m) => m.role === "tool_call" && (m.name === "measure_photo" || m.name === "measure_object"));
  const lastRoundCall = [...chat].reverse().find((m) => m.role === "tool_call" && m.name === "build_and_check" && results[m.id] && !results[m.id].error);
  const lastAssistant = [...chat].reverse().find((m) => m.role === "assistant");
  const waiting = s.agent !== "working" && s.agent !== "building";
  chat.forEach((m, i) => {
    if (m.role === "tool_result") return;
    const res = m.role === "tool_call" ? results[m.id] : null;
    if (m.role === "tool_call" && !res) return;
    out.push({ key: "m" + i, sig: JSON.stringify([m, res]), m, res, latest: false, choices: false });
  });
  return out;
}

// ---------- The stage: one scene at a time, full screen ----------
function scene(s) {
  const chat = s.chat || [], results = {};
  for (const m of chat) if (m.role === "tool_result") results[m.id] = m.result || {};
  const waiting = s.agent !== "working" && s.agent !== "building";
  if (s.agent === "building" && s.build) return { key: "fab" + (s.rounds || []).length + (lastBuilt ? "b" : ""), el: () => {
    const c = card("FABRICATOR · RUST ON THIS DEVICE");
    c.appendChild(lastBuilt ? holoBox(Promise.resolve(lastBuilt), { mode: "build" }, "MATERIALIZING · ANY PART MADE INSPECTS") : h("div", { class: "holo wait" }, "BUILDING THE MESH"));
    return c;
  } };
  const SCENES = ["build_and_check", "measure_photo", "measure_object", "quote", "order_link", "order_status"];
  const last = [...chat].reverse().find((m) => m.role === "tool_call" && SCENES.includes(m.name) && results[m.id] && !(m.name === "build_and_check" && results[m.id].error));
  if (last) {
    const res = results[last.id];
    const live = last.name.startsWith("measure") ? waiting : true;
    return { key: JSON.stringify([last.id, res, live, last.name === "build_and_check" ? (s.rounds || []).length : 0]), el: () => {
      if (last.name === "build_and_check") return roundCard(res, true);
      if (last.name === "measure_photo") return measureCard(last.args || {}, res, live);
      if (last.name === "measure_object") return objectCard(last.args || {}, res, live);
      const c = last.name === "quote" ? quoteCard(res) : last.name === "order_link" ? orderCard(res) : statusCard(res);
      const passed = [...(s.rounds || [])].reverse().find((r) => r.check && r.check.verdict === "ok");
      if (passed) { const hb = holoBox(meshFrom(passed.model_url), { mode: "check", ok: true, png: passed.picture_url }, "ROUND " + passed.n + " · PASSED"); hb.classList.add("compact"); c.insertBefore(hb, c.children[1]); }
      return c;
    } };
  }
  const photos = s.photos || [];
  if (photos.length) return { key: "look" + photos.length, el: () => {
    const c = card("MUSE SPARK · LOOKING AT PHOTO " + photos.length);
    c.appendChild(shot(photos.length, [], "small", true));
    return c;
  } };
  return { key: "start", el: () => $("start-tpl").content.firstElementChild.cloneNode(true) };
}
let stageKey = null;
function renderStage(s) {
  const sc = scene(s);
  if (sc.key === stageKey) return;
  stageKey = sc.key;
  const el = sc.el();
  el.classList.add("scene");
  $("stage").replaceChildren(el);
  el.scrollTop = 0;
  wireStart();
}
// A choice can be an action: mark on the photo, take a photo, or type.
function choose(c) {
  if (busy) return;
  const photos = (current && current.photos) || [];
  const lastMeasured = [...((current && current.measurements) || [])].reverse()[0];
  const which = lastMeasured ? lastMeasured.photo : photos.length;
  if (/\bruler\b/i.test(c) && photos.length) { openViewer(which, "ruler"); return; }
  if (/\b(mark|tap)\b/i.test(c) && photos.length) { openViewer(which); return; }
  if (/\b(take|new|add)\b.*\b(photo|picture)\b|\bcamera\b/i.test(c)) { $("cam").click(); return; }
  if (/^\s*type\b/i.test(c)) { $("text").placeholder = "Type the size in mm"; $("text").focus(); return; }
  send(c);
}
function renderDialog(s) {
  const chat = s.chat || [];
  const last = [...chat].reverse().find((m) => m.role === "assistant");
  const waiting = s.agent !== "working" && s.agent !== "building";
  const box = $("dialog");
  box.replaceChildren();
  box.appendChild(h("div", { class: "who" }, waiting ? "MUSE SPARK" : (s.agent === "building" ? "FABRICATOR" : "MUSE SPARK · THINKING")));
  if (!last) { box.appendChild(h("p", { class: "line" }, "Show me what the part must fit. A photo with a ruler or a coin right next to it works best.")); return; }
  const lines = String(last.text).split("\n");
  const ci = lines.findIndex((l) => /^\s*Choices:/i.test(l));
  const choices = ci >= 0 ? lines[ci].replace(/^\s*Choices:\s*/i, "").split("|").map((x) => x.trim()).filter(Boolean) : [];
  if (ci >= 0) lines.splice(ci, 1);
  // Links become buttons; nothing to copy.
  let text = lines.join("\n").trim();
  const urls = [...new Set(text.match(/https:\/\/[^\s)]+/g) || [])];
  for (const u of urls) text = text.split(u).join("").replace(/\s+([.,:])/g, "$1");
  const p = h("p", { class: "line clamp" + (waiting ? "" : " dimmed") }, text.replace(/[ \t]+\n/g, "\n").replace(/\n{2,}/g, "\n").trim());
  p.addEventListener("click", () => p.classList.toggle("clamp"));
  box.appendChild(p);
  const order = current && current.order && current.order.order_url;
  const onStage = !!document.querySelector("#stage .approve-card");
  for (const u of onStage ? [] : urls) {
    const a = h("a", { class: "btn primary approve", href: u, target: "_blank", rel: "noopener" }, u === order || /\/o\//.test(u) ? "Review and approve on anypartmade.com" : "Open link");
    box.appendChild(a);
  }
  const stagePicks = !!document.querySelector("#stage .opts3, #stage .approve-card");
  if (waiting && choices.length && !stagePicks) {
    const row = h("div", { class: "chips" });
    for (const c of choices.slice(0, 4)) { const b = h("button", { class: "chip-btn", type: "button" }, c); b.addEventListener("click", () => choose(c)); row.appendChild(b); }
    box.appendChild(row);
  }
  box.scrollTop = box.scrollHeight;
}
function element(it) {
  const m = it.m;
  if (it.building) {
    const c = card("FABRICATOR · RUST ON THIS DEVICE");
    c.appendChild(lastBuilt ? holoBox(Promise.resolve(lastBuilt), { mode: "build" }, "MATERIALIZING · ANY PART MADE INSPECTS") : h("div", { class: "holo wait" }, "BUILDING THE MESH"));
    return c;
  }
  if (m.role === "user") {
    const b = h("div", { class: "msg user" });
    if (m.text) b.appendChild(h("p", null, m.text.replace(/\[The user tapped photo (\d+) at x=\d+, y=\d+\]/, "◎ tapped photo $1").replace(/\[The user marked a box on photo (\d+) from [^\]]*\]/, "▢ marked photo $1")));
    if ((m.photos || []).length) { const st = h("div", { class: "strip" }); for (const n of m.photos) st.appendChild(shot(n, [], "thumb")); b.appendChild(st); }
    return b;
  }
  if (m.role === "assistant") {
    const lines = String(m.text).split("\n");
    const ci = lines.findIndex((l) => /^\s*Choices:/i.test(l));
    const choices = ci >= 0 ? lines[ci].replace(/^\s*Choices:\s*/i, "").split("|").map((x) => x.trim()).filter(Boolean) : [];
    if (ci >= 0) lines.splice(ci, 1);
    const b = h("div", { class: "msg muse" });
    b.appendChild(h("div", { class: "who" }, "MUSE SPARK"));
    b.appendChild(h("p", null, lines.join("\n").trim()));
    if (it.choices && choices.length) {
      const row = h("div", { class: "chips" });
      for (const c of choices.slice(0, 4)) { const btn = h("button", { class: "chip-btn", type: "button" }, c); btn.addEventListener("click", () => send(c)); row.appendChild(btn); }
      b.appendChild(row);
    }
    return b;
  }
  const res = it.res || {};
  if (m.name === "measure_photo") return measureCard(m.args || {}, res, it.latest);
  if (m.name === "measure_object") return objectCard(m.args || {}, res, it.latest);
  if (m.name === "set_dimension") {
    const c = card(res.error ? "NOT LOCKED" : "LOCKED · " + (NAMES[res.name] || res.name) + " " + res.value + " mm", res.error ? "bad" : "dim");
    c.appendChild(h("p", { class: "src" }, res.error || ("source " + res.source)));
    return c;
  }
  if (m.name === "build_and_check") return roundCard(res, it.latest);
  if (m.name === "quote") return quoteCard(res);
  if (m.name === "order_link") return orderCard(res);
  if (m.name === "order_status") return statusCard(res);
  return h("div");
}
let knownDims = new Set();
function render(s) {
  current = s;
  renderStage(s);
  renderDialog(s);
  const log = $("log"), list = items(s), keep = new Set();
  let prev = null;
  for (const it of list) {
    keep.add(it.key);
    let cur = shown.get(it.key);
    if (!cur || cur.sig !== it.sig) {
      const el = element(it);
      if (cur) cur.el.replaceWith(el); else if (prev) prev.after(el); else log.prepend(el);
      cur = { sig: it.sig, el };
      shown.set(it.key, cur);
    }
    prev = cur.el;
  }
  for (const [k, v] of shown) if (!keep.has(k)) { v.el.remove(); shown.delete(k); }
  const dims = $("dims");
  dims.replaceChildren();
  for (const k in s.dims || {}) {
    const d = s.dims[k];
    const chip = h("span", { class: "fact" + (knownDims.has(k) ? "" : " new") });
    chip.append(h("b", null, NAMES[k] || k), document.createTextNode(" " + d.value + " mm"), h("i", null, d.source.startsWith("user") ? "you" : d.source.split(".")[0]));
    dims.appendChild(chip);
  }
  knownDims = new Set(Object.keys(s.dims || {}));
  dims.hidden = !knownDims.size;
  if (!busy) $("status").textContent = s.agent === "working" ? "MUSE SPARK IS THINKING" : s.agent === "building" ? "FABRICATING ON THIS DEVICE" : "";
  $("log-btn").textContent = "LOG · " + list.length;
}

function showError(msg) { $("error").textContent = msg || ""; $("error").hidden = !msg; }
async function api(path, opts) {
  const r = await fetch(path, opts);
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error((j.error && j.error.message) || "Something went wrong. Try again.");
  return j;
}
async function upload(file) {
  const fd = new FormData();
  fd.append("photo", await shrink(file), "photo.jpg");
  return api("/api/session/" + current.id + "/photo", { method: "POST", body: fd });
}

// The part is built here, on this device: the same Rust geometry as WebAssembly.
let mesher = null, lastBuilt = null;
async function buildHere(b) {
  if (!mesher) {
    const bytes = await (await fetch("/mesher.wasm")).arrayBuffer();
    mesher = (await WebAssembly.instantiate(bytes, {})).instance.exports;
  }
  const input = new TextEncoder().encode(JSON.stringify(b));
  const p = mesher.alloc(input.length);
  new Uint8Array(mesher.memory.buffer, p, input.length).set(input);
  const n = mesher.build(p, input.length);
  const out = (i) => new Uint8Array(mesher.memory.buffer, mesher.out_ptr(i), mesher.out_len(i)).slice();
  const meta = JSON.parse(new TextDecoder().decode(out(0)));
  if (n !== 3) {
    const fd = new FormData();
    fd.append("error", meta.error || "unknown error");
    return api("/api/session/" + current.id + "/build", { method: "POST", body: fd });
  }
  const file = out(1), pic = out(2);
  lastBuilt = await read3mf(file.buffer);
  stageKey = null;
  render(current);
  const fd = new FormData();
  fd.append("model", new Blob([file], { type: "model/3mf" }), "part.3mf");
  fd.append("picture", new Blob([pic], { type: "image/png" }), "part.png");
  return api("/api/session/" + current.id + "/build", { method: "POST", body: fd });
}
async function run(body) {
  let s = await api("/api/session/" + current.id + "/agent", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  render(s);
  let guard = 0;
  while ((s.agent === "working" || s.agent === "building") && guard++ < 24) {
    if (s.agent === "building" && s.build) s = await buildHere(s.build);
    else s = await api("/api/session/" + current.id + "/agent", { method: "POST", headers: { "content-type": "application/json" }, body: "{}" });
    render(s);
  }
}
async function send(text, tap) {
  if (busy) return;
  text = (text || "").trim();
  if (!current && !pending.length) { showError("Add a photo first: what the part must fit, with a ruler or coin next to it."); return; }
  if (!text && !pending.length && !tap) return;
  busy = true; $("send").disabled = true; showError("");
  document.body.classList.add("busy");
  // Show the answer at once, and a running timer while Muse works.
  const said = text || (tap ? (tap.x2 !== undefined ? "▢ marked photo " + tap.photo : "◎ tapped photo " + tap.photo) : "photo sent");
  const dlg = $("dialog");
  dlg.replaceChildren(h("div", { class: "who" }, "YOU"), h("p", { class: "line" }, said));
  const t0 = Date.now();
  const timer = setInterval(() => {
    const label = current && current.agent === "building" ? "FABRICATING ON THIS DEVICE" : "MUSE SPARK IS THINKING";
    $("status").textContent = label + " · " + Math.round((Date.now() - t0) / 1000) + " S";
  }, 500);
  try {
    const added = [];
    if (!current) {
      $("status").textContent = "UPLOADING";
      const fd = new FormData();
      fd.append("photo", await shrink(pending[0]), "photo.jpg");
      fd.append("line", text.length >= 3 ? text : "replacement part");
      fd.append("mode", "agent");
      current = await api("/api/session", { method: "POST", body: fd });
      history.replaceState(null, "", "#s=" + current.id);
      added.push(1);
      localPhotos[1] = URL.createObjectURL(pending[0]);
      for (const f of pending.slice(1)) { current = await upload(f); added.push(current.photos.length); localPhotos[current.photos.length] = URL.createObjectURL(f); }
    } else {
      for (const f of pending) { $("status").textContent = "UPLOADING"; current = await upload(f); added.push(current.photos.length); localPhotos[current.photos.length] = URL.createObjectURL(f); }
    }
    pending = []; renderPending(); $("text").value = "";
    await run({ text: text, photos: added, tap: tap || null });
  } catch (e) { showError(e.message); }
  finally { clearInterval(timer); busy = false; document.body.classList.remove("busy"); $("send").disabled = false; $("text").placeholder = "Say what you need"; if (current) render(current); }
}
function renderPending() {
  const box = $("pending");
  box.replaceChildren();
  for (const f of pending) box.appendChild(h("img", { src: URL.createObjectURL(f), alt: "New photo" }));
  box.hidden = !pending.length;
}
for (const id of ["cam", "file"]) $(id).addEventListener("change", (ev) => { for (const f of ev.target.files || []) pending.push(f); ev.target.value = ""; renderPending(); });
$("composer").addEventListener("submit", (ev) => { ev.preventDefault(); send($("text").value); });
function wireStart() {
  for (const b of document.querySelectorAll("#stage [data-start]")) b.onclick = () => {
    $("text").value = b.dataset.start;
    const hint = document.querySelector("#stage .hint"); if (hint) hint.textContent = b.dataset.hint;
    $("text").focus();
  };
}
wireStart();
$("log-btn").addEventListener("click", () => { $("drawer").hidden = false; $("log").lastElementChild && $("log").lastElementChild.scrollIntoView(); });
$("drawer-close").addEventListener("click", () => { $("drawer").hidden = true; });

// Full screen photo: one tap marks a point, a second tap the opposite corner of a box.
let marks = [], viewMode = "object";
function openViewer(n, mode) {
  viewing = n; marks = []; viewMode = mode || "object";
  $("v-ruler").hidden = true;
  const p = current.photos[n - 1];
  const img = $("v-img");
  img.src = photoUrl(n); img.width = p.width; img.height = p.height;
  $("v-title").textContent = viewMode === "ruler" ? "PHOTO " + n + " · tap two marks on the ruler, as far apart as you can, for example 0 and 5 cm."
    : "PHOTO " + n + " · tap the thing to measure. For a board, tap two opposite corners.";
  $("v-point").hidden = true; $("v-marks").replaceChildren(); $("viewer").hidden = false;
}
function drawMarks() {
  const box = $("v-marks"); box.replaceChildren();
  const p = current.photos[viewing - 1];
  for (const m of marks) box.appendChild(h("span", { class: "mark", style: "left:" + (m[0] / p.width * 100) + "%;top:" + (m[1] / p.height * 100) + "%" }));
}
$("v-img").addEventListener("click", (ev) => {
  const p = current.photos[viewing - 1], r = ev.target.getBoundingClientRect();
  marks.push([Math.round((ev.clientX - r.left) * p.width / r.width), Math.round((ev.clientY - r.top) * p.height / r.height)]);
  drawMarks();
  if (viewMode === "ruler") {
    if (marks.length > 2) marks.splice(0, marks.length - 2);
    drawMarks();
    $("v-ruler").hidden = marks.length !== 2;
    return;
  }
  if (marks.length === 1) { $("v-point").hidden = false; return; }
  const [a, b] = marks;
  $("viewer").hidden = true;
  send($("text").value, { photo: viewing, x: a[0], y: a[1], x2: b[0], y2: b[1] });
});
$("v-point").addEventListener("click", () => { const a = marks[0]; $("viewer").hidden = true; send($("text").value, { photo: viewing, x: a[0], y: a[1] }); });
$("v-use").addEventListener("click", () => {
  const mm = parseFloat($("v-mm").value.replace(",", "."));
  if (!(mm > 0)) { $("v-mm").focus(); return; }
  const [a, b] = marks;
  $("viewer").hidden = true;
  send("I marked the ruler.", { photo: viewing, x: a[0], y: a[1], x2: b[0], y2: b[1], kind: "ruler", mm: mm });
});
$("v-close").addEventListener("click", () => { $("viewer").hidden = true; });

(async () => {
  const m = location.hash.match(/^#s=([0-9a-f]{32})$/);
  if (!m) return;
  try {
    current = await api("/api/session/" + m[1]); render(current);
    if (current.agent === "building" && current.build) { render(await buildHere(current.build)); await run({}); }
    else if (current.agent === "working") await run({});
  } catch (e) { showError(e.message); }
})();
"##;

const STYLE: &str = r#"
:root { color-scheme: dark; --bg: #04070d; --panel: rgba(16, 28, 44, 0.72); --line: rgba(56, 232, 255, 0.22);
  --ink: #dff6ff; --muted: #7c95a8; --cyan: #38e8ff; --ok: #5dffa0; --bad: #ff5468; --warn: #ffb020; }
* { box-sizing: border-box; }
body { margin: 0; color: var(--ink); background: radial-gradient(1200px 700px at 50% -10%, #0d2a44 0%, var(--bg) 60%) fixed, var(--bg);
  font: 15px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; }
body::before { content: ""; position: fixed; inset: 0; pointer-events: none; opacity: 0.35;
  background: repeating-linear-gradient(0deg, rgba(56,232,255,0.04) 0 1px, transparent 1px 3px); }
#app { height: 100vh; height: 100dvh; display: grid; grid-template-rows: auto minmax(0, 1fr) auto; grid-template-columns: minmax(0, 1fr);
  width: 100%; max-width: 720px; margin: 0 auto; position: relative; }
#start, #start2 { justify-content: center; text-align: center; position: relative; overflow: hidden; }
#start::before, #start2::before { content: ""; position: absolute; left: 50%; top: 50%; width: 260px; height: 260px; margin: -130px 0 0 -130px;
  border-radius: 50%; border: 1px solid rgba(56,232,255,0.35); box-shadow: 0 0 40px rgba(56,232,255,0.25), inset 0 0 40px rgba(56,232,255,0.15);
  animation: ring 3s ease-in-out infinite; pointer-events: none; }
@keyframes ring { 0%, 100% { transform: scale(0.85); opacity: 0.5; } 50% { transform: scale(1.1); opacity: 1; } }
#start .starts, #start2 .starts { position: relative; }
#hud { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 10px; padding: 8px 12px 4px; padding-top: max(8px, env(safe-area-inset-top)); }
.brand { font-size: 11px; letter-spacing: 0.22em; color: var(--cyan); text-shadow: 0 0 14px rgba(56,232,255,0.6); flex: 1; font-weight: 700; }
.log-btn { font: 600 11px ui-monospace, Menlo, monospace; letter-spacing: 0.14em; color: var(--muted); background: transparent;
  border: 1px solid var(--line); border-radius: 999px; padding: 4px 10px; cursor: pointer; }
#stage { overflow: hidden; padding: 0 8px; display: flex; flex-direction: column; min-height: 0; }
#stage > .scene { flex: 1; display: flex; flex-direction: column; margin: 2px 0 4px; animation: rise 0.45s ease-out; min-height: 0;
  overflow: auto; border: none; background: transparent; padding: 0 4px; backdrop-filter: none; }
#stage > .scene > .tt { margin: 2px 4px 4px; }
#stage .holo { flex: 1 0 58%; min-height: 240px; height: auto; margin: 0 0 6px; border-radius: 14px; }
#stage .shot.small { margin: 0 auto 6px; }
#stage > .scene > :not(.holo):not(.tt):not(.shot) { flex-shrink: 0; }
#dock { padding: 6px 12px 8px; padding-bottom: max(8px, env(safe-area-inset-bottom)); background: linear-gradient(0deg, rgba(4,7,13,0.98) 80%, rgba(4,7,13,0)); }
#dialog { border: 1px solid rgba(56,232,255,0.45); border-radius: 14px; padding: 8px 12px; background: rgba(8,20,32,0.92);
  box-shadow: 0 0 24px rgba(56,232,255,0.15), inset 0 0 20px rgba(56,232,255,0.06); max-height: 26vh; max-height: 26dvh; overflow: auto; font-size: 15px; }
#dialog .who { margin-bottom: 2px; font-size: 10px; }
#dialog .line { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
#dialog .line.dimmed { opacity: 0.55; }
#dialog .line.clamp { display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; cursor: pointer; }
#dialog .approve { display: flex; width: 100%; margin-top: 10px; }
#drawer { position: fixed; inset: 0; z-index: 9; background: rgba(2,6,12,0.96); overflow: auto; padding: 12px 16px 24px; }
.drawer-head { position: sticky; top: 0; display: flex; justify-content: space-between; align-items: center; padding: 6px 0;
  background: rgba(2,6,12,0.96); color: var(--cyan); letter-spacing: 0.2em; font-size: 12px; font-weight: 700; z-index: 1; }
#log { max-width: 680px; margin: 0 auto; }
h1 { font-size: 22px; margin: 6px 0 2px; letter-spacing: 0.12em; text-transform: uppercase; color: var(--cyan);
  text-shadow: 0 0 18px rgba(56,232,255,0.55); }
.lead { color: var(--muted); margin: 0 0 14px; font-size: 14px; }
.card, .tool, .msg, .round-line { background: var(--panel); border: 1px solid var(--line); border-radius: 14px; backdrop-filter: blur(6px); }
.card { padding: 14px; margin-bottom: 12px; }
h2 { font-size: 13px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--cyan); margin: 0 0 8px; }
.hint { color: var(--muted); font-size: 14px; margin: 6px 0 0; }
.starts { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-top: 10px; }
.btn, .chip-btn, .opt { font: inherit; cursor: pointer; color: var(--ink); }
.btn { display: inline-flex; align-items: center; justify-content: center; min-height: 44px; padding: 0 14px; border-radius: 10px;
  border: 1px solid var(--line); background: rgba(56,232,255,0.06); font-weight: 600; letter-spacing: 0.04em; }
.btn.primary { background: linear-gradient(180deg, #2bd6f0, #0a9fc0); color: #00131b; border-color: transparent; box-shadow: 0 0 22px rgba(56,232,255,0.35); }
.btn:disabled { opacity: 0.5; }
.msg { padding: 10px 14px; margin: 10px 0; max-width: 92%; white-space: pre-wrap; overflow-wrap: anywhere; animation: rise 0.35s ease-out; }
.msg p { margin: 0; }
.msg.user { margin-left: auto; background: rgba(56,232,255,0.14); border-color: rgba(56,232,255,0.45); }
.who { font-size: 11px; letter-spacing: 0.16em; color: var(--cyan); margin-bottom: 4px; }
@keyframes rise { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }
.tool { padding: 10px 12px; margin: 10px 0; font-size: 14px; overflow-wrap: anywhere; animation: rise 0.35s ease-out; border-left: 3px solid var(--cyan); }
.tool.good { border-left-color: var(--ok); }
.tool.bad { border-left-color: var(--bad); }
.tool.dim { border-left-color: #a58bff; }
.tt { font-size: 11px; letter-spacing: 0.14em; color: var(--cyan); margin-bottom: 6px; font-weight: 700; }
.tool.good .tt { color: var(--ok); }
.tool.bad .tt { color: var(--bad); }
.tool.dim .tt { color: #c4b2ff; }
.src { color: var(--muted); font-size: 12px; margin: 0; font-family: ui-monospace, Menlo, monospace; }
.strip { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 8px; }
.shot { position: relative; cursor: pointer; overflow: hidden; border-radius: 10px; }
.shot img { display: block; width: 100%; height: auto; }
.shot:not(.ready) { background: linear-gradient(90deg, rgba(56,232,255,0.05), rgba(56,232,255,0.15), rgba(56,232,255,0.05)); background-size: 200% 100%; animation: shimmer 1.2s linear infinite; }
@keyframes shimmer { to { background-position: -200% 0; } }
.shot:not(.ready)::after, .shot:not(.ready) .draw { animation-play-state: paused; }
.shot.thumb { width: 84px; }
.shot.small { width: 100%; max-width: 420px; margin: 4px 0 8px; }
.shot .pn { position: absolute; left: 6px; top: 6px; background: rgba(0,0,0,0.6); color: var(--cyan); font-size: 10px; letter-spacing: 0.12em;
  border-radius: 999px; padding: 1px 8px; }
.shot.scanning::after { content: ""; position: absolute; left: 0; right: 0; height: 30%; top: -30%;
  background: linear-gradient(180deg, transparent, rgba(56,232,255,0.35) 70%, rgba(200,255,255,0.9) 100%); animation: beam 1.6s ease-in-out 1 forwards; }
@keyframes beam { to { top: 100%; } }
.overlay { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
.draw { stroke-dasharray: 1; stroke-dashoffset: 1; animation: draw 1.4s 1.2s ease-out forwards; }
@keyframes draw { to { stroke-dashoffset: 0; } }
.readout .big { font-family: ui-monospace, Menlo, monospace; font-size: 22px; color: #fff; text-shadow: 0 0 12px rgba(56,232,255,0.6); white-space: pre; }
.readout .big.dim { color: var(--muted); font-size: 15px; text-shadow: none; }
.readout .small { font-family: ui-monospace, Menlo, monospace; font-size: 12px; color: var(--muted); }
.chips { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 10px; }
.chip-btn { border: 1px solid rgba(56,232,255,0.6); background: rgba(56,232,255,0.12); border-radius: 999px; padding: 7px 14px; font-size: 14px; }
.chip-btn.ghost { border-color: var(--line); background: transparent; color: var(--muted); }
.chip-btn:active, .opt:active { transform: scale(0.97); background: rgba(56,232,255,0.3); }
body.busy .chip-btn, body.busy .opt { opacity: 0.4; pointer-events: none; }
.holo { position: relative; height: 300px; margin: 4px 0 8px; border-radius: 12px; overflow: hidden;
  background: radial-gradient(circle at 50% 60%, rgba(56,232,255,0.16), transparent 65%), linear-gradient(180deg, rgba(0,0,0,0.2), rgba(0,0,0,0.5));
  border: 1px solid var(--line); }
.holo.wait { display: grid; place-items: center; color: var(--muted); font-size: 12px; letter-spacing: 0.14em; }
canvas.gl { width: 100%; height: 100%; display: block; touch-action: none; }
.lbl { position: absolute; left: 0; top: 0; font: 600 11px ui-monospace, Menlo, monospace; color: #fff; pointer-events: none;
  text-shadow: 0 0 6px rgba(0,0,0,0.9), 0 0 10px rgba(56,232,255,0.6); white-space: nowrap; transition: opacity 0.4s; }
.lbl.ax.x { color: #ff6b6b; } .lbl.ax.y { color: #6bff7a; } .lbl.ax.z { color: #7aa2ff; }
.legend { position: absolute; left: 10px; top: 24px; font: 10px ui-monospace, Menlo, monospace; letter-spacing: 0.12em; color: var(--muted); }
.hud { position: absolute; left: 10px; top: 8px; font-size: 11px; letter-spacing: 0.14em; color: var(--cyan); font-family: ui-monospace, Menlo, monospace; }
.tool.good .hud { color: var(--ok); }
img.knob { display: block; width: 220px; height: 220px; margin: 6px auto; border-radius: 10px; }
img.thumb3d { width: 96px; height: 96px; border-radius: 8px; float: right; margin: 0 0 6px 8px; }
.muse { margin: 4px 0; color: #c9ecff; }
.changes { margin: 4px 0; padding-left: 18px; color: var(--cyan); font-family: ui-monospace, Menlo, monospace; font-size: 12px; }
.issues { margin: 4px 0; padding-left: 18px; font-size: 13px; }
.issues li { margin-bottom: 4px; }
.issues li.error { color: var(--bad); }
.issues li.warning { color: var(--warn); }
.fix { color: var(--muted); }
.cert { border: 1px solid rgba(93,255,160,0.5); background: rgba(93,255,160,0.07); border-radius: 10px; padding: 8px 10px; margin: 8px 0;
  font-family: ui-monospace, Menlo, monospace; font-size: 13px; box-shadow: 0 0 22px rgba(93,255,160,0.18); }
.more summary { cursor: pointer; font: 12px ui-monospace, Menlo, monospace; color: var(--muted); letter-spacing: 0.06em; padding: 2px; }
.more[open] summary { color: var(--cyan); }
.dl { color: var(--cyan); font-size: 12px; letter-spacing: 0.1em; }
.round-line { margin: 8px 0; padding: 8px 12px; font-size: 13px; }
.round-line summary { cursor: pointer; font-family: ui-monospace, Menlo, monospace; letter-spacing: 0.06em; color: var(--warn); }
.round-line.good summary { color: var(--ok); }
.opt { display: grid; grid-template-columns: 1fr auto; gap: 2px 12px; width: 100%; text-align: left; padding: 10px 12px; margin: 6px 0;
  border: 1px solid var(--line); border-radius: 10px; background: rgba(56,232,255,0.05); }
.opt:hover { border-color: var(--cyan); }
.opt .price { font-weight: 700; grid-row: span 2; align-self: center; font-size: 20px; color: #fff; }
.opt .when { color: var(--muted); font-size: 12px; font-family: ui-monospace, Menlo, monospace; }
.note { color: var(--muted); font-size: 12px; margin: 4px 2px 0; }
.opts3 { display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px; }
.opt3 { display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 10px 4px; border-radius: 12px; cursor: pointer;
  border: 1px solid rgba(56,232,255,0.45); background: rgba(56,232,255,0.07); color: var(--ink); font: inherit; }
.opt3:active { transform: scale(0.97); background: rgba(56,232,255,0.25); }
.opt3 .lead { font-size: 11px; letter-spacing: 0.12em; text-transform: uppercase; color: var(--cyan); }
.opt3 .price { font-size: 22px; font-weight: 700; color: #fff; }
.opt3 .when { font: 11px ui-monospace, Menlo, monospace; color: var(--muted); }
body.busy .opt3 { opacity: 0.4; pointer-events: none; }
#stage .holo.compact { flex: 1 1 auto; min-height: 150px; }
a.btn { text-decoration: none; }
#dims { display: flex; gap: 6px; flex-wrap: nowrap; width: 100%; overflow-x: auto; scrollbar-width: none; }
#dims::-webkit-scrollbar { display: none; }
#dims .fact { flex: 0 0 auto; padding: 3px 8px; font-size: 11px; }
.fact { font-family: ui-monospace, Menlo, monospace; font-size: 12px; padding: 4px 10px; border-radius: 8px; border: 1px solid rgba(165,139,255,0.5);
  background: rgba(165,139,255,0.12); color: #e9e2ff; }
.fact b { color: #c4b2ff; font-weight: 600; margin-right: 2px; }
.fact i { font-style: normal; color: var(--muted); margin-left: 6px; font-size: 10px; }
.fact.new { animation: lock 1.2s ease-out; }
@keyframes lock { 0% { box-shadow: 0 0 0 rgba(165,139,255,0); transform: scale(1.15); } 40% { box-shadow: 0 0 24px rgba(165,139,255,0.9); } 100% { transform: none; } }
.error { color: var(--bad); font-weight: 600; }
.status { color: var(--cyan); font-size: 11px; letter-spacing: 0.16em; min-height: 0; margin: 6px 0 0; font-family: ui-monospace, Menlo, monospace; }
.status:not(:empty)::before { content: ""; display: inline-block; width: 10px; height: 10px; margin-right: 8px; border-radius: 50%;
  background: var(--cyan); box-shadow: 0 0 12px var(--cyan); animation: pulse 1s ease-in-out infinite; }
@keyframes pulse { 50% { opacity: 0.25; } }
#composer { margin-top: 8px; }
#composer { margin-top: 6px; }
#composer .row { display: flex; gap: 6px; align-items: stretch; }
#composer .btn { min-height: 42px; }
#composer input[type=text] { flex: 1; min-width: 0; min-height: 42px; padding: 0 12px; border: 1px solid var(--line); border-radius: 10px;
  font: inherit; background: rgba(8,16,26,0.9); color: var(--ink); }
#composer .btn.icon { padding: 0 10px; font: 600 11px ui-monospace, Menlo, monospace; letter-spacing: 0.1em; }
#composer input[type=file] { position: absolute; width: 1px; height: 1px; opacity: 0; }
#pending { display: flex; gap: 6px; margin-top: 8px; }
#pending img { width: 64px; height: 64px; object-fit: cover; border-radius: 8px; border: 1px solid var(--line); }
#viewer { position: fixed; inset: 0; background: rgba(2,6,12,0.94); z-index: 10; overflow: auto; padding: 12px; }
#viewer p { color: var(--cyan); margin: 4px 0 10px; font-size: 12px; letter-spacing: 0.1em; }
#viewer img { display: block; width: 100%; height: auto; cursor: crosshair; border-radius: 8px; }
#viewer .btn { margin: 10px 8px 0 0; }
.vwrap { position: relative; }
.v-ruler { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin-top: 10px; color: var(--ink); font-size: 14px; }
.v-ruler label { width: 100%; }
.v-ruler input { width: 90px; min-height: 42px; padding: 0 10px; border-radius: 10px; border: 1px solid var(--line); background: rgba(8,16,26,0.9); color: var(--ink); font: inherit; }
#v-marks { position: absolute; inset: 0; pointer-events: none; }
.mark { position: absolute; width: 22px; height: 22px; margin: -11px 0 0 -11px; border: 2px solid var(--cyan); border-radius: 50%;
  box-shadow: 0 0 14px var(--cyan); }
footer { color: var(--muted); font-size: 12px; text-align: center; margin-top: 18px; }
footer a { color: inherit; }
@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; } .draw { stroke-dashoffset: 0; } }
"#;

pub fn html() -> String {
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>Any Part Made Studio</title>
<link rel="icon" href="data:,">
<style>{STYLE}</style>
</head>
<body>
<div id="app">
<header id="hud">
<div class="brand">ANY PART MADE STUDIO</div>
<button id="log-btn" class="log-btn" type="button">LOG</button>
<div id="dims" hidden></div>
</header>
<section id="stage" aria-live="polite">
<div class="card scene" id="start">
<h2>Start</h2>
<p class="hint">Take a photo with a ruler or a coin right next to what the part must fit, then say what you need. Muse Spark measures with SAM 3.1, designs the part, and Any Part Made inspects, prices and guarantees it.</p>
<div class="starts">
<button class="btn" type="button" data-start="My oven is missing a knob. Here is the empty shaft with a ruler next to it." data-hint="Photo straight at the shaft end, ruler right beside it. A second photo from the side helps.">Replace a knob</button>
<button class="btn" type="button" data-start="I need a case for this board." data-hint="Board flat on the table, ruler touching its long edge, photo from straight above.">Case for a board</button>
</div>
</div>
</section>
<section id="dock">
<div id="dialog"><div class="who">MUSE SPARK</div><p class="line">Show me what the part must fit. A photo with a ruler or a coin right next to it works best.</p></div>
<p id="error" class="error" role="alert" hidden></p>
<p id="status" class="status" aria-live="polite"></p>
<form id="composer">
<div id="pending" hidden></div>
<div class="row">
<label class="btn icon" title="Camera">CAM<input id="cam" type="file" accept="image/*" capture="environment"></label>
<label class="btn icon" title="Photos">IMG<input id="file" type="file" accept="image/*" multiple></label>
<input id="text" type="text" placeholder="Say what you need" maxlength="2000" autocomplete="off">
<button id="send" class="btn primary" type="submit">Send</button>
</div>
</form>
</section>
</div>
<template id="start-tpl"><div class="card" id="start2">
<h2>Start</h2>
<p class="hint">Take a photo with a ruler or a coin right next to what the part must fit, then say what you need.</p>
<div class="starts">
<button class="btn" type="button" data-start="My oven is missing a knob. Here is the empty shaft with a ruler next to it." data-hint="Photo straight at the shaft end, ruler right beside it. A second photo from the side helps.">Replace a knob</button>
<button class="btn" type="button" data-start="I need a case for this board." data-hint="Board flat on the table, ruler touching its long edge, photo from straight above.">Case for a board</button>
</div>
</div></template>
<div id="drawer" hidden>
<div class="drawer-head"><span>MISSION LOG</span><button id="drawer-close" class="btn" type="button">Close</button></div>
<div id="log"></div>
<footer>Built for the Meta Global AI Developer Hackathon. Inspection by <a href="https://anypartmade.com">Any Part Made</a>. Photos are deleted after 7 days. <a href="/guided">Step by step mode</a></footer>
</div>
<div id="viewer" hidden>
<p id="v-title"></p>
<div class="vwrap"><img id="v-img" alt="Tap what to measure"><div id="v-marks"></div></div>
<button id="v-point" class="btn primary" type="button" hidden>Use this point</button>
<div id="v-ruler" class="v-ruler" hidden><label for="v-mm">Distance between the two marks, mm</label>
<input id="v-mm" type="text" inputmode="decimal" value="50"><button id="v-use" class="btn primary" type="button">Use these marks</button></div>
<button id="v-close" class="btn" type="button">Close</button>
</div>
<script>{SCRIPT}</script>
</body>
</html>"##
    )
}

pub fn csp() -> String {
    crate::page::csp_for(SCRIPT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_page_is_self_contained() {
        let h = html();
        assert!(h.contains(r#"capture="environment""#));
        assert!(!h
            .replace("http://www.w3.org/2000/svg", "")
            .contains("http://"));
        assert!(!h.contains("<script src"));
        assert!(!h.contains('\u{2013}') && !h.contains('\u{2014}'));
        assert!(h.contains(&format!("<script>{SCRIPT}</script>")));
        assert!(csp().contains("script-src 'sha256-"));
    }
}
