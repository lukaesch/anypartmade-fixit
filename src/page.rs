//! The phone page. One static HTML document with a small inline script; the CSP allows
//! exactly that script by its hash.

use crate::coins::COINS;
use crate::session::COUNTRIES;
use base64::Engine;
use sha2::{Digest, Sha256};

const SCRIPT: &str = r##"
const $ = (id) => document.getElementById(id);
const MAX_EDGE = 2048;
let chosen = { photo: null, side: null };

function pick(slot, input) {
  const f = input.files && input.files[0];
  if (!f) return;
  chosen[slot] = f;
  const img = $(slot + "-preview");
  img.src = URL.createObjectURL(f);
  img.hidden = false;
  $(slot + "-name").textContent = f.name || "Photo";
}
for (const slot of ["photo", "side"]) {
  for (const suffix of ["-camera", "-file"]) {
    const el = $(slot + suffix);
    if (el) el.addEventListener("change", () => pick(slot, el));
  }
}

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
  const w0 = bmp.width, h0 = bmp.height;
  const k = Math.min(1, MAX_EDGE / Math.max(w0, h0));
  const c = document.createElement("canvas");
  c.width = Math.round(w0 * k);
  c.height = Math.round(h0 * k);
  c.getContext("2d").drawImage(bmp, 0, 0, c.width, c.height);
  return await new Promise((ok) => c.toBlob(ok, "image/jpeg", 0.9));
}

function setBusy(on, text) {
  $("go").disabled = on;
  $("status").textContent = text || "";
}

function showError(msg) {
  const e = $("error");
  e.textContent = msg;
  e.hidden = !msg;
}

$("form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  showError("");
  if (!chosen.photo) { showError("Take or choose the photo first."); return; }
  if (!$("form").reportValidity()) return;
  try {
    setBusy(true, "Preparing the photo");
    const fd = new FormData();
    fd.append("photo", await shrink(chosen.photo), "photo.jpg");
    if (chosen.side) fd.append("side", await shrink(chosen.side), "side.jpg");
    fd.append("line", $("line").value);
    fd.append("coin", $("coin").value);
    setBusy(true, "Uploading");
    const r = await fetch("/api/session", { method: "POST", body: fd });
    const body = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error((body.error && body.error.message) || "Upload failed. Try again.");
    history.replaceState(null, "", "#s=" + body.id);
    render(body);
  } catch (e) {
    showError(e.message);
  } finally {
    setBusy(false, "");
  }
});

const STAGES = ["photo", "see", "design", "check", "quote"];
const SVGNS = "http://www.w3.org/2000/svg";
const COLORS = { coin: "#d18f00", shaft: "#0b6bcb", knob: "#1d7a46", ruler: "#c2185b" };
const SCALE_TEXT = { coin: "from the coin", ruler: "from tapped ruler marks", ruler_ticks: "from the ruler ticks" };
let taps = [];
let editRuler = false;
function rulerMode(s) { return s.scale_ref === "ruler" && editRuler; }
function el(tag, attrs, text) {
  const e = document.createElementNS(SVGNS, tag);
  for (const k in attrs) e.setAttribute(k, attrs[k]);
  if (text) e.textContent = text;
  return e;
}
function h(tag, attrs, text) {
  const e = document.createElement(tag);
  for (const k in attrs) e.setAttribute(k, attrs[k]);
  if (text) e.textContent = text;
  return e;
}
function pct(c) { return Math.round((c || 0) * 100) + "%"; }
function row(dl, k, v) {
  const dt = document.createElement("dt"); dt.textContent = k;
  const dd = document.createElement("dd"); dd.textContent = v;
  dl.append(dt, dd);
}

