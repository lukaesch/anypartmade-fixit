//! HTTP routes. Only glue lives here; checks and data shapes are in the plain modules.

use crate::knob::KnobSpec;
use crate::sam;
use crate::see::{self, ManualShaft, RulerScale, ScaleSource};
use crate::session::{self, PhotoRef, Round, Session};
use crate::{agent, coins, mcp, muse, page, photo, render, threemf};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use worker::wasm_bindgen::JsValue;
use worker::*;

const PHOTOS: &str = "PHOTOS";

/// The browser part builder (mesher/), built before the Worker.
static MESHER: &[u8] = include_bytes!("../target/wasm32-unknown-unknown/release/fixit_mesher.wasm");

fn json_response(status: u16, v: &Value) -> Result<Response> {
    let mut r = Response::from_json(v)?.with_status(status);
    r.headers_mut().set("cache-control", "no-store")?;
    Ok(r)
}

fn error(status: u16, code: &str, message: &str) -> Result<Response> {
    json_response(
        status,
        &json!({"error": {"code": code, "message": message}}),
    )
}

fn new_id() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|e| Error::RustError(e.to_string()))?;
    Ok(hex::encode(b))
}

fn now_ms() -> i64 {
    Date::now().as_millis() as i64
}

/// Hashed client IP with the day mixed in. The raw IP is never stored.
fn ip_key(req: &Request, day: i64) -> Result<String> {
    let ip = req
        .headers()
        .get("cf-connecting-ip")?
        .unwrap_or_else(|| "unknown".into());
    let h = Sha256::digest(format!("fixit|{day}|{ip}").as_bytes());
    Ok(hex::encode(&h[..16]))
}

async fn bump_rate(db: &D1Database, key: &str, day: i64) -> Result<i64> {
    let row: Option<Value> = db
        .prepare(
            "INSERT INTO rate (key, day, n) VALUES (?1, ?2, 1) \
             ON CONFLICT(key) DO UPDATE SET n = n + 1 RETURNING n",
        )
        .bind(&[key.into(), (day as f64).into()])?
        .first(None)
        .await?;
    Ok(row
        .and_then(|r| r.get("n").and_then(Value::as_i64))
        .unwrap_or(0))
}

async fn load(db: &D1Database, id: &str) -> Result<Option<Session>> {
    let row: Option<Value> = db
        .prepare("SELECT data FROM sessions WHERE id = ?1")
        .bind(&[id.into()])?
        .first(None)
        .await?;
    Ok(row
        .and_then(|r| r.get("data").and_then(Value::as_str).map(str::to_owned))
        .and_then(|d| serde_json::from_str(&d).ok()))
}

async fn save(db: &D1Database, s: &Session) -> Result<()> {
    let data = serde_json::to_string(s)?;
    db.prepare(
        "INSERT INTO sessions (id, created_at, data) VALUES (?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(&[
        s.id.as_str().into(),
        (s.created_at_ms as f64).into(),
        data.into(),
    ])?
    .run()
    .await?;
    Ok(())
}

async fn file_field(form: &FormData, name: &str) -> Result<Option<Vec<u8>>> {
    match form.get(name) {
        Some(FormEntry::File(f)) => Ok(Some(f.bytes().await?)),
        _ => Ok(None),
    }
}

fn text_field(form: &FormData, name: &str) -> String {
    match form.get(name) {
        Some(FormEntry::Field(s)) => s,
        _ => String::new(),
    }
}

async fn store_photo(env: &Env, bytes: Vec<u8>) -> std::result::Result<PhotoRef, String> {
    let info = photo::inspect(&bytes)?;
    let r = PhotoRef::new(&info);
    let bucket = env.bucket(PHOTOS).map_err(|e| e.to_string())?;
    bucket
        .put(&r.key, bytes)
        .http_metadata(HttpMetadata {
            content_type: Some(r.content_type.clone()),
            ..Default::default()
        })
        .execute()
        .await
        .map_err(|e| e.to_string())?;
    Ok(r)
}

async fn create_session(mut req: Request, env: &Env) -> Result<Response> {
    let len: usize = req
        .headers()
        .get("content-length")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if len > 2 * photo::MAX_BYTES + 64 * 1024 {
        return error(
            413,
            "TOO_LARGE",
            "The upload is too large. Photos can be at most 8 MB each.",
        );
    }
    let form = match req.form_data().await {
        Ok(f) => f,
        Err(_) => return error(400, "BAD_FORM", "Send the photo as a form upload."),
    };
    // Agent mode starts from the user's first message: the line is its start, and the
    // agent picks the scale reference per photo.
    let agent_mode = text_field(&form, "mode") == "agent";
    let raw_line = text_field(&form, "line");
    let raw_line = if agent_mode {
        raw_line.chars().take(120).collect()
    } else {
        raw_line
    };
    let line = match session::clean_line(&raw_line) {
        Ok(l) => l,
        Err(m) => return error(400, "BAD_LINE", &m),
    };
    let coin = if agent_mode && text_field(&form, "coin").is_empty() {
        session::RULER.to_string()
    } else {
        text_field(&form, "coin")
    };
    let coin = match session::scale_ref(&coin) {
        Ok(c) => c,
        Err(m) => return error(400, "BAD_COIN", &m),
    };
    let Some(main) = file_field(&form, "photo").await? else {
        return error(400, "PHOTO_REQUIRED", "Take or choose the photo first.");
    };
    let side = file_field(&form, "side").await?;

    let db = env.d1("DB")?;
    let now = now_ms();
    let day = session::day(now);
    let n = bump_rate(&db, &ip_key(&req, day)?, day).await?;
    if n > session::SESSIONS_PER_IP_PER_DAY {
        return error(
            429,
            "RATE_LIMIT",
            "That is 20 repairs today from this network. Try again tomorrow.",
        );
    }

    let mut photos = Vec::new();
    for bytes in std::iter::once(main).chain(side) {
        match store_photo(env, bytes).await {
            Ok(p) => photos.push(p),
            Err(m) => return error(400, "BAD_PHOTO", &m),
        }
    }
    let s = Session {
        id: new_id()?,
        created_at_ms: now,
        line,
        coin_id: coin,
        photos,
        stage: "photo".into(),
        see: None,
        shaft_manual: None,
        ruler: None,
        shaft_tap: None,
        spec: None,
        rounds: Vec::new(),
        quote: None,
        order: None,
        chat: Vec::new(),
        measurements: Vec::new(),
        dims: Default::default(),
        agent: String::new(),
        ruler_marks: Default::default(),
        build: None,
    };
    save(&db, &s).await?;
    console_log!("session {} created, {} photo(s)", s.id, s.photos.len());
    json_response(201, &s.state_json())
}

async fn get_photo(env: &Env, s: &Session, n: &str) -> Result<Response> {
    let Some(p) = n
        .parse::<usize>()
        .ok()
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| s.photos.get(i))
    else {
        return error(404, "NOT_FOUND", "No such photo.");
    };
    let Some(obj) = env.bucket(PHOTOS)?.get(&p.key).execute().await? else {
        return error(
            404,
            "NOT_FOUND",
            "The photo was deleted. Photos are kept for 7 days.",
        );
    };
    let bytes = match obj.body() {
        Some(b) => b.bytes().await?,
        None => Vec::new(),
    };
    let mut r = Response::from_bytes(bytes)?;
    let h = r.headers_mut();
    h.set("content-type", &p.content_type)?;
    h.set("cache-control", "private, max-age=86400")?;
    h.set("x-content-type-options", "nosniff")?;
    Ok(r)
}

/// POST to the Meta Model API. The key is a Worker secret and is never logged.
async fn meta_post(env: &Env, url: &str, body: &Value) -> std::result::Result<Value, String> {
    let key = env
        .secret("META_API_KEY")
        .map_err(|_| "The Meta API key is not set.".to_string())?
        .to_string()
        .trim()
        .to_string();
    if key.is_empty() {
        return Err("The Meta API key is empty.".into());
    }
    let headers = Headers::new();
    let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| e.to_string());
    set("authorization", &format!("Bearer {key}"))?;
    set("content-type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(JsValue::from_str(&body.to_string())));
    let req = Request::new_with_init(url, &init).map_err(|e| e.to_string())?;
    let mut resp = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| format!("Meta API not reachable: {e}"))?;
    let status = resp.status_code();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status != 200 {
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let msg: String = msg.chars().take(300).collect();
        return Err(format!("Meta API answered {status}. {msg}"));
    }
    serde_json::from_str(&text).map_err(|_| "Meta API sent a reply we cannot read.".into())
}

