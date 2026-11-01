//! HTTP routes. Only glue lives here; checks and data shapes are in the plain modules.

use crate::sam;
use crate::see::{self, ManualShaft, RulerScale, ScaleSource};
use crate::session::{self, PhotoRef, Session};
use crate::{coins, page, photo};
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

/// SAM 3.1 on the first photo for coin, shaft and knob (cached per photo hash),
/// then the measurements in Rust.
async fn run_see(env: &Env, mut s: Session) -> Result<Response> {
    let Some(photo) = s.photos.first().cloned() else {
        return error(400, "PHOTO_REQUIRED", "This session has no photo.");
    };
    let ruler = s.coin_id == session::RULER;
    if ruler && s.ruler.is_none() {
        return error(400, "SCALE_REQUIRED", "Tap two marks on the ruler first.");
    }
    let db = env.d1("DB")?;
    let bucket = env.bucket(PHOTOS)?;
    // With a ruler there is no coin to find, so SAM runs only for shaft and knob.
    let wanted: Vec<bool> = see::CONCEPTS
        .iter()
        .map(|(k, _)| !(ruler && *k == "coin"))
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
    let coin_mm = coins::find(&s.coin_id)
        .map(|c| c.diameter_mm)
        .unwrap_or(23.25);
    let source = match &s.ruler {
        Some(r) if ruler => ScaleSource::Ruler(r),
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