function render(s) {
  current = s;
  $("start").hidden = true;
  $("session").hidden = false;
  $("s-line").textContent = s.line;
  $("s-coin").textContent = s.coin ? s.coin.label + ", " + s.coin.diameter_mm + " mm" :
    (s.scale_ref === "ruler" ? "Ruler" : "");
  const box = $("s-photos");
  box.replaceChildren();
  s.photos.forEach((p, i) => {
    const wrap = document.createElement("div");
    wrap.className = "shot";
    const img = document.createElement("img");
    img.src = p.url;
    img.alt = "Your photo";
    img.width = p.width;
    img.height = p.height;
    wrap.appendChild(img);
    if (i === 0) {
      const svg = el("svg", { viewBox: "0 0 " + p.width + " " + p.height, class: "overlay", id: "overlay" });
      const fs = Math.round(Math.max(p.width, p.height) / 45);
      const label = (x, y, c, text) => svg.appendChild(el("text", { x: x, y: Math.max(fs, y), "font-size": fs,
        fill: "#fff", stroke: c, "stroke-width": fs / 5, "paint-order": "stroke", "font-weight": "700" }, text));
      const lw = Math.max(2, fs / 8);
      for (const o of (s.see ? s.see.overlays : [])) {
        const c = COLORS[o.label] || "#000";
        if (o.line) {
          svg.appendChild(el("line", { x1: o.line[0][0], y1: o.line[0][1], x2: o.line[1][0], y2: o.line[1][1],
            stroke: c, "stroke-width": lw }));
          label(o.line[0][0], o.line[0][1] - fs / 2, c, o.label + ": " + o.text);
          continue;
        }
        svg.appendChild(el("polygon", { points: o.outline.map((q) => q.join(",")).join(" "),
          fill: c, "fill-opacity": "0.25", stroke: c, "stroke-width": lw }));
        label(o.box[0], o.box[1] - fs / 3, c, o.label + ": " + o.text);
      }
      if (s.ruler && !s.see && !rulerMode(s)) {
        const r = s.ruler;
        svg.appendChild(el("line", { x1: r.p1[0], y1: r.p1[1], x2: r.p2[0], y2: r.p2[1],
          stroke: COLORS.ruler, "stroke-width": lw }));
        label(r.p1[0], r.p1[1] - fs / 2, COLORS.ruler, "ruler: " + r.mm + " mm");
      }
      if (rulerMode(s)) {
        for (const q of taps) svg.appendChild(el("circle", { cx: q[0], cy: q[1], r: fs / 2.5,
          fill: "none", stroke: COLORS.ruler, "stroke-width": lw }));
        if (taps.length === 2) svg.appendChild(el("line", { x1: taps[0][0], y1: taps[0][1], x2: taps[1][0],
          y2: taps[1][1], stroke: COLORS.ruler, "stroke-width": lw / 2 }));
        img.classList.add("tap");
        img.addEventListener("click", (ev) => {
          const r = img.getBoundingClientRect();
          const q = [(ev.clientX - r.left) * p.width / r.width, (ev.clientY - r.top) * p.height / r.height];
          if (taps.length < 2) taps.push(q);
          else {
            const d = (a) => Math.hypot(a[0] - q[0], a[1] - q[1]);
            taps[d(taps[0]) <= d(taps[1]) ? 0 : 1] = q;
          }
          render(current);
        });
      }
      wrap.appendChild(svg);
    }
    box.appendChild(wrap);
  });
  const m = $("s-measure");
  m.replaceChildren();
  $("s-notes").replaceChildren();
  if (s.see) {
    const sc = s.see.scale, sh = s.see.shaft, kn = s.see.knob;
    if (sc) row(m, "Scale", (1 / sc.mm_per_px).toFixed(2) + " px per mm, " + (SCALE_TEXT[sc.source] || "") +
      " (" + pct(sc.confidence) + ")");
    if (sh && sh.diameter_mm) row(m, "Shaft", sh.diameter_mm.toFixed(2) + " mm round, " +
      (sh.has_flat ? sh.flat_to_round_mm.toFixed(2) + " mm across the flat" : "no flat seen") +
      ", ±" + sh.plus_minus_mm.toFixed(2) + " mm (" + pct(sh.confidence) + ")");
    if (kn && kn.outer_diameter_mm) row(m, "Knob", kn.outer_diameter_mm.toFixed(1) + " mm, " +
      (kn.rib_count ? kn.rib_count + " grip ribs" : "no ribs seen") + " (" + pct(kn.confidence) + ")");
    for (const n of s.see.notes) {
      const li = document.createElement("li"); li.textContent = n; $("s-notes").appendChild(li);
    }
  }
  if (s.shaft_manual) row(m, "Shaft, typed", s.shaft_manual.diameter_mm + " mm round, " +
    s.shaft_manual.flat_to_round_mm + " mm across the flat");
  const rm = rulerMode(s);
  $("ruler").hidden = !rm;
  $("r-save").disabled = taps.length !== 2;
  $("r-change").hidden = !(s.scale_ref === "ruler" && !rm);
  $("r-change").textContent = s.ruler ? "Change ruler marks" : "Tap two ruler marks instead";
  if (s.ruler && !rm) row(m, "Ruler", s.ruler.mm + " mm over " + Math.round(Math.hypot(s.ruler.p2[0] - s.ruler.p1[0],
    s.ruler.p2[1] - s.ruler.p1[1])) + " px");
  $("measure").hidden = !!s.see || rm;
  $("manual").hidden = !s.see;
  $("s-hint").textContent = rm ? "" : s.see ? "Check the numbers. If the shaft is off, type your calipers reading." :
    (s.scale_ref === "ruler" ? (s.ruler ? "Scale saved. Next: SAM 3.1 finds the shaft and the knob." :
      "Next: SAM 3.1 finds the ruler, the shaft and the knob. The scale comes from the ruler's millimetre ticks.") :
    "Photo saved. Next: SAM 3.1 finds the coin, the shaft and the knob.");
  renderDesign(s);
  renderQuote(s);
  const done = STAGES.indexOf(s.stage);
  document.querySelectorAll("#steps li").forEach((li, i) => {
    li.className = i <= done ? "done" : (i === done + 1 ? "next" : "");
  });
}

