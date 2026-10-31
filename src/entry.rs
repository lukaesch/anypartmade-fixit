//! HTTP routes. Only glue lives here; checks and data shapes are in the plain modules.

use crate::session::{self, PhotoRef, Session};
use crate::{page, photo};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
    let coin = match session::coin(&text_field(&form, "coin")) {
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
        coin_id: coin.id.into(),
        photos,
        stage: "photo".into(),
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
        (Method::Get, ["api", "session", id, rest @ ..]) => {
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
            match rest {
                [] => json_response(200, &s.state_json()),
                ["photo", n] => get_photo(&env, &s, n).await,
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