fn budget(env: &Env) -> f64 {
    env.var("BUDGET_USD_PER_DAY")
        .ok()
        .and_then(|v| v.to_string().parse().ok())
        .unwrap_or(5.0)
}

async fn spent_today(db: &D1Database, day: i64) -> Result<f64> {
    let row: Option<Value> = db
        .prepare("SELECT COALESCE(SUM(usd), 0) AS s FROM spend WHERE day = ?1")
        .bind(&[(day as f64).into()])?
        .first(None)
        .await?;
    Ok(row.and_then(|r| r["s"].as_f64()).unwrap_or(0.0))
}

async fn log_spend(
    db: &D1Database,
    model: &str,
    session: &str,
    detail: &str,
    usd: f64,
) -> Result<()> {
    let now = now_ms();
    db.prepare(
        "INSERT INTO spend (at, day, model, session, detail, usd) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&[
        (now as f64).into(),
        (session::day(now) as f64).into(),
        model.into(),
        session.into(),
        detail.into(),
        usd.into(),
    ])?
    .run()
    .await?;
    console_log!("spend {model} {detail} ${usd:.4} session {session}");
    Ok(())
}

async fn r2_text(bucket: &Bucket, key: &str) -> Result<Option<String>> {
    match bucket.get(key).execute().await? {
        Some(obj) => match obj.body() {
            Some(b) => Ok(Some(b.text().await?)),
            None => Ok(Some(String::new())),
        },
        None => Ok(None),
    }
}

fn sam_cache_key(p: &PhotoRef, key: &str) -> String {
    format!("sam/{}/{}.txt", p.sha256, key)
}

/// One SAM call for one concept, budget checked, logged and cached.
/// The inner Err is the failure to report.
async fn sam_call(
    env: &Env,
    db: &D1Database,
    bucket: &Bucket,
    session_id: &str,
    photo: &PhotoRef,
    key: &str,
    prompt: &str,
) -> Result<std::result::Result<String, Fail>> {
    let day = session::day(now_ms());
    if spent_today(db, day).await? + sam::USD_PER_IMAGE > budget(env) {
        return Ok(Err(budget_fail()));
    }
    let Some(bytes) = photo_bytes(bucket, photo).await? else {
        return Ok(Err(photo_gone()));
    };
    let data_url = format!(
        "data:{};base64,{}",
        photo.content_type,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );
    let reply = match meta_post(env, sam::URL, &sam::request(prompt, &data_url)).await {
        Ok(r) => r,
        Err(m) => return Ok(Err((502, "META_ERROR", m))),
    };
    let n = sam::images_processed(&reply);
    log_spend(
        db,
        sam::MODEL,
        session_id,
        key,
        n as f64 * sam::USD_PER_IMAGE,
    )
    .await?;
    let text = sam::output_text(&reply);
    bucket
        .put(sam_cache_key(photo, key), text.clone().into_bytes())
        .execute()
        .await?;
    Ok(Ok(text))
}