const VERDICT = { ok: "ok, ready to quote", needs_changes: "needs changes", needs_review: "a person reviews it",
  refused: "refused", needs_upload: "waiting for the file" };
function shaftKnown(s) {
  return !!(s.shaft_manual || (s.see && s.see.shaft && s.see.shaft.diameter_mm));
}
function renderDesign(s) {
  $("design-card").hidden = !shaftKnown(s);
  const rounds = s.rounds || [];
  $("design").textContent = rounds.length ? "Design again with Muse Spark" : "Design with Muse Spark";
  $("design").hidden = designing;
  const list = $("rounds");
  list.replaceChildren();
  for (const r of rounds) {
    const c = r.check || {};
    const li = document.createElement("li");
    li.className = "round v-" + (c.verdict || "x");
    const head = h("div", { class: "rh" });
    head.append(h("b", null, "Round " + r.n), h("span", { class: "badge" }, VERDICT[c.verdict] || String(c.verdict)));
    li.appendChild(head);
    const pic = h("img", { class: "knob", src: r.picture_url, alt: "The knob of round " + r.n, width: "480", height: "480" });
    li.appendChild(pic);
    const ch = r.changes || {};
    if (ch.summary) li.appendChild(h("p", { class: "muse" }, "Muse Spark: " + ch.summary));
    const changes = (ch.changes || []).filter((x) => x.from !== x.to);
    if (changes.length) {
      const ul = h("ul", { class: "changes" });
      for (const x of changes) ul.appendChild(h("li", null, x.field + ": " + x.from + " → " + x.to + ". " + x.why));
      li.appendChild(ul);
    }
    const issues = (c.manufacturability || []).filter((i) => i.severity !== "info");
    if (issues.length) {
      const ul = h("ul", { class: "issues" });
      for (const i of issues) {
        const it = h("li", { class: i.severity }, i.severity + " " + i.code + ": " + i.message);
        if (i.fix) it.appendChild(h("div", { class: "fix" }, "Fix: " + i.fix));
        ul.appendChild(it);
      }
      li.appendChild(ul);
    }
    // A refusal is shown exactly as the core wrote it.
    if (c.verdict === "refused" || c.verdict === "needs_review")
      li.appendChild(h("p", { class: "core" }, [c.message, c.reason, c.next_step].filter(Boolean).join(" ")));
    if (c.verdict === "ok" && c.guarantee_preview) {
      const dims = (c.guarantee_preview.dimensions || []).map((d) => d.label + " " + d.nominal_mm + " mm " + d.tolerance_mm);
      li.appendChild(h("p", { class: "core" }, "Guaranteed: " + dims.join(", ") + ". " + (c.guarantee_preview.text || "")));
    }
    const a = h("a", { href: r.model_url, download: "" }, "Download 3MF (" + Math.round(r.file_bytes / 1024) + " KB)");
    li.appendChild(a);
    list.appendChild(li);
  }
}
const LEAD = { opt_express: "Express", opt_standard: "Standard", opt_economy: "Economy" };
function lastOk(s) {
  const r = s.rounds || [];
  return r.length > 0 && r[r.length - 1].check.verdict === "ok";
}
function renderQuote(s) {
  $("quote-card").hidden = !lastOk(s) || designing;
  const q = s.quote, box = $("q-options");
  box.replaceChildren();
  $("q-guarantee").replaceChildren();
  $("q-notes").replaceChildren();
  $("q-get").textContent = q ? "Price again" : "Get the price";
  if (!q) { $("q-order").hidden = true; return; }
  if (q.ship_to_country) $("q-country").value = q.ship_to_country;
  for (const o of q.options || []) {
    const d = o.delivery_estimate || {};
    const b = h("button", { class: "opt" + (s.order && s.order.lead_time === o.lead_time ? " picked" : ""), type: "button" });
    b.append(h("b", null, LEAD[o.option_id] || o.lead_time), h("span", { class: "price" }, o.price_display),
      h("span", { class: "when" }, "arrives " + d.earliest + " to " + d.latest));
    b.addEventListener("click", () => pickOption(o.option_id));
    box.appendChild(b);
  }
  const g = q.guarantee || {};
  for (const d of g.dimensions || [])
    $("q-guarantee").appendChild(h("li", null, d.label + ": " + d.nominal_mm + " mm, " + d.tolerance_mm + " mm"));
  if (g.remedy) $("q-guarantee").appendChild(h("li", null, g.remedy));
  for (const n of q.notes || []) $("q-notes").appendChild(h("li", null, n));
  const o = s.order;
  $("q-order").hidden = !o;
  if (o) {
    $("q-link").href = o.order_url;
    $("q-order-text").textContent = (LEAD["opt_" + o.lead_time] || o.lead_time) + ", " + o.price_display +
      ". Nothing is ordered until you approve it on anypartmade.com. The link works until " +
      String(o.link_expires || "").replace("T", " ").replace("Z", " UTC") + ".";
  }
}
async function pickOption(id) {
  $("q-error").hidden = true;
  $("q-status").textContent = "Asking Any Part Made for the approval link";
  try { render(await post("/order", { option_id: id })); }
  catch (e) { $("q-error").textContent = e.message; $("q-error").hidden = false; }
  finally { $("q-status").textContent = ""; }
}
let designing = false;
const MAX_FIXES = 4;

