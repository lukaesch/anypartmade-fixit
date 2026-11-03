//! The agent page: a chat with Muse Spark. The user sends photos and words and taps
//! points on photos; every measurement, check round and quote shows as a card.

const SCRIPT: &str = r##"
const $ = (id) => document.getElementById(id);
const MAX_EDGE = 2048;
let current = null;
let pending = [];
let busy = false;
let viewing = null;
const COLORS = { coin: "#d18f00", shaft: "#0b6bcb", knob: "#1d7a46", ruler: "#c2185b" };
const VERDICT = { ok: "ok, ready to quote", needs_changes: "needs changes", needs_review: "a person reviews it",
  refused: "refused", needs_upload: "waiting for the file" };
const LEAD = { opt_express: "Express", opt_standard: "Standard", opt_economy: "Economy" };
const NAMES = { shaft_diameter_mm: "Shaft diameter", flat_to_round_mm: "Across the flat", shaft_length_mm: "Shaft length",
  board_length_mm: "Board length", board_width_mm: "Board width", board_height_mm: "Board height" };

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
  try {
    bmp = await createImageBitmap(file, { imageOrientation: "from-image" });
  } catch (e) {
    bmp = await new Promise((ok, fail) => {
      const i = new Image();
      i.onload = () => ok(i);
      i.onerror = () => fail(new Error("This photo format cannot be read here. Try JPEG."));
      i.src = URL.createObjectURL(file);
    });
  }
  const k = Math.min(1, MAX_EDGE / Math.max(bmp.width, bmp.height));
  const c = document.createElement("canvas");
  c.width = Math.round(bmp.width * k);
  c.height = Math.round(bmp.height * k);
  c.getContext("2d").drawImage(bmp, 0, 0, c.width, c.height);
  return await new Promise((ok) => c.toBlob(ok, "image/jpeg", 0.9));
}

function photoUrl(n) { return "/api/session/" + current.id + "/photo/" + n; }

// A photo with the outlines of one measurement drawn over it.
function shot(n, overlays, cls) {
  const p = current.photos[n - 1];
  const wrap = h("div", { class: "shot " + (cls || "") });
  const img = h("img", { src: photoUrl(n), alt: "Photo " + n, width: p.width, height: p.height });
  wrap.appendChild(img);
  if (overlays && overlays.length) {
    const s = svg("svg", { viewBox: "0 0 " + p.width + " " + p.height, class: "overlay" });
    const fs = Math.round(Math.max(p.width, p.height) / 40), lw = Math.max(2, fs / 7);
    for (const o of overlays) {
      const c = COLORS[o.label] || "#000";
      if (o.line) {
        s.appendChild(svg("line", { x1: o.line[0][0], y1: o.line[0][1], x2: o.line[1][0], y2: o.line[1][1], stroke: c, "stroke-width": lw }));
        continue;
      }
      if (o.outline) s.appendChild(svg("polygon", { points: o.outline.map((q) => q.join(",")).join(" "),
        fill: c, "fill-opacity": "0.25", stroke: c, "stroke-width": lw }));
    }
    wrap.appendChild(s);
  }
  wrap.appendChild(h("span", { class: "pn" }, String(n)));
  wrap.addEventListener("click", () => openViewer(n));
  return wrap;
}

function card(title, cls) {
  const c = h("div", { class: "tool " + (cls || "") });
  c.appendChild(h("div", { class: "tt" }, title));
  return c;
}