/// SAM 3.1 on the first photo for coin, shaft and knob (cached per photo hash),
/// then the measurements in Rust.
/// A failed step: HTTP status, error code, message for the user.
type Fail = (u16, &'static str, String);

fn fail(f: Fail) -> Result<Response> {
    error(f.0, f.1, &f.2)
}

async fn photo_bytes(bucket: &Bucket, photo: &PhotoRef) -> Result<Option<Vec<u8>>> {
    Ok(match bucket.get(&photo.key).execute().await? {
        Some(obj) => Some(match obj.body() {
            Some(b) => b.bytes().await?,
            None => Vec::new(),
        }),
        None => None,
    })
}

/// SAM 3.1 on one photo for the scale reference, shaft and knob (cached per photo hash),
/// then the measurements in Rust. `scale_ref` is "ruler" or a coin id; `taps` are two
/// ruler marks the user set; `shaft_point` is a tap on the shaft.
#[allow(clippy::too_many_arguments)]
async fn measure_photo(
    env: &Env,
    session_id: &str,
    photo: &PhotoRef,
    scale_ref: &str,
    taps: Option<&RulerScale>,
    shaft_point: Option<[u32; 2]>,
    with_shaft: bool,
    focus: Option<(f64, f64)>,
) -> Result<std::result::Result<Value, Fail>> {
    let ruler = scale_ref == session::RULER;
    // Tapped marks win; otherwise the ruler is found by SAM and read from its ticks.
    let ruler_auto = ruler && taps.is_none();
    let db = env.d1("DB")?;
    let bucket = env.bucket(PHOTOS)?;
    let wanted: Vec<bool> = see::CONCEPTS
        .iter()
        .map(|(k, _)| match *k {
            "coin" => !ruler,
            "ruler" => ruler_auto,
            _ => with_shaft,
        })
        .collect();
    let mut texts: Vec<Option<String>> = Vec::new();
    for (i, (key, _)) in see::CONCEPTS.iter().enumerate() {
        texts.push(if wanted[i] {
            r2_text(&bucket, &sam_cache_key(photo, key)).await?
        } else {
            Some(String::new())
        });
    }
    let missing: Vec<usize> = (0..texts.len()).filter(|&i| texts[i].is_none()).collect();
    if !missing.is_empty() {
        let day = session::day(now_ms());
        let cost = missing.len() as f64 * sam::USD_PER_IMAGE;
        if spent_today(&db, day).await? + cost > budget(env) {
            return Ok(Err(budget_fail()));
        }
        let Some(bytes) = photo_bytes(&bucket, photo).await? else {
            return Ok(Err(photo_gone()));
        };
        let data_url = format!(
            "data:{};base64,{}",
            photo.content_type,
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        );
        let calls = missing.iter().map(|&i| {
            let body = sam::request(see::CONCEPTS[i].1, &data_url);
            async move { meta_post(env, sam::URL, &body).await }
        });
        let replies = futures::future::join_all(calls).await;
        for (&i, reply) in missing.iter().zip(replies) {
            let reply = match reply {
                Ok(r) => r,
                Err(m) => return Ok(Err((502, "META_ERROR", m))),
            };
            let n = sam::images_processed(&reply);
            log_spend(
                &db,
                sam::MODEL,
                session_id,
                see::CONCEPTS[i].0,
                n as f64 * sam::USD_PER_IMAGE,
            )
            .await?;
            let text = sam::output_text(&reply);
            bucket
                .put(
                    sam_cache_key(photo, see::CONCEPTS[i].0),
                    text.clone().into_bytes(),
                )
                .execute()
                .await?;
            texts[i] = Some(text);
        }
    }
    let mut parsed = Vec::new();
    for (i, t) in texts.iter().enumerate() {
        match sam::parse(t.as_deref().unwrap_or("")) {
            Ok(o) => parsed.push(o),
            Err(m) => {
                return Ok(Err((
                    502,
                    "SAM_PARSE",
                    format!(
                        "Could not read the SAM result for {}: {m}",
                        see::CONCEPTS[i].0
                    ),
                )))
            }
        }
    }
    // A tap on the shaft wins: SAM segments what is under the finger.
    if let Some([x, y]) = shaft_point {
        let key = format!("shaft_tap_{x}_{y}");
        let prompt = sam::point_prompt(x, y, photo.width, photo.height);
        let text = match r2_text(&bucket, &sam_cache_key(photo, &key)).await? {
            Some(t) => t,
            None => match sam_call(env, &db, &bucket, session_id, photo, &key, &prompt).await? {
                Ok(t) => t,
                Err(f) => return Ok(Err(f)),
            },
        };
        parsed[1] = sam::parse(&text).unwrap_or_default();
    }
    // No shaft under that name: try other names, one paid call at a time, until one hits.
    for (key, prompt) in see::SHAFT_FALLBACKS {
        if !parsed[1].is_empty() || !with_shaft {
            break;
        }
        let text = match r2_text(&bucket, &sam_cache_key(photo, key)).await? {
            Some(t) => t,
            None => match sam_call(env, &db, &bucket, session_id, photo, key, prompt).await? {
                Ok(t) => t,
                Err(f) => return Ok(Err(f)),
            },
        };
        parsed[1] = sam::parse(&text).unwrap_or_default();
    }
    let coin_mm = coins::find(scale_ref)
        .map(|c| c.diameter_mm)
        .unwrap_or(23.25);
    let ruler_obj = see::best_object(&parsed[3]);
    let ruler_fit = match ruler_obj {
        Some(o) if ruler_auto => photo_bytes(&bucket, photo)
            .await?
            .and_then(|b| crate::ruler::decode_jpeg_gray(&b))
            .and_then(|img| {
                crate::ruler::read_ruler(&img, o, focus.or_else(|| see::shaft_centre(&parsed[1])))
            }),
        _ => None,
    };
    let source = match taps {
        Some(r) if ruler => ScaleSource::Ruler(r),
        _ if ruler => ScaleSource::RulerFound {
            object: ruler_obj,
            fit: ruler_fit.as_ref(),
        },
        _ => ScaleSource::Coin {
            diameter_mm: coin_mm,
            objects: &parsed[0],
        },
    };
    Ok(Ok(see::assemble(source, &parsed[1], &parsed[2])))
}

fn budget_fail() -> Fail {
    (
        429,
        "BUDGET",
        "The demo has used its model budget for today. Try again tomorrow.".into(),
    )
}

fn photo_gone() -> Fail {
    (
        404,
        "NOT_FOUND",
        "The photo was deleted. Photos are kept for 7 days.".into(),
    )
}

/// POST /see: measure the first photo with the session's scale reference.
async fn run_see(env: &Env, mut s: Session) -> Result<Response> {
    let Some(photo) = s.photos.first().cloned() else {
        return error(400, "PHOTO_REQUIRED", "This session has no photo.");
    };
    let result = match measure_photo(
        env,
        &s.id,
        &photo,
        &s.coin_id,
        s.ruler.as_ref(),
        s.shaft_tap,
        true,
        None,
    )
    .await?
    {
        Ok(v) => v,
        Err(f) => return fail(f),
    };
    s.see = Some(result);
    s.stage = "see".into();
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// The raw SAM output for one concept, for transparency and test fixtures.
async fn sam_raw(env: &Env, s: &Session, key: &str) -> Result<Response> {
    // "2.ruler" reads the result for photo 2; a plain key is photo 1.
    let (n, key) = match key.split_once('.') {
        Some((n, k)) => (n.parse::<usize>().unwrap_or(0), k),
        None => (1, key),
    };
    let (Some(photo), true) = (
        n.checked_sub(1).and_then(|i| s.photos.get(i)),
        key.len() <= 40
            && key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
    ) else {
        return error(404, "NOT_FOUND", "Not found.");
    };
    match r2_text(&env.bucket(PHOTOS)?, &sam_cache_key(photo, key)).await? {
        Some(t) => {
            let mut r = Response::ok(t)?;
            r.headers_mut()
                .set("content-type", "text/plain; charset=utf-8")?;
            Ok(r)
        }
        None => error(404, "NOT_FOUND", "Not measured yet."),
    }
}

async fn set_ruler(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let v: Value = match req.json().await {
        Ok(v) => v,
        Err(_) => return error(400, "BAD_JSON", "Send p1, p2 and mm."),
    };
    let pt = |k: &str| {
        [
            v[k][0].as_f64().unwrap_or(f64::NAN),
            v[k][1].as_f64().unwrap_or(f64::NAN),
        ]
    };
    let Some(p) = s.photos.first() else {
        return error(400, "PHOTO_REQUIRED", "This session has no photo.");
    };
    let mm = v["mm"].as_f64().unwrap_or(f64::NAN);
    match RulerScale::validate(pt("p1"), pt("p2"), mm, p.width, p.height) {
        Ok(r) => {
            s.ruler = Some(r);
            s.coin_id = session::RULER.into();
            // A new scale makes old measurements stale.
            s.see = None;
            s.stage = "photo".into();
        }
        Err(m) => return error(400, "BAD_SCALE", &m),
    }
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// POST /shaft_tap {"x": .., "y": ..}: the user taps the shaft, SAM segments it there,
/// and the measuring runs again with that outline.
async fn shaft_tap(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let v: Value = req.json().await.unwrap_or(Value::Null);
    let Some(p) = s.photos.first() else {
        return error(400, "PHOTO_REQUIRED", "This session has no photo.");
    };
    let (x, y) = (
        v["x"].as_f64().unwrap_or(-1.0),
        v["y"].as_f64().unwrap_or(-1.0),
    );
    if !(x >= 0.0 && y >= 0.0 && x < p.width as f64 && y < p.height as f64) {
        return error(400, "BAD_POINT", "Tap on the photo.");
    }
    s.shaft_tap = Some([x.round() as u32, y.round() as u32]);
    run_see(env, s).await
}

async fn set_shaft(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let v: Value = match req.json().await {
        Ok(v) => v,
        Err(_) => return error(400, "BAD_JSON", "Send diameter_mm and flat_to_round_mm."),
    };
    let num = |k: &str| v[k].as_f64().unwrap_or(f64::NAN);
    match ManualShaft::validate(num("diameter_mm"), num("flat_to_round_mm")) {
        Ok(m) => s.shaft_manual = Some(m),
        Err(m) => return error(400, "BAD_SHAFT", &m),
    }
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// POST to the Any Part Made MCP endpoint, one JSON-RPC call.
async fn mcp_call(env: &Env, tool: &str, arguments: Value) -> std::result::Result<Value, String> {
    let url = env
        .var("MCP_URL")
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "https://mcp.anypartmade.com/mcp".into());
    let body = mcp::rpc(1, tool, arguments).to_string();
    // A busy core sometimes answers 502 or 503 without a body. The same request, unchanged,
    // is sent again after a short pause; any answer with content is final.
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            Delay::from(std::time::Duration::from_millis(1500 * attempt)).await;
        }
        let headers = Headers::new();
        let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| e.to_string());
        set("content-type", "application/json")?;
        set("accept", "application/json, text/event-stream")?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(JsValue::from_str(&body)));
        let req = Request::new_with_init(&url, &init).map_err(|e| e.to_string())?;
        let mut resp = match Fetch::Request(req).send().await {
            Ok(r) => r,
            Err(e) => {
                last = format!("Any Part Made not reachable: {e}");
                continue;
            }
        };
        let status = resp.status_code();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if let Some(v) = mcp::parse_body(&text) {
            return mcp::tool_result(&v);
        }
        last = format!("Any Part Made answered {status} with no JSON.");
        if ![502, 503, 504, 520, 522, 524].contains(&status) {
            break;
        }
        console_log!("mcp {tool}: {status}, trying again");
    }
    Err(last)
}

/// One check round: build the knob from the spec, store the 3MF and its picture, ask
/// Any Part Made. The shaft numbers come from the measuring step; a spec may not change
/// them. A known shaft length caps the bore depth so the knob does not rub on the panel.
async fn do_check(
    env: &Env,
    s: &mut Session,
    mut spec: KnobSpec,
    changes: Value,
) -> Result<std::result::Result<(), Fail>> {
    if s.rounds.len() >= session::MAX_ROUNDS {
        return Ok(Err((
            429,
            "TOO_MANY_ROUNDS",
            "This repair has used its check rounds. Start a new one.".into(),
        )));
    }
    let Some((d, f)) = s.shaft() else {
        return Ok(Err((
            400,
            "SHAFT_REQUIRED",
            "Measure the shaft first, or type it from calipers.".into(),
        )));
    };
    spec.bore.diameter_mm = d;
    spec.bore.flat_to_round_mm = f;
    if let Some(len) = s.dims.get("shaft_length_mm") {
        spec.bore.depth_mm = spec.bore.depth_mm.min(len.value - 1.0);
    }
    if let Err(m) = spec.validate() {
        return Ok(Err((400, "BAD_SPEC", m)));
    }
    let mesh = spec.mesh();
    let file = threemf::three_mf(&mesh, &s.line);
    let n = s.rounds.len() + 1;
    let bucket = env.bucket(PHOTOS)?;
    bucket
        .put(format!("models/{}/{n}.3mf", s.id), file.clone())
        .execute()
        .await?;
    let picture = render::png(&render::shade(&mesh, 480), 480, 480);
    bucket
        .put(format!("models/{}/{n}.png", s.id), picture)
        .execute()
        .await?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&file);
    let check = match mcp_call(env, "check_part", mcp::check_args(&spec, &s.line, &b64)).await {
        Ok(c) => c,
        Err(m) => return Ok(Err((502, "CHECK_ERROR", m))),
    };
    console_log!(
        "session {} round {n}: {}",
        s.id,
        check["verdict"].as_str().unwrap_or("?")
    );
    s.rounds.push(Round {
        n,
        at_ms: now_ms(),
        spec: serde_json::to_value(&spec)?,
        changes,
        check: mcp::summary(&check),
        file_bytes: file.len(),
    });
    s.spec = Some(spec);
    s.stage = "check".into();
    Ok(Ok(()))
}