let current = null;
async function post(path, body) {
  const r = await fetch("/api/session/" + current.id + path, { method: "POST",
    headers: body ? { "content-type": "application/json" } : {}, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error((j.error && j.error.message) || "Something went wrong. Try again.");
  return j;
}
$("measure").addEventListener("click", async () => {
  const b = $("measure");
  b.disabled = true;
  $("s-error").hidden = true;
  $("s-status").textContent = "SAM 3.1 is looking for the coin, the shaft and the knob";
  try { render(await post("/see")); }
  catch (e) { $("s-error").textContent = e.message; $("s-error").hidden = false; }
  finally { b.disabled = false; $("s-status").textContent = ""; }
});
$("r-save").addEventListener("click", async () => {
  $("s-error").hidden = true;
  try {
    const mm = parseFloat($("r-mm").value.replace(",", "."));
    const s = await post("/scale", { p1: taps[0], p2: taps[1], mm: mm });
    editRuler = false;
    render(s);
  } catch (e) { $("s-error").textContent = e.message; $("s-error").hidden = false; }
});
$("r-change").addEventListener("click", () => { editRuler = true; taps = []; render(current); });
$("r-reset").addEventListener("click", () => { taps = []; render(current); });
$("manual").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  $("s-error").hidden = true;
  try {
    render(await post("/shaft", { diameter_mm: parseFloat($("m-d").value.replace(",", ".")),
      flat_to_round_mm: parseFloat($("m-f").value.replace(",", ".")) }));
  } catch (e) { $("s-error").textContent = e.message; $("s-error").hidden = false; }
});