function measureCard(args, res) {
  if (res.error) { const c = card("Measuring photo " + args.photo, "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const r = res.result || {}, sc = r.scale || {}, sh = r.shaft || {};
  const c = card("SAM 3.1 measured photo " + res.photo + " (" + res.id + ")");
  const m = (current.measurements || []).find((x) => x.id === res.id);
  c.appendChild(shot(res.photo, m ? m.overlays : [], "small"));
  const ul = h("ul");
  if (sc.px_per_mm) ul.appendChild(h("li", null, "Scale " + sc.px_per_mm.toFixed(2) + " px per mm from the " +
    (sc.source === "coin" ? "coin" : "ruler ticks") + " (" + pct(sc.confidence) + ")"));
  else ul.appendChild(h("li", null, "No scale found"));
  if (sh.diameter_mm) ul.appendChild(h("li", null, "Shaft " + sh.diameter_mm.toFixed(2) + " mm" +
    (sh.view === "side" ? " across, side view" + (sh.length_mm ? ", " + sh.length_mm.toFixed(1) + " mm long" : "") :
      (sh.flat_to_round_mm ? ", flat " + sh.flat_to_round_mm.toFixed(2) + " mm" : "")) + " (" + pct(sh.confidence) + ")"));
  else ul.appendChild(h("li", null, "No shaft found"));
  c.appendChild(ul);
  return c;
}

function objectCard(args, res) {
  if (res.error) { const c = card("Measuring photo " + args.photo, "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const r = res.result || {};
  const c = card("SAM 3.1 measured " + (args.object || "the tapped object") + " in photo " + res.photo + " (" + res.id + ")");
  const m = (current.measurements || []).find((x) => x.id === res.id);
  c.appendChild(shot(res.photo, m ? m.overlays : [], "small"));
  const ul = h("ul");
  if (!r.found) ul.appendChild(h("li", null, r.note || "Nothing found"));
  else {
    if (r.length_mm) ul.appendChild(h("li", null, r.length_mm.toFixed(2) + " x " + r.width_mm.toFixed(2) + " mm" +
      (r.round_diameter_mm && r.rectangle_fill < 0.85 ? ", round " + r.round_diameter_mm.toFixed(2) + " mm" : "")));
    if (r.scale && r.scale.px_per_mm) ul.appendChild(h("li", null, "Scale " + r.scale.px_per_mm.toFixed(2) + " px per mm (" + pct(r.scale.confidence) + ")"));
    else ul.appendChild(h("li", null, "No scale found"));
  }
  c.appendChild(ul);
  return c;
}

function roundCard(res) {
  if (res.error) { const c = card("Design check", "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const r = (current.rounds || [])[res.round - 1];
  const v = res.verdict;
  const c = card("Round " + res.round + ": Any Part Made says " + (VERDICT[v] || v), v === "ok" ? "good" : "");
  if (r) {
    c.appendChild(h("img", { class: "knob", src: r.picture_url, alt: "Knob of round " + r.n, width: "480", height: "480" }));
    const ch = r.changes || {};
    if (ch.summary) c.appendChild(h("p", { class: "muse" }, "Muse Spark: " + ch.summary));
    const changes = (ch.changes || []).filter((x) => x.from !== x.to);
    if (changes.length) {
      const ul = h("ul", { class: "changes" });
      for (const x of changes) ul.appendChild(h("li", null, x.field + ": " + x.from + " → " + x.to + ". " + x.why));
      c.appendChild(ul);
    }
  }
  if ((res.issues || []).length) {
    const ul = h("ul", { class: "issues" });
    for (const i of res.issues) {
      const li = h("li", { class: i.severity }, i.severity + " " + i.code + ": " + i.message);
      if (i.fix) li.appendChild(h("div", { class: "fix" }, "Fix: " + i.fix));
      ul.appendChild(li);
    }
    c.appendChild(ul);
  }
  if (v === "ok" && res.guarantee) c.appendChild(h("p", { class: "guar" }, "Guaranteed: " +
    res.guarantee.map((d) => d.label + " " + d.nominal_mm + " mm " + d.tolerance_mm).join(", ") +
    ((res.interfaces || []).length ? "; fits " + res.interfaces.join(", ") : "")));
  if (v === "refused" && res.refusal) c.appendChild(h("p", { class: "core" }, JSON.stringify(res.refusal)));
  if (r) c.appendChild(h("a", { href: r.model_url, download: "" }, "Download 3MF (" + Math.round(r.file_bytes / 1024) + " KB)"));
  return c;
}

function quoteCard(res) {
  if (res.error) { const c = card("Price", "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const c = card("Any Part Made: price and guarantee");
  for (const o of res.options || []) {
    const d = o.delivery_estimate || {};
    const b = h("button", { class: "opt", type: "button" });
    b.append(h("b", null, LEAD[o.option_id] || o.lead_time), h("span", { class: "price" }, o.price_display),
      h("span", { class: "when" }, "arrives " + d.earliest + " to " + d.latest));
    b.addEventListener("click", () => send("I take " + (LEAD[o.option_id] || o.lead_time) + " (" + o.option_id + ")."));
    c.appendChild(b);
  }
  for (const n of res.notes || []) c.appendChild(h("p", { class: "note" }, n));
  return c;
}

function orderCard(res) {
  if (res.error) { const c = card("Approval link", "bad"); c.appendChild(h("p", null, res.error)); return c; }
  const c = card("Approve on anypartmade.com", "good");
  c.appendChild(h("p", null, res.price + ". Nothing is ordered until you approve it there."));
  c.appendChild(h("a", { class: "btn primary", href: res.order_url, target: "_blank", rel: "noopener" }, "Review and approve"));
  return c;
}

function render(s) {
  current = s;
  $("start").hidden = true;
  const log = $("log");
  log.replaceChildren();
  const results = {};
  for (const m of s.chat || []) if (m.role === "tool_result") results[m.id] = m.result || {};
  for (const m of s.chat || []) {
    if (m.role === "user") {
      const b = h("div", { class: "msg user" });
      if (m.text) b.appendChild(h("p", null, m.text.replace(/\[The user tapped photo (\d+) at x=\d+, y=\d+\]/, "Tapped photo $1")
        .replace(/\[The user marked a box on photo (\d+) from [^\]]*\]/, "Marked a box on photo $1")));
      if ((m.photos || []).length) {
        const strip = h("div", { class: "strip" });
        for (const n of m.photos) strip.appendChild(shot(n, [], "thumb"));
        b.appendChild(strip);
      }
      log.appendChild(b);
    } else if (m.role === "assistant") {
      log.appendChild(h("div", { class: "msg muse" }, m.text));
    } else if (m.role === "tool_call") {
      const res = results[m.id] || {};
      if (m.name === "measure_photo") log.appendChild(measureCard(m.args || {}, res));
      else if (m.name === "measure_object") log.appendChild(objectCard(m.args || {}, res));
      else if (m.name === "set_dimension") {
        const c = card(res.error ? "Not recorded" : "Recorded: " + (NAMES[res.name] || res.name) + " " + res.value + " mm", res.error ? "bad" : "dim");
        c.appendChild(h("p", null, res.error || ("From " + res.source + ". " + ((m.args || {}).why || ""))));
        log.appendChild(c);
      } else if (m.name === "build_and_check") log.appendChild(roundCard(res));
      else if (m.name === "quote") log.appendChild(quoteCard(res));
      else if (m.name === "order_link") log.appendChild(orderCard(res));
    }
  }
  const dims = $("dims");
  dims.replaceChildren();
  for (const k in s.dims || {}) {
    const d = s.dims[k];
    dims.appendChild(h("span", { class: "chip" }, (NAMES[k] || k) + " " + d.value + " mm"));
  }
  $("status").textContent = s.agent === "working" ? "Muse Spark is working" : s.agent === "building" ? "Rust builds the part on your device" : "";
  window.scrollTo(0, document.body.scrollHeight);
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
let mesher = null;
async function buildHere(b) {
  $("status").textContent = "Rust builds the part on your device";
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
  if (n !== 3) throw new Error("The part could not be built: " + (meta.error || "unknown error"));
  const fd = new FormData();
  fd.append("model", new Blob([out(1)], { type: "model/3mf" }), "part.3mf");
  fd.append("picture", new Blob([out(2)], { type: "image/png" }), "part.png");
  $("status").textContent = "Any Part Made checks the part";
  return api("/api/session/" + current.id + "/build", { method: "POST", body: fd });
}

async function run(body) {
  let s = await api("/api/session/" + current.id + "/agent", { method: "POST",
    headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  render(s);
  let guard = 0;
  while ((s.agent === "working" || s.agent === "building") && guard++ < 20) {
    if (s.agent === "building" && s.build) s = await buildHere(s.build);
    else s = await api("/api/session/" + current.id + "/agent", { method: "POST",
      headers: { "content-type": "application/json" }, body: "{}" });
    render(s);
  }
}

async function send(text, tap) {
  if (busy) return;
  text = (text || "").trim();
  if (!current && !pending.length) { showError("Add a photo of the part first."); return; }
  if (!text && !pending.length && !tap) return;
  busy = true; $("send").disabled = true; showError("");
  try {
    let added = [];
    if (!current) {
      $("status").textContent = "Uploading";
      const fd = new FormData();
      fd.append("photo", await shrink(pending[0]), "photo.jpg");
      fd.append("line", text.length >= 3 ? text : "replacement knob");
      fd.append("mode", "agent");
      current = await api("/api/session", { method: "POST", body: fd });
      history.replaceState(null, "", "#s=" + current.id);
      added.push(1);
      for (const f of pending.slice(1)) { current = await upload(f); added.push(current.photos.length); }
    } else {
      for (const f of pending) { $("status").textContent = "Uploading"; current = await upload(f); added.push(current.photos.length); }
    }
    pending = []; renderPending(); $("text").value = "";
    $("status").textContent = "Muse Spark is working";
    await run({ text: text, photos: added, tap: tap || null });
  } catch (e) { showError(e.message); }
  finally { busy = false; $("send").disabled = false; if (current) $("status").textContent = current.agent === "working" ? "Muse Spark is working" : ""; }
}

function renderPending() {
  const box = $("pending");
  box.replaceChildren();
  for (const f of pending) box.appendChild(h("img", { src: URL.createObjectURL(f), alt: "New photo" }));
  box.hidden = !pending.length;
}
for (const id of ["cam", "file"]) {
  $(id).addEventListener("change", (ev) => {
    for (const f of ev.target.files || []) pending.push(f);
    ev.target.value = "";
    renderPending();
  });
}
$("composer").addEventListener("submit", (ev) => { ev.preventDefault(); send($("text").value); });

// Full screen photo: one tap marks a point, a second tap the opposite corner of a box.
let marks = [];
function openViewer(n) {
  viewing = n; marks = [];
  const p = current.photos[n - 1];
  const img = $("v-img");
  img.src = photoUrl(n); img.width = p.width; img.height = p.height;
  $("v-title").textContent = "Photo " + n + ": tap the thing to measure. For a board, tap two opposite corners.";
  $("v-point").hidden = true;
  $("v-marks").replaceChildren();
  $("viewer").hidden = false;
}
function drawMarks() {
  const box = $("v-marks");
  box.replaceChildren();
  const p = current.photos[viewing - 1];
  for (const m of marks) box.appendChild(h("span", { class: "mark", style: "left:" + (m[0] / p.width * 100) + "%;top:" + (m[1] / p.height * 100) + "%" }));
}
$("v-img").addEventListener("click", (ev) => {
  const p = current.photos[viewing - 1], r = ev.target.getBoundingClientRect();
  marks.push([Math.round((ev.clientX - r.left) * p.width / r.width), Math.round((ev.clientY - r.top) * p.height / r.height)]);
  drawMarks();
  if (marks.length === 1) { $("v-point").hidden = false; return; }
  const [a, b] = marks;
  $("viewer").hidden = true;
  send($("text").value, { photo: viewing, x: a[0], y: a[1], x2: b[0], y2: b[1] });
});
$("v-point").addEventListener("click", () => {
  const a = marks[0];
  $("viewer").hidden = true;
  send($("text").value, { photo: viewing, x: a[0], y: a[1] });
});
$("v-close").addEventListener("click", () => { $("viewer").hidden = true; });

(async () => {
  const m = location.hash.match(/^#s=([0-9a-f]{32})$/);
  if (!m) return;
  try {
    current = await api("/api/session/" + m[1]); render(current);
    if (current.agent === "building" && current.build) { render(await buildHere(current.build)); await run({}); }
    else if (current.agent === "working") await run({});
  }
  catch (e) { showError(e.message); }
})();
"##;

const STYLE: &str = r#"
.msg { border-radius: 14px; padding: 10px 14px; margin: 10px 0; max-width: 92%; white-space: pre-wrap; }
.msg p { margin: 0; }
.msg.user { background: var(--accent); color: #fff; margin-left: auto; }
.msg.muse { background: var(--card); border: 1px solid var(--line); }
.strip { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 8px; }
.shot { position: relative; cursor: pointer; }
.shot img { display: block; width: 100%; height: auto; border-radius: 10px; }
.shot.thumb { width: 84px; }
.shot.small { width: 100%; max-width: 360px; margin: 6px 0; }
.shot .pn { position: absolute; left: 6px; top: 6px; background: rgba(0,0,0,0.6); color: #fff; font-size: 12px;
  border-radius: 999px; padding: 0 7px; }
.overlay { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
.tool { background: var(--card); border: 1px solid var(--line); border-left: 4px solid var(--accent); border-radius: 10px;
  padding: 10px 12px; margin: 10px 0; font-size: 14px; }
.tool.good { border-left-color: var(--ok); }
.tool.bad { border-left-color: var(--bad); }
.tool.dim { border-left-color: #6b5bd2; }
.tool .tt { font-weight: 700; margin-bottom: 4px; }
.tool ul { margin: 4px 0; padding-left: 18px; }
.tool p { margin: 4px 0; }
img.knob { display: block; width: 200px; height: 200px; margin: 6px auto; border-radius: 10px; }
.changes li { color: var(--accent); }
.issues li.error { color: var(--bad); }
.issues li.warning { color: #8a5a00; }
.fix { color: var(--muted); }
.guar { color: var(--ok); }
.note { color: var(--muted); font-size: 13px; }
.opt { display: grid; grid-template-columns: 1fr auto; gap: 2px 12px; width: 100%; text-align: left; padding: 10px 12px; margin: 6px 0;
  border: 1px solid var(--line); border-radius: 10px; background: #fff; color: var(--ink); font: inherit; cursor: pointer; }
.opt .price { font-weight: 700; grid-row: span 2; align-self: center; font-size: 18px; }
.opt .when { color: var(--muted); font-size: 13px; }
a.btn { text-decoration: none; }
#dims { display: flex; gap: 6px; flex-wrap: wrap; margin: 6px 0; }
.chip { font-size: 13px; padding: 3px 10px; border-radius: 999px; background: #efeafc; color: #4b3cb0; }
#composer { position: sticky; bottom: 0; background: var(--bg); padding: 10px 0 12px; border-top: 1px solid var(--line); }
#composer textarea { width: 100%; min-height: 64px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 10px;
  font: inherit; resize: vertical; background: #fff; color: var(--ink); }
#composer .row { display: flex; gap: 8px; margin-top: 8px; }
#composer .row .primary { flex: 1; width: auto; }
#composer input[type=file] { position: absolute; width: 1px; height: 1px; opacity: 0; }
#pending { display: flex; gap: 6px; margin-top: 8px; }
#pending img { width: 64px; height: 64px; object-fit: cover; border-radius: 8px; }
#viewer { position: fixed; inset: 0; background: rgba(0,0,0,0.85); z-index: 10; overflow: auto; padding: 12px; }
#viewer p { color: #fff; margin: 4px 0 10px; font-weight: 600; }
#viewer img { display: block; width: 100%; height: auto; cursor: crosshair; border-radius: 8px; }
#viewer .btn { margin-top: 10px; }
.vwrap { position: relative; }
#v-marks { position: absolute; inset: 0; pointer-events: none; }
.mark { position: absolute; width: 18px; height: 18px; margin: -9px 0 0 -9px; border: 3px solid #ffd400; border-radius: 50%;
  box-shadow: 0 0 0 2px rgba(0,0,0,0.6); }
.status:not(:empty)::before { content: ""; display: inline-block; width: 12px; height: 12px; margin-right: 8px;
  vertical-align: -2px; border: 2px solid var(--line); border-top-color: var(--accent); border-radius: 50%;
  animation: spin 0.8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .status:not(:empty)::before { animation: none; } }
"#;

pub fn html() -> String {
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light">
<title>Fix it with a photo</title>
<link rel="icon" href="data:,">
<style>{base}{STYLE}</style>
</head>
<body>
<main>
<h1>Fix it with a photo</h1>
<p class="lead">Need a part? A lost knob, a case for a board, a cap. Send photos and say what it is for. Muse Spark measures with SAM 3.1, asks for what it still needs, designs the part, and Any Part Made checks, prices and guarantees it.</p>
<div id="start" class="card">
<h2>How to start</h2>
<p class="hint">Take a photo of what the part must fit, with a ruler or a coin right next to it: the empty shaft for a knob, the board for a case. Then say what you need.</p>
</div>
<div id="dims"></div>
<div id="log" aria-live="polite"></div>
<p id="error" class="error" role="alert" hidden></p>
<p id="status" class="status" aria-live="polite"></p>
<form id="composer">
<textarea id="text" placeholder="For example: my electric oven is missing a knob, or: a case for this board" maxlength="2000"></textarea>
<div id="pending" hidden></div>
<div class="row">
<label class="btn">Camera<input id="cam" type="file" accept="image/*" capture="environment"></label>
<label class="btn">Photos<input id="file" type="file" accept="image/*" multiple></label>
<button id="send" class="btn primary" type="submit">Send</button>
</div>
</form>
<footer>Built for the Meta Global AI Developer Hackathon. Checks by <a href="https://anypartmade.com">Any Part Made</a>. Photos are deleted after 7 days. <a href="/guided">Step by step mode</a></footer>
</main>
<div id="viewer" hidden>
<p id="v-title"></p>
<div class="vwrap"><img id="v-img" alt="Tap what to measure"><div id="v-marks"></div></div>
<button id="v-point" class="btn primary" type="button" hidden>Use this point</button>
<button id="v-close" class="btn" type="button">Close</button>
</div>
<script>{SCRIPT}</script>
</body>
</html>"##,
        base = crate::page::STYLE
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