async fn run_check(env: &Env, mut s: Session, spec: KnobSpec, changes: Value) -> Result<Response> {
    if let Err(f) = do_check(env, &mut s, spec, changes).await? {
        return fail(f);
    }
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// One Muse Spark call, cached by the hash of the request (same photo, same question,
/// same answer during development), budget checked and logged.
async fn muse_call(
    env: &Env,
    s: &Session,
    detail: &str,
    body: &Value,
) -> Result<std::result::Result<Value, Response>> {
    let bucket = env.bucket(PHOTOS)?;
    let key = format!(
        "muse/{}.json",
        hex::encode(Sha256::digest(body.to_string().as_bytes()))
    );
    if let Some(t) = r2_text(&bucket, &key).await? {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            return Ok(Ok(v));
        }
    }
    let db = env.d1("DB")?;
    // A design call with a photo costs about a cent; keep that much headroom.
    if spent_today(&db, session::day(now_ms())).await? + 0.02 > budget(env) {
        return Ok(Err(error(
            429,
            "BUDGET",
            "The demo has used its model budget for today. Try again tomorrow.",
        )?));
    }
    let reply = match meta_post(env, muse::URL, body).await {
        Ok(r) => r,
        Err(m) => return Ok(Err(error(502, "META_ERROR", &m)?)),
    };
    log_spend(&db, muse::MODEL, &s.id, detail, muse::cost_usd(&reply)).await?;
    bucket
        .put(key, reply.to_string().into_bytes())
        .execute()
        .await?;
    Ok(Ok(reply))
}

/// Asks Muse, applies the measured shaft, and asks again once if Rust refuses the spec.
async fn muse_spec(
    env: &Env,
    s: &Session,
    detail: &str,
    mut body: Value,
) -> Result<std::result::Result<muse::Answer, Response>> {
    let (d, f) = s.shaft().unwrap_or((6.0, 4.6));
    for attempt in 0..2 {
        let reply = match muse_call(env, s, detail, &body).await? {
            Ok(r) => r,
            Err(r) => return Ok(Err(r)),
        };
        let problem = match muse::parse(&reply) {
            Ok(mut a) => {
                a.spec.bore.diameter_mm = d;
                a.spec.bore.flat_to_round_mm = f;
                match a.spec.validate() {
                    Ok(()) => return Ok(Ok(a)),
                    Err(m) => m,
                }
            }
            Err(m) => m,
        };
        if attempt == 1 {
            return Ok(Err(error(502, "MUSE_SPEC", &problem)?));
        }
        // Tell Muse what Rust refused; the question changes, so the cache does too.
        let content = &reply["choices"][0]["message"]["content"];
        if let Some(m) = body["messages"].as_array_mut() {
            if content.is_string() {
                m.push(json!({"role": "assistant", "content": content}));
                m.push(json!({"role": "user", "content": format!("Rust refused that spec: {problem} Send a corrected spec.")}));
            } else {
                m.push(json!({"role": "user", "content": "Your last answer was empty. Answer with the JSON only, and keep your reasoning short."}));
            }
        }
    }
    unreachable!()
}

/// What the design step knows: shaft numbers and where they came from, knob and scale.
fn design_facts(s: &Session) -> Value {
    let see = s.see.clone().unwrap_or(Value::Null);
    json!({
        "shaft": s.shaft().map(|(d, f)| json!({
            "diameter_mm": d,
            "flat_to_round_mm": f,
            "source": if s.shaft_manual.is_some() { "calipers" } else { "photo" },
        })),
        "sibling_knob": see["knob"],
        "scale": see["scale"],
    })
}

/// POST /design: Muse designs the knob from the photo, then round 1 is checked.
async fn design_route(env: &Env, s: Session) -> Result<Response> {
    if s.shaft().is_none() {
        return error(
            400,
            "SHAFT_REQUIRED",
            "Measure the shaft first, or type it from calipers.",
        );
    }
    let Some(photo) = s.photos.first() else {
        return error(400, "PHOTO_REQUIRED", "This session has no photo.");
    };
    let Some(obj) = env.bucket(PHOTOS)?.get(&photo.key).execute().await? else {
        return error(
            404,
            "NOT_FOUND",
            "The photo was deleted. Photos are kept for 7 days.",
        );
    };
    let bytes = match obj.body() {
        Some(b) => b.bytes().await?,
        None => Vec::new(),
    };
    let data_url = format!(
        "data:{};base64,{}",
        photo.content_type,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );
    let (d, f) = s.shaft().unwrap_or((6.0, 4.6));
    let body = muse::design_request(
        &s.line,
        &design_facts(&s),
        &data_url,
        &KnobSpec::around_shaft(d, f),
    );
    let a = match muse_spec(env, &s, "design", body).await? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    let mut s = s;
    s.rounds.clear();
    s.quote = None;
    s.order = None;
    run_check(
        env,
        s,
        a.spec,
        json!({"by": "muse", "summary": a.summary, "changes": a.changes}),
    )
    .await
}

/// POST /fix: Muse reads the last check's issues and fixes, then the new spec is checked.
async fn fix_route(env: &Env, s: Session) -> Result<Response> {
    let Some(last) = s.rounds.last() else {
        return error(400, "DESIGN_FIRST", "Design the knob first.");
    };
    match last.check["verdict"].as_str() {
        Some("ok") => return error(409, "ALREADY_OK", "The part already passed the check."),
        // A refusal is final and shown as is. Never ask again with other words.
        Some("refused") => {
            return error(
                409,
                "REFUSED",
                "Any Part Made refused this part. See the reason above.",
            )
        }
        Some("needs_review") => {
            return error(
                409,
                "NEEDS_REVIEW",
                "A person at Any Part Made reviews this part.",
            )
        }
        _ => {}
    }
    if s.rounds.len() > muse::MAX_FIXES {
        return error(
            409,
            "ROUNDS_USED",
            "Four fix rounds are used. See what is left above.",
        );
    }
    let Ok(last_spec) = serde_json::from_value::<KnobSpec>(last.spec.clone()) else {
        return error(
            409,
            "NOT_A_KNOB",
            "This design was made in chat mode. Go on in the chat.",
        );
    };
    let body = muse::fix_request(&s.line, &last_spec, &last.check, "");
    let detail = format!("fix {}", s.rounds.len());
    let a = match muse_spec(env, &s, &detail, body).await? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    run_check(
        env,
        s,
        a.spec,
        json!({"by": "muse", "summary": a.summary, "changes": a.changes}),
    )
    .await
}

/// Prices for the last round, which must be ok.
async fn do_quote(
    env: &Env,
    s: &mut Session,
    country: &str,
) -> Result<std::result::Result<(), Fail>> {
    let country = country.to_uppercase();
    if !session::COUNTRIES.iter().any(|(c, _)| *c == country) {
        return Ok(Err((
            400,
            "BAD_COUNTRY",
            "Pick the country to ship to.".into(),
        )));
    }
    let Some(check_ids) = s
        .rounds
        .last()
        .filter(|r| r.check["verdict"] == "ok")
        .map(|r| match r.check["check_ids"].as_array() {
            Some(a) => Value::Array(a.clone()),
            None => json!([r.check["check_id"]]),
        })
    else {
        return Ok(Err((
            409,
            "NOT_OK",
            "The part has to pass the check before it can be quoted.".into(),
        )));
    };
    let q = match mcp_call(
        env,
        "quote_part",
        json!({"check_ids": check_ids, "ship_to_country": country}),
    )
    .await
    {
        Ok(q) => q,
        Err(m) => return Ok(Err((502, "QUOTE_ERROR", m))),
    };
    s.quote = Some(mcp::quote_view(&q));
    s.order = None;
    s.stage = "quote".into();
    Ok(Ok(()))
}