$("design").addEventListener("click", async () => {
  designing = true;
  $("d-error").hidden = true;
  render(current);
  try {
    $("d-status").textContent = "Muse Spark looks at the photo and designs the knob, then Any Part Made checks it";
    let s = await post("/design");
    render(s);
    let fixes = 0;
    while (s.rounds.length && s.rounds[s.rounds.length - 1].check.verdict === "needs_changes" && fixes < MAX_FIXES) {
      fixes++;
      $("d-status").textContent = "Round " + (s.rounds.length + 1) + ": Muse Spark reads the issues and changes the design";
      s = await post("/fix");
      render(s);
    }
  } catch (e) { $("d-error").textContent = e.message; $("d-error").hidden = false; }
  finally { designing = false; $("d-status").textContent = ""; render(current); }
});

$("q-get").addEventListener("click", async () => {
  $("q-error").hidden = true;
  $("q-get").disabled = true;
  $("q-status").textContent = "Any Part Made prices the part";
  try { render(await post("/quote", { country: $("q-country").value })); }
  catch (e) { $("q-error").textContent = e.message; $("q-error").hidden = false; }
  finally { $("q-get").disabled = false; $("q-status").textContent = ""; }
});

$("again").addEventListener("click", () => {
  history.replaceState(null, "", location.pathname);
  location.reload();
});

