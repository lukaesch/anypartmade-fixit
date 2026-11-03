//! HTTP routes. Only glue lives here; checks and data shapes are in the plain modules.

use crate::knob::KnobSpec;
use crate::sam;
use crate::see::{self, ManualShaft, RulerScale, ScaleSource};
use crate::session::{self, PhotoRef, Round, Session};
use crate::{coins, mcp, muse, page, photo, threemf};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use worker::wasm_bindgen::JsValue;
use worker::*;

const PHOTOS: &str = "PHOTOS";

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
    let line = match session::clean_line(&text_field(&form, "line")) {
        Ok(l) => l,
        Err(m) => return error(400, "BAD_LINE", &m),
    };
    let coin = match session::scale_ref(&text_field(&form, "coin")) {
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
        spec: None,
        rounds: Vec::new(),
        quote: None,
        order: None,
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
/// The inner Err is the error response to send.
async fn sam_call(
    env: &Env,
    db: &D1Database,
    bucket: &Bucket,
    session_id: &str,
    photo: &PhotoRef,
    key: &str,
    prompt: &str,
) -> Result<std::result::Result<String, Response>> {
    let day = session::day(now_ms());
    if spent_today(db, day).await? + sam::USD_PER_IMAGE > budget(env) {
        return Ok(Err(error(
            429,
            "BUDGET",
            "The demo has used its model budget for today. Try again tomorrow.",
        )?));
    }
    let bytes = match bucket.get(&photo.key).execute().await? {
        Some(obj) => match obj.body() {
            Some(b) => b.bytes().await?,
            None => Vec::new(),
        },
        None => {
            return Ok(Err(error(
                404,
                "NOT_FOUND",
                "The photo was deleted. Photos are kept for 7 days.",
            )?))
        }
    };
    let data_url = format!(
        "data:{};base64,{}",
        photo.content_type,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );
    let reply = match meta_post(env, sam::URL, &sam::request(prompt, &data_url)).await {
        Ok(r) => r,
        Err(m) => return Ok(Err(error(502, "META_ERROR", &m)?)),
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
async fn run_see(env: &Env, mut s: Session) -> Result<Response> {
    let Some(photo) = s.photos.first().cloned() else {
        return error(400, "PHOTO_REQUIRED", "This session has no photo.");
    };
    let ruler = s.coin_id == session::RULER;
    // Tapped marks win; otherwise the ruler is found by SAM and read from its ticks.
    let ruler_auto = ruler && s.ruler.is_none();
    let db = env.d1("DB")?;
    let bucket = env.bucket(PHOTOS)?;
    let wanted: Vec<bool> = see::CONCEPTS
        .iter()
        .map(|(k, _)| match *k {
            "coin" => !ruler,
            "ruler" => ruler_auto,
            _ => true,
        })
        .collect();
    let mut texts: Vec<Option<String>> = Vec::new();
    for (i, (key, _)) in see::CONCEPTS.iter().enumerate() {
        texts.push(if wanted[i] {
            r2_text(&bucket, &sam_cache_key(&photo, key)).await?
        } else {
            Some(String::new())
        });
    }
    let missing: Vec<usize> = (0..texts.len()).filter(|&i| texts[i].is_none()).collect();
    if !missing.is_empty() {
        let day = session::day(now_ms());
        let cost = missing.len() as f64 * sam::USD_PER_IMAGE;
        if spent_today(&db, day).await? + cost > budget(env) {
            return error(
                429,
                "BUDGET",
                "The demo has used its model budget for today. Try again tomorrow.",
            );
        }
        let Some(obj) = bucket.get(&photo.key).execute().await? else {
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
        let calls = missing.iter().map(|&i| {
            let body = sam::request(see::CONCEPTS[i].1, &data_url);
            async move { meta_post(env, sam::URL, &body).await }
        });
        let replies = futures::future::join_all(calls).await;
        for (&i, reply) in missing.iter().zip(replies) {
            let reply = match reply {
                Ok(r) => r,
                Err(m) => return error(502, "META_ERROR", &m),
            };
            let n = sam::images_processed(&reply);
            log_spend(
                &db,
                sam::MODEL,
                &s.id,
                see::CONCEPTS[i].0,
                n as f64 * sam::USD_PER_IMAGE,
            )
            .await?;
            let text = sam::output_text(&reply);
            env.bucket(PHOTOS)?
                .put(
                    sam_cache_key(&photo, see::CONCEPTS[i].0),
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
                return error(
                    502,
                    "SAM_PARSE",
                    &format!(
                        "Could not read the SAM result for {}: {m}",
                        see::CONCEPTS[i].0
                    ),
                )
            }
        }
    }
    // No shaft under that name: try other names, one paid call at a time, until one hits.
    for (key, prompt) in see::SHAFT_FALLBACKS {
        if !parsed[1].is_empty() {
            break;
        }
        let text = match r2_text(&bucket, &sam_cache_key(&photo, key)).await? {
            Some(t) => t,
            None => match sam_call(env, &db, &bucket, &s.id, &photo, key, prompt).await? {
                Ok(t) => t,
                Err(r) => return Ok(r),
            },
        };
        parsed[1] = sam::parse(&text).unwrap_or_default();
    }
    let coin_mm = coins::find(&s.coin_id)
        .map(|c| c.diameter_mm)
        .unwrap_or(23.25);
    let ruler_obj = see::best_object(&parsed[3]);
    let ruler_fit = match ruler_obj {
        Some(o) if ruler_auto => {
            let bytes = match bucket.get(&photo.key).execute().await? {
                Some(obj) => match obj.body() {
                    Some(b) => b.bytes().await?,
                    None => Vec::new(),
                },
                None => Vec::new(),
            };
            crate::ruler::decode_jpeg_gray(&bytes)
                .and_then(|img| crate::ruler::read_ruler(&img, o, see::shaft_centre(&parsed[1])))
        }
        _ => None,
    };
    let source = match &s.ruler {
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
    let result = see::assemble(source, &parsed[1], &parsed[2]);
    s.see = Some(result);
    s.stage = "see".into();
    save(&db, &s).await?;
    json_response(200, &s.state_json())
}

/// The raw SAM output for one concept, for transparency and test fixtures.
async fn sam_raw(env: &Env, s: &Session, key: &str) -> Result<Response> {
    let (Some(photo), true) = (
        s.photos.first(),
        see::CONCEPTS.iter().any(|(k, _)| *k == key),
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
    let headers = Headers::new();
    let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| e.to_string());
    set("content-type", "application/json")?;
    set("accept", "application/json, text/event-stream")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(JsValue::from_str(
            &mcp::rpc(1, tool, arguments).to_string(),
        )));
    let req = Request::new_with_init(&url, &init).map_err(|e| e.to_string())?;
    let mut resp = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| format!("Any Part Made not reachable: {e}"))?;
    let status = resp.status_code();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let Some(v) = mcp::parse_body(&text) else {
        return Err(format!("Any Part Made answered {status} with no JSON."));
    };
    mcp::tool_result(&v)
}

/// One check round: build the knob from the spec, store the 3MF, ask Any Part Made.
/// The shaft numbers come from the measuring step; a spec may not change them.
async fn run_check(env: &Env, mut s: Session, spec: KnobSpec, changes: Value) -> Result<Response> {
    if s.rounds.len() >= session::MAX_ROUNDS {
        return error(
            429,
            "TOO_MANY_ROUNDS",
            "This repair has used its check rounds. Start a new one.",
        );
    }
    let Some((d, f)) = s.shaft() else {
        return error(
            400,
            "SHAFT_REQUIRED",
            "Measure the shaft first, or type it from calipers.",
        );
    };
    let mut spec = spec;
    spec.bore.diameter_mm = d;
    spec.bore.flat_to_round_mm = f;
    if let Err(m) = spec.validate() {
        return error(400, "BAD_SPEC", &m);
    }
    let file = threemf::three_mf(&spec.mesh(), &s.line);
    let n = s.rounds.len() + 1;
    env.bucket(PHOTOS)?
        .put(format!("models/{}/{n}.3mf", s.id), file.clone())
        .execute()
        .await?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&file);
    let check = match mcp_call(env, "check_part", mcp::check_args(&spec, &s.line, &b64)).await {
        Ok(c) => c,
        Err(m) => return error(502, "CHECK_ERROR", &m),
    };
    console_log!(
        "session {} round {n}: {}",
        s.id,
        check["verdict"].as_str().unwrap_or("?")
    );
    s.rounds.push(Round {
        n,
        at_ms: now_ms(),
        spec: spec.clone(),
        changes,
        check: mcp::summary(&check),
        file_bytes: file.len(),
    });
    s.spec = Some(spec);
    s.stage = "check".into();
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
    let body = muse::fix_request(&s.line, &last.spec, &last.check, "");
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

/// POST /quote {"country": "DE"}: prices for the last round, which must be ok.
async fn quote_route(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let body: Value = req.json().await.unwrap_or(Value::Null);
    let country = body["country"].as_str().unwrap_or("DE").to_uppercase();
    if !session::COUNTRIES.iter().any(|(c, _)| *c == country) {
        return error(400, "BAD_COUNTRY", "Pick the country to ship to.");
    }
    let Some(check_id) = s
        .rounds
        .last()
        .filter(|r| r.check["verdict"] == "ok")
        .and_then(|r| r.check["check_id"].as_str())
        .map(str::to_owned)
    else {
        return error(
            409,
            "NOT_OK",
            "The part has to pass the check before it can be quoted.",
        );
    };
    let q = match mcp_call(
        env,
        "quote_part",
        json!({"check_ids": [check_id], "ship_to_country": country}),
    )
    .await
    {
        Ok(q) => q,
        Err(m) => return error(502, "QUOTE_ERROR", &m),
    };
    s.quote = Some(mcp::quote_view(&q));
    s.order = None;
    s.stage = "quote".into();
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
}

/// POST /order {"option_id": "opt_standard"}: the approval link on anypartmade.com.
/// Nothing is ordered or charged here; the user approves there.
async fn order_route(mut req: Request, env: &Env, mut s: Session) -> Result<Response> {
    let body: Value = req.json().await.unwrap_or(Value::Null);
    let option = body["option_id"].as_str().unwrap_or("");
    if !mcp::OPTIONS.contains(&option) {
        return error(400, "BAD_OPTION", "Pick one of the options.");
    }
    let Some(quote_id) = s
        .quote
        .as_ref()
        .and_then(|q| q["quote_id"].as_str())
        .map(str::to_owned)
    else {
        return error(409, "QUOTE_FIRST", "Get the price first.");
    };
    let o = match mcp_call(
        env,
        "create_order_link",
        json!({"quote_id": quote_id, "option_id": option, "via": "link"}),
    )
    .await
    {
        Ok(o) => o,
        Err(m) => return error(502, "ORDER_ERROR", &m),
    };
    s.order = Some(mcp::order_view(&o));
    save(&env.d1("DB")?, &s).await?;
    json_response(200, &s.state_json())
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

async fn get_model(env: &Env, s: &Session, n: &str) -> Result<Response> {
    let Some(n) = n
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1 && *n <= s.rounds.len())
    else {
        return error(404, "NOT_FOUND", "No such model.");
    };
    let Some(obj) = env
        .bucket(PHOTOS)?
        .get(format!("models/{}/{n}.3mf", s.id))
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
    h.set("content-type", "model/3mf")?;
    h.set(
        "content-disposition",
        &format!("attachment; filename=\"knob-round-{n}.3mf\""),
    )?;
    h.set("x-content-type-options", "nosniff")?;
    Ok(r)
}

fn page_response() -> Result<Response> {
    let mut r = Response::from_html(page::html())?;
    let h = r.headers_mut();
    h.set("content-security-policy", &page::csp())?;
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
        (Method::Get, [""]) => page_response(),
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
                (Method::Post, ["scale"]) => set_ruler(req, &env, s).await,
                (Method::Post, ["check"]) => check_route(req, &env, s).await,
                (Method::Post, ["design"]) => design_route(&env, s).await,
                (Method::Post, ["fix"]) => fix_route(&env, s).await,
                (Method::Post, ["quote"]) => quote_route(req, &env, s).await,
                (Method::Post, ["order"]) => order_route(req, &env, s).await,
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