/// POST /quote {"country": "DE"}.
async fn quote_route(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let body: Value = req.json().await.unwrap_or(Value::Null);
    let country = body["country"].as_str().unwrap_or("DE").to_string();
    if let Err(f) = do_quote(env, &mut s, &country).await? {
        return fail(f);
    }
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// The approval link on anypartmade.com. Nothing is ordered or charged here; the user
/// approves there.
async fn do_order(
    env: &Env,
    s: &mut Session,
    option: &str,
) -> Result<std::result::Result<(), Fail>> {
    if !mcp::OPTIONS.contains(&option) {
        return Ok(Err((400, "BAD_OPTION", "Pick one of the options.".into())));
    }
    let Some(quote_id) = s
        .quote
        .as_ref()
        .and_then(|q| q["quote_id"].as_str())
        .map(str::to_owned)
    else {
        return Ok(Err((409, "QUOTE_FIRST", "Get the price first.".into())));
    };
    let o = match mcp_call(
        env,
        "create_order_link",
        json!({"quote_id": quote_id, "option_id": option, "via": "link"}),
    )
    .await
    {
        Ok(o) => o,
        Err(m) => return Ok(Err((502, "ORDER_ERROR", m))),
    };
    s.order = Some(mcp::order_view(&o));
    Ok(Ok(()))
}

/// POST /order {"option_id": "opt_standard"}.
async fn order_route(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let body: Value = req.json().await.unwrap_or(Value::Null);
    let option = body["option_id"].as_str().unwrap_or("").to_string();
    if let Err(f) = do_order(env, &mut s, &option).await? {
        return fail(f);
    }
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// POST /photo: one more photo for agent mode. Returns its number.
async fn add_photo(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    if s.photos.len() >= agent::MAX_PHOTOS {
        return error(
            409,
            "TOO_MANY_PHOTOS",
            "Six photos is the most for one repair.",
        );
    }
    let form = match req.form_data().await {
        Ok(f) => f,
        Err(_) => return error(400, "BAD_FORM", "Send the photo as a form upload."),
    };
    let Some(bytes) = file_field(&form, "photo").await? else {
        return error(400, "PHOTO_REQUIRED", "Take or choose the photo first.");
    };
    match store_photo(env, bytes).await {
        Ok(p) => s.photos.push(p),
        Err(m) => return error(400, "BAD_PHOTO", &m),
    }
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// Measures any object in a photo: SAM finds it by a short noun phrase or a point, Rust
/// gives its length, width and round diameter against the photo's ruler or coin.
#[allow(clippy::too_many_arguments)]
async fn measure_object(
    env: &Env,
    s: &Session,
    photo: &PhotoRef,
    scale_ref: &str,
    concept: Option<&str>,
    point: Option<[u32; 2]>,
    bbox: Option<[u32; 4]>,
    taps: Option<&RulerScale>,
    alternatives: &[String],
) -> Result<std::result::Result<Value, Fail>> {
    let db = env.d1("DB")?;
    let bucket = env.bucket(PHOTOS)?;
    let (key, prompt) = match (bbox, point, concept) {
        (Some([x1, y1, x2, y2]), _, _) => (
            format!("obj_box_{x1}_{y1}_{x2}_{y2}"),
            sam::box_prompt(x1, y1, x2, y2, photo.width, photo.height),
        ),
        (None, Some([x, y]), _) => (
            format!("obj_pt_{x}_{y}"),
            sam::point_prompt(x, y, photo.width, photo.height),
        ),
        (None, None, Some(c)) => {
            let c: String = c
                .chars()
                .filter(|ch| ch.is_alphanumeric() || *ch == ' ')
                .take(40)
                .collect();
            let c = c.trim().to_lowercase();
            if c.is_empty() {
                return Ok(Err((
                    400,
                    "BAD_OBJECT",
                    "Name the object in a few words, or give a point.".into(),
                )));
            }
            (format!("obj_{}", c.replace(' ', "_")), c)
        }
        (None, None, None) => {
            return Ok(Err((
                400,
                "BAD_OBJECT",
                "Name the object, or give a point or box on it.".into(),
            )))
        }
    };
    let text = match r2_text(&bucket, &sam_cache_key(photo, &key)).await? {
        Some(t) => t,
        None => match sam_call(env, &db, &bucket, &s.id, photo, &key, &prompt).await? {
            Ok(t) => t,
            Err(f) => return Ok(Err(f)),
        },
    };
    let mut objs = sam::parse(&text).unwrap_or_default();
    // The word search depends on the phrase: Muse gives other names for the same thing,
    // tried in order until one finds it.
    if objs.is_empty() && point.is_none() && bbox.is_none() {
        for alt in alternatives.iter().take(3) {
            let alt: String = alt
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ')
                .take(40)
                .collect();
            let alt = alt.trim().to_lowercase();
            if alt.is_empty() || alt == prompt {
                continue;
            }
            let key = format!("obj_{}", alt.replace(' ', "_"));
            let text = match r2_text(&bucket, &sam_cache_key(photo, &key)).await? {
                Some(t) => t,
                None => match sam_call(env, &db, &bucket, &s.id, photo, &key, &alt).await? {
                    Ok(t) => t,
                    Err(f) => return Ok(Err(f)),
                },
            };
            objs = sam::parse(&text).unwrap_or_default();
            if !objs.is_empty() {
                break;
            }
        }
    }
    let Some(o) = see::best_object(&objs) else {
        return Ok(Ok(
            json!({"found": false, "note": "SAM found nothing there. Try other words, or a point on the object."}),
        ));
    };
    let Some(size) = crate::measure::object_size(o) else {
        return Ok(Ok(
            json!({"found": false, "note": "The outline is too small to measure."}),
        ));
    };
    let see = match measure_photo(
        env,
        &s.id,
        photo,
        scale_ref,
        taps,
        None,
        false,
        Some((size.cx, size.cy)),
    )
    .await?
    {
        Ok(v) => v,
        Err(f) => return Ok(Err(f)),
    };
    let k = see["scale"]["mm_per_px"].as_f64();
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    let mm = |px: f64| k.map(|k| r2(px * k));
    let outline = crate::measure::outline(&crate::measure::edge_points(o), size.cx, size.cy, 160);
    Ok(Ok(json!({
        "found": true,
        "confidence": o.confidence,
        "scale": see["scale"],
        "length_mm": mm(size.length_px),
        "width_mm": mm(size.width_px),
        "round_diameter_mm": size.circle_px.and_then(mm),
        "angle_deg": r2(size.angle.to_degrees()),
        "rectangle_fill": r2(size.fill),
        "box_px": [o.x1, o.y1, o.x2, o.y2],
        "box_mm": k.map(|k| [r2((o.x2 - o.x1) as f64 * k), r2((o.y2 - o.y1) as f64 * k)]),
        "notes": see["notes"].as_array().map(|a| a.iter().filter(|n| {
            let t = n.as_str().unwrap_or("");
            !t.contains("shaft") && !t.contains("knob")
        }).cloned().collect::<Vec<_>>()),
        "overlays": [{"label": "knob", "outline": outline}],
    })))
}

fn dim_values(s: &Session) -> std::collections::BTreeMap<String, f64> {
    s.dims.iter().map(|(k, d)| (k.clone(), d.value)).collect()
}

/// The page built the waiting design (each part's 3MF and picture, in WebAssembly on
/// the phone): store the files, have Any Part Made check every part (later parts with
/// the parts they fit onto as mates), and answer Muse's build_and_check call. Checks
/// and poses come from the design here, not from the page.
async fn build_route(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let Some(b) = s.build.clone() else {
        return error(
            409,
            "NOTHING_TO_BUILD",
            "There is no part waiting to be built.",
        );
    };
    let form = match req.form_data().await {
        Ok(f) => f,
        Err(_) => {
            return error(
                400,
                "BAD_FORM",
                "Send the models and pictures as a form upload.",
            )
        }
    };
    let call_id = b["call_id"].clone();
    let finish = |s: &mut Session, result: Value| {
        s.chat
            .push(json!({"role": "tool", "tool_call_id": call_id, "content": result.to_string()}));
        s.build = None;
        s.agent = "working".into();
    };
    // The device could not build it: Muse gets the reason and fixes the design.
    let failed = text_field(&form, "error");
    if !failed.is_empty() {
        let msg: String = failed.chars().take(300).collect();
        finish(&mut s, json!({"error": msg, "code": "BUILD_FAILED"}));
        save(&env.d1("DB")?, &s).await?;
        return json_response(200, &s.state_json());
    }
    let a = match crate::assembly::parse(&b["design"], &dim_values(&s)) {
        Ok(a) => a,
        Err(m) => {
            finish(&mut s, json!({"error": m, "code": "BAD_PART"}));
            save(&env.d1("DB")?, &s).await?;
            return json_response(200, &s.state_json());
        }
    };
    let mut files = Vec::new();
    for k in 0..a.parts.len() {
        let (Some(file), Some(picture)) = (
            file_field(&form, &format!("model_{k}")).await?,
            file_field(&form, &format!("picture_{k}")).await?,
        ) else {
            return error(
                400,
                "BAD_FORM",
                "Send a model and a picture for every part.",
            );
        };
        if !file.starts_with(b"PK\x03\x04")
            || file.len() > 24 * 1024 * 1024
            || !picture.starts_with(b"\x89PNG")
            || picture.len() > 2 * 1024 * 1024
        {
            return error(
                400,
                "BAD_MODEL",
                "The built files are not a 3MF and a PNG of a fitting size.",
            );
        }
        files.push((file, picture));
    }
    let n = s.rounds.len() + 1;
    let bucket = env.bucket(PHOTOS)?;
    let base = env
        .var("PUBLIC_URL")
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "https://studio.anypartmade.com".into());
    let material = b["material"].as_str().unwrap_or("petg").to_string();
    let use_description = b["use_description"].as_str().unwrap_or(&s.line).to_string();
    let mut results: Vec<Value> = Vec::new();
    let mut file_ids: Vec<Option<String>> = Vec::new();
    let mut total = 0;
    for (k, ((file, picture), p)) in files.into_iter().zip(&a.parts).enumerate() {
        let stem = if k == 0 {
            format!("{n}")
        } else {
            format!("{n}_{k}")
        };
        let bytes = file.len();
        total += bytes;
        let b64 =
            (bytes <= 480 * 1024).then(|| base64::engine::general_purpose::STANDARD.encode(&file));
        bucket
            .put(format!("models/{}/{stem}.3mf", s.id), file)
            .execute()
            .await?;
        bucket
            .put(format!("models/{}/{stem}.png", s.id), picture)
            .execute()
            .await?;
        let url = format!("{base}/api/session/{}/model/{stem}.3mf", s.id);
        let mut args = mcp::check_args_part(
            &format!("{use_description} (part {}: {})", k + 1, p.name),
            &s.line,
            if p.material == "asa" {
                "asa"
            } else {
                &material
            },
            b64.as_deref().ok_or(url.as_str()),
            &p.part.critical_dimensions,
            &p.part.interfaces,
        );
        // Fit against the parts it slides onto, already checked and stored.
        let mates: Vec<Value> = p
            .mates
            .iter()
            .filter_map(|m| {
                let id = file_ids.get(m.with)?.clone()?;
                let (transform, dir) = a.mate_for_check(k, m);
                Some(
                    json!({"file_id": id, "name": a.parts[m.with].name, "transform": transform,
                    "insertion": {"direction": dir}, "min_clearance_mm": m.min_clearance_mm}),
                )
            })
            .collect();
        if !mates.is_empty() {
            args["part"]["mates"] = json!(mates);
        }
        let check = match mcp_call(env, "check_part", args).await {
            Ok(c) => c,
            Err(m) => json!({"verdict": "error", "error": m}),
        };
        file_ids.push(check["file_id"].as_str().map(str::to_owned));
        console_log!(
            "session {} round {n} part {}: {}",
            s.id,
            p.name,
            check["verdict"].as_str().unwrap_or("?")
        );
        results.push(json!({"name": p.name, "check": mcp::summary(&check), "model": format!("/api/session/{}/model/{stem}.3mf", s.id), "picture": format!("/api/session/{}/model/{stem}.png", s.id), "bytes": bytes}));
    }
    // The design passes when every part passes; otherwise the most serious verdict.
    let rank = |v: &str| match v {
        "refused" => 5,
        "error" => 4,
        "needs_review" => 3,
        "needs_changes" => 2,
        "ok" => 0,
        _ => 1,
    };
    let verdict = results
        .iter()
        .map(|r| {
            r["check"]["verdict"]
                .as_str()
                .unwrap_or("error")
                .to_string()
        })
        .max_by_key(|v| rank(v))
        .unwrap_or_else(|| "error".into());
    let several = results.len() > 1;
    let tag = |name: &Value, text: &Value| -> Value {
        if several {
            json!(format!(
                "[{}] {}",
                name.as_str().unwrap_or(""),
                text.as_str().unwrap_or("")
            ))
        } else {
            text.clone()
        }
    };
    let mut issues = Vec::new();
    let mut dims = Vec::new();
    let mut interfaces = Vec::new();
    for r in &results {
        for i in r["check"]["manufacturability"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut i = i.clone();
            i["part"] = r["name"].clone();
            i["message"] = tag(&r["name"], &i["message"]);
            issues.push(i);
        }
        if let Some(e) = r["check"]["error"].as_str() {
            issues.push(json!({"severity": "error", "code": "CHECK_ERROR", "part": r["name"], "message": tag(&r["name"], &json!(e)), "fix": "Send the design again."}));
        }
        for d in r["check"]["guarantee_preview"]["dimensions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut d = d.clone();
            d["label"] = tag(&r["name"], &d["label"]);
            dims.push(d);
        }
        for i in r["check"]["guarantee_preview"]["interfaces"]
            .as_array()
            .into_iter()
            .flatten()
        {
            interfaces.push(i.clone());
        }
    }
    let check_ids: Vec<Value> = results
        .iter()
        .map(|r| r["check"]["check_id"].clone())
        .collect();
    let combined = json!({
        "verdict": verdict,
        "check_id": check_ids.first().cloned().unwrap_or(Value::Null),
        "check_ids": check_ids,
        "manufacturability": issues,
        "guarantee_preview": {"dimensions": dims, "interfaces": interfaces},
        "parts": results,
        "refusal": results.iter().find_map(|r| (!r["check"]["refusal"].is_null()).then(|| r["check"]["refusal"].clone())),
    });
    s.rounds.push(Round {
        n,
        at_ms: now_ms(),
        spec: json!({"design": b["design"], "view": a.view(), "fitted": a.parts.iter().map(|p| json!({"part": p.name, "fitted": p.part.fitted})).collect::<Vec<_>>()}),
        changes: b["changes"].clone(),
        check: combined.clone(),
        file_bytes: total,
    });
    s.stage = "check".into();
    let tool = json!({
        "round": n,
        "verdict": verdict,
        "parts": results.iter().map(|r| json!({
            "name": r["name"],
            "verdict": r["check"]["verdict"],
            "issues": r["check"]["manufacturability"].as_array().map(|a| a.iter().filter(|i| i["severity"] != "info").map(|i| json!({"severity": i["severity"], "code": i["code"], "message": i["message"], "fix": i["fix"]})).collect::<Vec<_>>()),
            "guarantee": r["check"]["guarantee_preview"]["dimensions"],
            "interfaces": r["check"]["guarantee_preview"]["interfaces"],
            "error": r["check"]["error"],
        })).collect::<Vec<_>>(),
        "fitted_from_measurements": a.parts.iter().map(|p| json!({"part": p.name, "fitted": p.part.fitted})).collect::<Vec<_>>(),
        "refusal": combined["refusal"],
    });
    finish(&mut s, tool);
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

fn tool_error(f: Fail) -> Value {
    json!({"error": f.2, "code": f.1})
}

/// Runs one tool call of the agent and returns what the model reads back.
async fn run_tool(env: &Env, s: &mut Session, name: &str, args: &Value) -> Result<Value> {
    Ok(match name {
        "measure_photo" => {
            let n = args["photo"].as_u64().unwrap_or(0) as usize;
            let Some(photo) = n.checked_sub(1).and_then(|i| s.photos.get(i)).cloned() else {
                return Ok(json!({"error": format!("There is no photo {n}.")}));
            };
            let scale = args["scale"].as_str().unwrap_or(session::RULER);
            if session::scale_ref(scale).is_err() {
                return Ok(json!({"error": "scale must be ruler or a coin id"}));
            }
            let point = match (
                args["shaft_point"]["x"].as_u64(),
                args["shaft_point"]["y"].as_u64(),
            ) {
                (Some(x), Some(y)) if (x as u32) < photo.width && (y as u32) < photo.height => {
                    Some([x as u32, y as u32])
                }
                (Some(_), Some(_)) => {
                    return Ok(json!({"error": format!(
                        "shaft_point lies outside photo {n}, which is {} x {} px.",
                        photo.width, photo.height
                    )}))
                }
                _ => None,
            };
            let marks = s.ruler_marks.get(&n.to_string()).cloned();
            match measure_photo(env, &s.id, &photo, scale, marks.as_ref(), point, true, None)
                .await?
            {
                Ok(see) => {
                    let id = format!("m{}", s.measurements.len() + 1);
                    let view = agent::measurement_view(&id, n, &see);
                    let mut stored = view.clone();
                    stored["overlays"] = see["overlays"].clone();
                    s.measurements.push(stored);
                    if n == 1 {
                        s.see = Some(see);
                    }
                    view
                }
                Err(f) => tool_error(f),
            }
        }
        "set_dimension" => {
            let words = agent::user_words(&s.chat);
            match agent::dimension(args, &s.measurements, &words) {
                Ok((k, d)) => {
                    let v = json!({"ok": true, "name": k, "value": d.value, "source": d.source});
                    s.dims.insert(k, d);
                    v
                }
                Err(m) => json!({"error": m}),
            }
        }
        "measure_object" => {
            let n = args["photo"].as_u64().unwrap_or(0) as usize;
            let Some(photo) = n.checked_sub(1).and_then(|i| s.photos.get(i)).cloned() else {
                return Ok(json!({"error": format!("There is no photo {n}.")}));
            };
            let scale = args["scale"].as_str().unwrap_or(session::RULER);
            if session::scale_ref(scale).is_err() {
                return Ok(json!({"error": "scale must be ruler or a coin id"}));
            }
            let point = match (args["point"]["x"].as_u64(), args["point"]["y"].as_u64()) {
                (Some(x), Some(y)) if (x as u32) < photo.width && (y as u32) < photo.height => {
                    Some([x as u32, y as u32])
                }
                (Some(_), Some(_)) => {
                    return Ok(
                        json!({"error": format!("point lies outside photo {n}, which is {} x {} px.", photo.width, photo.height)}),
                    )
                }
                _ => None,
            };
            let b = &args["box"];
            let bbox = match (
                b["x1"].as_u64(),
                b["y1"].as_u64(),
                b["x2"].as_u64(),
                b["y2"].as_u64(),
            ) {
                (Some(x1), Some(y1), Some(x2), Some(y2)) => {
                    let (w, h) = (photo.width as u64, photo.height as u64);
                    if x1.max(x2) >= w
                        || y1.max(y2) >= h
                        || x1.abs_diff(x2) < 4
                        || y1.abs_diff(y2) < 4
                    {
                        return Ok(
                            json!({"error": format!("box must lie inside photo {n} ({w} x {h} px) and be more than a few pixels wide")}),
                        );
                    }
                    Some([x1 as u32, y1 as u32, x2 as u32, y2 as u32])
                }
                _ => None,
            };
            let alternatives: Vec<String> = args["alternatives"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            match measure_object(
                env,
                s,
                &photo,
                scale,
                args["object"].as_str(),
                point,
                bbox,
                s.ruler_marks.get(&n.to_string()).cloned().as_ref(),
                &alternatives,
            )
            .await?
            {
                Ok(mut v) => {
                    let id = format!("m{}", s.measurements.len() + 1);
                    let overlays = v["overlays"].take();
                    if let Some(o) = v.as_object_mut() {
                        o.remove("overlays");
                    }
                    let view = json!({"id": id, "photo": n, "result": v});
                    let mut stored = view.clone();
                    stored["overlays"] = overlays;
                    s.measurements.push(stored);
                    view
                }
                Err(f) => tool_error(f),
            }
        }
        "build_and_check" => {
            if s.rounds.len() >= session::MAX_ROUNDS {
                return Ok(json!({"error": "This repair has used its check rounds."}));
            }
            let design = if args["design"].is_object() {
                &args["design"]
            } else {
                &args["part"]
            };
            match crate::assembly::parse(design, &dim_values(s)) {
                // Valid: the page builds it and calls /build, which answers this call.
                Ok(_) => Value::Null,
                Err(m) => json!({"error": m, "code": "BAD_PART"}),
            }
        }
        "quote" => match do_quote(env, s, args["country"].as_str().unwrap_or("DE")).await? {
            Ok(()) => {
                let q = s.quote.as_ref().expect("quote");
                json!({"options": q["options"], "guarantee": q["guarantee"], "notes": q["notes"]})
            }
            Err(f) => tool_error(f),
        },
        "order_link" => match do_order(env, s, args["option_id"].as_str().unwrap_or("")).await? {
            Ok(()) => {
                let o = s.order.as_ref().expect("order");
                json!({"order_url": o["order_url"], "price": o["price_display"], "note": "The user approves on anypartmade.com. Nothing is ordered yet."})
            }
            Err(f) => tool_error(f),
        },
        "order_status" => {
            let Some(id) = s
                .order
                .as_ref()
                .and_then(|o| o["order_id"].as_str())
                .map(str::to_owned)
            else {
                return Ok(json!({"error": "There is no approval link yet."}));
            };
            match mcp_call(env, "order_status", json!({"order_id": id})).await {
                Ok(o) => {
                    let v = mcp::order_view(&o);
                    if let Some(ord) = s.order.as_mut() {
                        ord["status"] = v["status"].clone();
                        ord["status_label"] = v["status_label"].clone();
                    }
                    json!({"status": o["status"], "status_label": o["status_label"], "test_mode": o["test_mode"], "history": o["history"], "next_step": o["next_step"]})
                }
                Err(m) => json!({"error": m}),
            }
        }
        _ => json!({"error": format!("unknown tool {name}")}),
    })
}

/// POST /agent {"text", "photos": [n], "tap": {"photo", "x", "y"}}: adds the user's
/// message if any, then runs one model step. The page calls again while the state says
/// "working", so every tool call shows as it happens.
async fn agent_route(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let body: Value = req.json().await.unwrap_or(Value::Null);
    let text = body["text"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(2000)
        .collect::<String>();
    let photos: Vec<(usize, u32, u32)> = body["photos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_u64())
        .filter_map(|n| {
            s.photos
                .get((n as usize).checked_sub(1)?)
                .map(|p| (n as usize, p.width, p.height))
        })
        .collect();
    let tap = match (
        body["tap"]["photo"].as_u64(),
        body["tap"]["x"].as_u64(),
        body["tap"]["y"].as_u64(),
    ) {
        (Some(n), Some(x), Some(y)) => {
            let second = match (body["tap"]["x2"].as_u64(), body["tap"]["y2"].as_u64()) {
                (Some(a), Some(b)) => Some((a as u32, b as u32)),
                _ => None,
            };
            Some((n as usize, x as u32, y as u32, second))
        }
        _ => None,
    };
    // Two taps on the ruler with the distance between them: that photo's exact scale.
    let mut text = text;
    let mut tap = tap;
    if body["tap"]["kind"] == "ruler" {
        if let Some((n, x, y, Some((x2, y2)))) = tap {
            let mm = body["tap"]["mm"].as_f64().unwrap_or(50.0);
            let Some(p) = n.checked_sub(1).and_then(|i| s.photos.get(i)) else {
                return error(400, "BAD_POINT", "No such photo.");
            };
            match RulerScale::validate(
                [x as f64, y as f64],
                [x2 as f64, y2 as f64],
                mm,
                p.width,
                p.height,
            ) {
                Ok(r) => {
                    s.ruler_marks.insert(n.to_string(), r);
                    text.push_str(&format!(
                        "\n[The user marked two ruler marks {mm} mm apart on photo {n}. Its scale now comes from these marks; measure photo {n} again.]"
                    ));
                    tap = None;
                }
                Err(m) => return error(400, "BAD_RULER", &m),
            }
        }
    }
    if !text.trim().is_empty() || !photos.is_empty() || tap.is_some() {
        if text.chars().any(|c| c.is_control() && !c.is_whitespace()) {
            return error(400, "BAD_TEXT", "The message has characters we cannot use.");
        }
        s.chat.push(agent::user_message(&text, &photos, tap));
    } else if s.agent != "working" {
        return json_response(200, &s.state_json());
    }
    if s.build.is_some() {
        return error(
            409,
            "BUILDING",
            "The part is being built on your device. Wait for it.",
        );
    }
    let since_user = s
        .chat
        .iter()
        .rev()
        .take_while(|m| m["role"] != "user")
        .filter(|m| m["role"] == "assistant")
        .count();
    if since_user >= agent::MAX_STEPS {
        s.chat.push(json!({"role": "assistant", "content": "I stopped here to keep this repair short. Tell me how you want to go on."}));
        s.agent = "waiting".into();
        save(&env.d1("DB")?, &s).await?;
        return json_response(200, &s.state_json());
    }
    // Photos go to Muse inline as data URLs: Meta's fetcher does not reliably reach our
    // links, and the Worker has the CPU for the encoding.
    let bucket = env.bucket(PHOTOS)?;
    let mut urls: Vec<Option<String>> = Vec::new();
    for p in &s.photos {
        urls.push(photo_bytes(&bucket, p).await?.map(|b| {
            format!(
                "data:{};base64,{}",
                p.content_type,
                base64::engine::general_purpose::STANDARD.encode(&b)
            )
        }));
    }
    let messages = agent::materialize(&s.chat, &|n| urls.get(n.checked_sub(1)?).cloned().flatten());
    // Talking and measuring need little thinking; designing and fixing need more.
    let designing = !s.rounds.is_empty()
        || s.dims.contains_key("flat_to_round_mm")
        || s.dims.contains_key("board_height_mm")
        || s.dims.contains_key("board_width_mm") && s.chat.len() > 12;
    let body = agent::request(messages, if designing { "medium" } else { "low" });
    let reply = match muse_call(env, &s, "agent", &body).await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let msg = &reply["choices"][0]["message"];
    let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
    if calls.is_empty() {
        let text = msg["content"].as_str().unwrap_or("").trim().to_string();
        let text = if text.is_empty() {
            "I lost my train of thought there. Send me a short message and I will go on."
                .to_string()
        } else {
            text
        };
        s.chat.push(json!({"role": "assistant", "content": text}));
        s.agent = "waiting".into();
    } else {
        s.chat
            .push(json!({"role": "assistant", "content": msg["content"], "tool_calls": calls}));
        for c in &calls {
            let name = c["function"]["name"].as_str().unwrap_or("");
            let args: Value =
                serde_json::from_str(c["function"]["arguments"].as_str().unwrap_or("{}"))
                    .unwrap_or(Value::Null);
            let result = run_tool(env, &mut s, name, &args).await?;
            if name == "build_and_check" && result.is_null() {
                s.build = Some(json!({
                    "call_id": c["id"],
                    "design": if args["design"].is_object() { args["design"].clone() } else { args["part"].clone() },
                    "material": args["material"],
                    "use_description": args["use_description"],
                    "changes": {"by": "muse", "summary": args["summary"], "changes": args["changes"]},
                }));
                continue;
            }
            s.chat.push(
                json!({"role": "tool", "tool_call_id": c["id"], "content": result.to_string()}),
            );
        }
        s.agent = if s.build.is_some() {
            "building"
        } else {
            "working"
        }
        .into();
    }
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// POST /order_status: the order's real state from Any Part Made, for the page to show
/// after the user comes back from the approval page.
async fn order_status_route(env: &Env, mut s: Session) -> Result<Response> {
    let Some(id) = s
        .order
        .as_ref()
        .and_then(|o| o["order_id"].as_str())
        .map(str::to_owned)
    else {
        return error(409, "NO_ORDER", "There is no approval link yet.");
    };
    match mcp_call(env, "order_status", json!({"order_id": id})).await {
        Ok(o) => {
            if let Some(ord) = s.order.as_mut() {
                ord["status"] = o["status"].clone();
                ord["status_label"] = o["status_label"].clone();
            }
            save(&env.d1("DB")?, &s).await?;
            json_response(200, &s.state_json())
        }
        Err(m) => error(502, "STATUS_ERROR", &m),
    }
}

/// POST /check with an optional `{"spec": ...}`: checks that spec, else the current one,
/// else a plain knob around the measured shaft.
async fn check_route(mut req: Request, env: &Env, s: Session) -> Result<Response> {
    let body: Value = req.json().await.unwrap_or(Value::Null);
    let spec = match body.get("spec") {
        Some(v) => match serde_json::from_value::<KnobSpec>(v.clone()) {
            Ok(k) => k,
            Err(e) => return error(400, "BAD_SPEC", &format!("The knob spec is not valid: {e}")),
        },
        None => match (&s.spec, s.shaft()) {
            (Some(k), _) => k.clone(),
            (None, Some((d, f))) => KnobSpec::around_shaft(d, f),
            (None, None) => KnobSpec::around_shaft(6.0, 4.6),
        },
    };
    run_check(env, s, spec, Value::Null).await
}

async fn get_model(env: &Env, s: &Session, name: &str) -> Result<Response> {
    let (n, ext, content_type) = match name.strip_suffix(".png") {
        Some(n) => (n, "png", "image/png"),
        None => (
            name.strip_suffix(".3mf").unwrap_or(name),
            "3mf",
            "model/3mf",
        ),
    };
    // "3" is round 3's first part, "3_1" its second.
    let (n, part) = match n.split_once('_') {
        Some((a, k)) => (
            a,
            k.parse::<usize>()
                .ok()
                .map(|k| format!("_{k}"))
                .unwrap_or_default(),
        ),
        None => (n, String::new()),
    };
    let Some(n) = n
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1 && *n <= s.rounds.len() + 1)
    else {
        return error(404, "NOT_FOUND", "No such model.");
    };
    let Some(obj) = env
        .bucket(PHOTOS)?
        .get(format!("models/{}/{n}{part}.{ext}", s.id))
        .execute()
        .await?
    else {
        return error(404, "NOT_FOUND", "The model was deleted.");
    };
    let bytes = match obj.body() {
        Some(b) => b.bytes().await?,
        None => Vec::new(),
    };
    let mut r = Response::from_bytes(bytes)?;
    let h = r.headers_mut();
    h.set("content-type", content_type)?;
    if ext == "3mf" {
        h.set(
            "content-disposition",
            &format!("attachment; filename=\"knob-round-{n}.3mf\""),
        )?;
    }
    h.set("cache-control", "private, max-age=86400")?;
    h.set("x-content-type-options", "nosniff")?;
    Ok(r)
}

fn page_response(html: String, csp: String) -> Result<Response> {
    let mut r = Response::from_html(html)?;
    let h = r.headers_mut();
    h.set("content-security-policy", &csp)?;
    h.set("x-content-type-options", "nosniff")?;
    h.set("referrer-policy", "no-referrer")?;
    h.set("cache-control", "no-cache")?;
    Ok(r)
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let path = req.path();
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (req.method(), parts.as_slice()) {
        (Method::Get, [""]) => page_response(crate::agent_page::html(), crate::agent_page::csp()),
        (Method::Get, ["guided"]) => page_response(page::html(), page::csp()),
        (Method::Get, ["mesher.wasm"]) => {
            let mut r = Response::from_bytes(MESHER.to_vec())?;
            r.headers_mut().set("content-type", "application/wasm")?;
            r.headers_mut()
                .set("cache-control", "public, max-age=300")?;
            Ok(r)
        }
        (Method::Get, ["health"]) => json_response(200, &json!({"ok": true})),
        (Method::Post, ["api", "session"]) => create_session(req, &env).await,
        (method, ["api", "session", id, rest @ ..]) => {
            if !session::valid_id(id) {
                return error(404, "NOT_FOUND", "No such session.");
            }
            let Some(s) = load(&env.d1("DB")?, id).await? else {
                return error(
                    404,
                    "NOT_FOUND",
                    "No such session. Sessions are kept for 7 days.",
                );
            };
            match (method, rest) {
                (Method::Get, []) => json_response(200, &s.state_json()),
                (Method::Get, ["photo", n]) => get_photo(&env, &s, n).await,
                (Method::Get, ["sam", key]) => sam_raw(&env, &s, key).await,
                (Method::Post, ["see"]) => run_see(&env, s).await,
                (Method::Post, ["shaft"]) => set_shaft(req, &env, s).await,
                (Method::Post, ["shaft_tap"]) => shaft_tap(req, &env, s).await,
                (Method::Post, ["scale"]) => set_ruler(req, &env, s).await,
                (Method::Post, ["check"]) => check_route(req, &env, s).await,
                (Method::Post, ["design"]) => design_route(&env, s).await,
                (Method::Post, ["fix"]) => fix_route(&env, s).await,
                (Method::Post, ["quote"]) => quote_route(req, &env, s).await,
                (Method::Post, ["order"]) => order_route(req, &env, s).await,
                (Method::Post, ["order_status"]) => order_status_route(&env, s).await,
                (Method::Post, ["photo"]) => add_photo(req, &env, s).await,
                (Method::Post, ["agent"]) => agent_route(req, &env, s).await,
                (Method::Post, ["build"]) => build_route(req, &env, s).await,
                (Method::Get, ["model", n]) => get_model(&env, &s, n).await,
                _ => error(404, "NOT_FOUND", "Not found."),
            }
        }
        _ => error(404, "NOT_FOUND", "Not found."),
    }
}

/// Daily cleanup: sessions and rate counters older than 7 days. Photos expire by an R2 rule.
#[event(scheduled)]
async fn scheduled(_ev: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let now = now_ms();
    let Ok(db) = env.d1("DB") else { return };
    let cutoff = session::expired_before(now) as f64;
    let today = session::day(now) as f64;
    for (sql, v) in [
        ("DELETE FROM sessions WHERE created_at < ?1", cutoff),
        ("DELETE FROM rate WHERE day < ?1 - 7", today),
    ] {
        if let Ok(stmt) = db.prepare(sql).bind(&[v.into()]) {
            if let Err(e) = stmt.run().await {
                console_error!("cleanup failed: {e}");
            }
        }
    }
}