(async () => {
  const m = location.hash.match(/^#s=([0-9a-f]{32})$/);
  if (!m) return;
  const r = await fetch("/api/session/" + m[1]);
  if (r.ok) render(await r.json());
})();
"##;

const STYLE: &str = r#"
:root { color-scheme: light; --ink: #1c1d1f; --muted: #5d6168; --line: #dfe2e6; --bg: #f6f7f8;
  --card: #fff; --accent: #0b6bcb; --ok: #1d7a46; --bad: #b42318; }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink);
  font: 16px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; }
main { max-width: 640px; margin: 0 auto; padding: 20px 16px 48px; }
h1 { font-size: 26px; margin: 8px 0 4px; letter-spacing: -0.01em; }
.lead { color: var(--muted); margin: 0 0 20px; }
.card { background: var(--card); border: 1px solid var(--line); border-radius: 14px; padding: 16px; margin-bottom: 16px; }
h2 { font-size: 17px; margin: 0 0 8px; }
p.hint { color: var(--muted); font-size: 14px; margin: 4px 0 12px; }
.pick { display: flex; gap: 8px; flex-wrap: wrap; }
.btn { display: inline-flex; align-items: center; justify-content: center; min-height: 44px; padding: 0 16px;
  border-radius: 10px; border: 1px solid var(--line); background: #fff; color: var(--ink); font: inherit;
  font-weight: 600; cursor: pointer; }
.btn.primary { background: var(--accent); border-color: var(--accent); color: #fff; width: 100%; }
.btn:disabled { opacity: 0.6; cursor: default; }
.pick input { position: absolute; width: 1px; height: 1px; opacity: 0; }
.preview { display: block; max-width: 100%; max-height: 260px; margin-top: 12px; border-radius: 10px; }
.fname { font-size: 13px; color: var(--muted); margin-top: 6px; word-break: break-all; }
label.f { display: block; font-weight: 600; font-size: 14px; margin: 14px 0 6px; }
input[type=text], select { width: 100%; min-height: 44px; padding: 8px 12px; border: 1px solid var(--line);
  border-radius: 10px; font: inherit; background: #fff; color: var(--ink); }
.error { color: var(--bad); font-weight: 600; margin: 12px 0 0; }
.status { color: var(--muted); font-size: 14px; min-height: 20px; margin: 10px 0 0; }
#steps { list-style: none; padding: 0; margin: 0 0 16px; display: flex; gap: 6px; flex-wrap: wrap; }
#steps li { font-size: 13px; padding: 4px 10px; border-radius: 999px; border: 1px solid var(--line); color: var(--muted); background: #fff; }
#steps li.done { background: #e7f4ec; border-color: #b9dfc8; color: var(--ok); }
#steps li.next { border-color: var(--accent); color: var(--accent); }
#s-photos img { display: block; width: 100%; height: auto; border-radius: 10px; margin-bottom: 8px; }
dl { display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; margin: 0 0 12px; }
dt { color: var(--muted); }
dd { margin: 0; }
.shot { position: relative; margin-bottom: 8px; }
.shot img { margin: 0; }
.overlay { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
.notes { color: var(--muted); font-size: 14px; padding-left: 18px; margin: 0 0 8px; }
.manual { margin: 8px 0 0; }
.ruler { margin: 8px 0; }
.ruler .pick { margin-top: 12px; }
.ruler .primary { width: auto; flex: 1; }
img.tap { cursor: crosshair; }
.manual .btn { margin-top: 12px; }
.rounds { list-style: none; padding: 0; margin: 12px 0 0; }
.round { border: 1px solid var(--line); border-radius: 10px; padding: 12px; margin-top: 10px; font-size: 14px; }
.rh { display: flex; justify-content: space-between; align-items: center; gap: 8px; margin-bottom: 6px; }
.badge { font-size: 13px; font-weight: 600; padding: 2px 10px; border-radius: 999px; background: #fdecea; color: var(--bad); }
.v-ok { border-color: #b9dfc8; }
.v-ok .badge { background: #e7f4ec; color: var(--ok); }
.muse { margin: 0 0 6px; }
img.knob { display: block; width: 160px; height: 160px; margin: 0 auto 8px; border-radius: 10px; }
.v-ok img.knob { width: 240px; height: 240px; }
.changes, .issues { margin: 0 0 8px; padding-left: 18px; }
.changes li { color: var(--accent); }
.issues li.error { color: var(--bad); }
.issues li.warning { color: #8a5a00; }
.fix { color: var(--muted); }
.core { margin: 0 0 8px; color: var(--ink); }
.round a { color: var(--accent); }
.status:not(:empty)::before { content: ""; display: inline-block; width: 12px; height: 12px; margin-right: 8px;
  vertical-align: -2px; border: 2px solid var(--line); border-top-color: var(--accent); border-radius: 50%;
  animation: spin 0.8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .status:not(:empty)::before { animation: none; } }
.issues li.error { margin: 0; }
.issues li { margin-bottom: 4px; }
.opts { display: grid; gap: 8px; margin: 12px 0; }
.opt { display: grid; grid-template-columns: 1fr auto; gap: 2px 12px; text-align: left; padding: 12px;
  border: 1px solid var(--line); border-radius: 10px; background: #fff; color: var(--ink); font: inherit; cursor: pointer; }
.opt .price { font-weight: 700; grid-row: span 2; align-self: center; font-size: 18px; }
.opt .when { color: var(--muted); font-size: 14px; }
.opt.picked { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
.guarantee { padding-left: 18px; margin: 0 0 8px; font-size: 14px; color: var(--ok); }
a.btn { text-decoration: none; }
footer { color: var(--muted); font-size: 13px; text-align: center; margin-top: 24px; }
footer a { color: inherit; }
"#;

fn coin_options() -> String {
    COINS
        .iter()
        .map(|c| {
            format!(
                r#"<option value="{}">{}, {} mm</option>"#,
                c.id, c.label, c.diameter_mm
            )
        })
        .chain(std::iter::once(
            r#"<option value="ruler">Ruler with millimetre marks</option>"#.to_string(),
        ))
        .collect()
}

pub fn html() -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light">
<title>Fix it with a photo</title>
<link rel="icon" href="data:,">
<style>{STYLE}</style>
</head>
<body>
<main>
<h1>Fix it with a photo</h1>
<p class="lead">A knob is missing? Take one photo. Meta SAM 3.1 measures, Muse Spark designs the part, Any Part Made checks it.</p>

<section id="start">
<form id="form" class="card" novalidate>
<h2>1. Photo</h2>
<p class="hint">One photo from the front: the empty shaft, a knob that is still there, and a coin flat next to the shaft (or a ruler). Step back a little, use 2x zoom and hold the phone straight.</p>
<div class="pick">
<label class="btn">Take photo<input id="photo-camera" type="file" accept="image/*" capture="environment"></label>
<label class="btn">Choose photo<input id="photo-file" type="file" accept="image/*"></label>
</div>
<div id="photo-name" class="fname"></div>
<img id="photo-preview" class="preview" alt="Your photo" hidden>

<label class="f" for="line">What is the part?</label>
<input id="line" type="text" required minlength="3" maxlength="120" placeholder="control knob, electric oven" autocomplete="off">

<label class="f" for="coin">Coin or ruler in the photo</label>
<select id="coin" required>{coins}</select>

<details>
<summary class="hint">Optional: a second photo of the knob from the side, for its height</summary>
<div class="pick">
<label class="btn">Take side photo<input id="side-camera" type="file" accept="image/*" capture="environment"></label>
<label class="btn">Choose side photo<input id="side-file" type="file" accept="image/*"></label>
</div>
<div id="side-name" class="fname"></div>
<img id="side-preview" class="preview" alt="Side photo" hidden>
</details>

<p id="error" class="error" role="alert" hidden></p>
<p id="status" class="status" aria-live="polite"></p>
<button id="go" class="btn primary" type="submit">Save photo</button>
</form>
</section>

<section id="session" hidden>
<ol id="steps">
<li>Photo</li><li>Measure</li><li>Design</li><li>Check</li><li>Quote</li>
</ol>
<div class="card">
<h2>Your photo</h2>
<dl><dt>Part</dt><dd id="s-line"></dd><dt>Scale</dt><dd id="s-coin"></dd></dl>
<div id="s-photos"></div>
<dl id="s-measure"></dl>
<ul id="s-notes" class="notes"></ul>
<div id="ruler" class="ruler" hidden>
<p class="hint"><b>Set the scale.</b> Tap two marks on the ruler in the photo, as far apart as you can. Tap again to move the closer mark.</p>
<label class="f" for="r-mm">Distance between the two marks (mm)</label>
<input id="r-mm" type="text" inputmode="decimal" value="50">
<div class="pick"><button id="r-save" class="btn primary" type="button" disabled>Save scale</button>
<button id="r-reset" class="btn" type="button">Clear marks</button></div>
</div>
<button id="r-change" class="btn" type="button" hidden>Change ruler marks</button>
<p id="s-hint" class="hint"></p>
<p id="s-error" class="error" role="alert" hidden></p>
<p id="s-status" class="status" aria-live="polite"></p>
<button id="measure" class="btn primary" type="button">Measure with SAM 3.1</button>
<form id="manual" class="manual" hidden>
<label class="f" for="m-d">Shaft diameter from calipers (mm)</label>
<input id="m-d" type="text" inputmode="decimal" placeholder="6.00" required>
<label class="f" for="m-f">Across the flat, flat side to round side (mm)</label>
<input id="m-f" type="text" inputmode="decimal" placeholder="4.60" required>
<button class="btn" type="submit">Use these numbers</button>
</form>
</div>
<div class="card" id="design-card" hidden>
<h2>Design and check</h2>
<p class="hint">Muse Spark writes the knob as a small spec. Rust builds the 3D file. Any Part Made checks it and says what to fix. Muse fixes it, up to four rounds.</p>
<p id="d-error" class="error" role="alert" hidden></p>
<p id="d-status" class="status" aria-live="polite"></p>
<button id="design" class="btn primary" type="button">Design with Muse Spark</button>
<ol id="rounds" class="rounds"></ol>
</div>
<div class="card" id="quote-card" hidden>
<h2>Price and approval</h2>
<label class="f" for="q-country">Ship to</label>
<select id="q-country">{countries}</select>
<p id="q-error" class="error" role="alert" hidden></p>
<p id="q-status" class="status" aria-live="polite"></p>
<button id="q-get" class="btn primary" type="button">Get the price</button>
<div id="q-options" class="opts"></div>
<ul id="q-guarantee" class="guarantee"></ul>
<ul id="q-notes" class="notes"></ul>
<div id="q-order" hidden>
<p id="q-order-text" class="hint"></p>
<a id="q-link" class="btn primary" href="/" target="_blank" rel="noopener">Review and approve on anypartmade.com</a>
</div>
</div>
<p><button id="again" class="btn" type="button">Start over</button></p>
</section>

<footer>Built for the Meta Global AI Developer Hackathon. Checks by <a href="https://anypartmade.com">Any Part Made</a>. Photos are deleted after 7 days.</footer>
</main>
<script>{SCRIPT}</script>
</body>
</html>"#,
        coins = coin_options(),
        countries = COUNTRIES
            .iter()
            .map(|(c, n)| format!(r#"<option value="{c}">{n}</option>"#))
            .collect::<String>()
    )
}

/// Content Security Policy: only this page's own inline script and same origin requests.
pub fn csp() -> String {
    let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(SCRIPT.as_bytes()));
    format!(
        "default-src 'none'; script-src 'sha256-{hash}'; style-src 'unsafe-inline'; \
         img-src 'self' blob: data:; connect-src 'self'; base-uri 'none'; form-action 'self'; \
         frame-ancestors 'none'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_has_capture_coins_and_no_external_assets() {
        let h = html();
        assert!(h.contains(r#"capture="environment""#));
        assert_eq!(
            h.matches("<option").count(),
            COINS.len() + 1 + COUNTRIES.len()
        );
        assert!(h.contains(r#"value="ruler""#));
        assert!(h.contains("1 euro, 23.25 mm"));
        // The only http URL is the SVG namespace name, which loads nothing.
        assert!(!h
            .replace("http://www.w3.org/2000/svg", "")
            .contains("http://"));
        assert!(!h.contains("<script src"));
        assert!(!h.contains('\u{2013}') && !h.contains('\u{2014}'));
        assert!(h.contains(&format!("<script>{SCRIPT}</script>")));
    }

    #[test]
    fn csp_pins_the_script() {
        let c = csp();
        assert!(c.contains("script-src 'sha256-"));
        assert!(!c.contains("unsafe-eval"));
    }
}
