//! The phone page. One static HTML document with a small inline script; the CSP allows
//! exactly that script by its hash.

use crate::coins::COINS;
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
function render(s) {
  $("start").hidden = true;
  $("session").hidden = false;
  $("s-line").textContent = s.line;
  $("s-coin").textContent = s.coin ? s.coin.label + ", " + s.coin.diameter_mm + " mm" : "";
  const box = $("s-photos");
  box.replaceChildren();
  for (const p of s.photos) {
    const img = document.createElement("img");
    img.src = p.url;
    img.alt = "Your photo";
    img.width = p.width;
    img.height = p.height;
    box.appendChild(img);
  }
  const done = STAGES.indexOf(s.stage);
  document.querySelectorAll("#steps li").forEach((li, i) => {
    li.className = i <= done ? "done" : (i === done + 1 ? "next" : "");
  });
}

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
<p class="hint">One photo from the front: the empty shaft, a knob that is still there, and a coin next to them. Hold the phone straight above.</p>
<div class="pick">
<label class="btn">Take photo<input id="photo-camera" type="file" accept="image/*" capture="environment"></label>
<label class="btn">Choose photo<input id="photo-file" type="file" accept="image/*"></label>
</div>
<div id="photo-name" class="fname"></div>
<img id="photo-preview" class="preview" alt="Your photo" hidden>

<label class="f" for="line">What is the part?</label>
<input id="line" type="text" required minlength="3" maxlength="120" placeholder="control knob, electric oven" autocomplete="off">

<label class="f" for="coin">Coin in the photo</label>
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
<dl><dt>Part</dt><dd id="s-line"></dd><dt>Coin</dt><dd id="s-coin"></dd></dl>
<div id="s-photos"></div>
<p class="hint">Photo saved. Measuring with SAM 3.1 comes next.</p>
<button id="again" class="btn" type="button">Start over</button>
</div>
</section>

<footer>Built for the Meta Global AI Developer Hackathon. Checks by <a href="https://anypartmade.com">Any Part Made</a>. Photos are deleted after 7 days.</footer>
</main>
<script>{SCRIPT}</script>
</body>
</html>"#,
        coins = coin_options()
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
        assert_eq!(h.matches("<option").count(), COINS.len());
        assert!(h.contains("1 euro, 23.25 mm"));
        assert!(!h.contains("http://"));
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
