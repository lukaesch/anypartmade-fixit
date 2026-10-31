//! A session is one repair: the photos, the user's line, the coin, and the results of
//! each step. Stored as one JSON document per session.

use crate::coins::{self, Coin};
use crate::photo::PhotoInfo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const SESSIONS_PER_IP_PER_DAY: i64 = 20;
pub const KEEP_DAYS: i64 = 7;
const DAY_MS: i64 = 86_400_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhotoRef {
    /// R2 key.
    pub key: String,
    pub content_type: String,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
    pub sha256: String,
}

impl PhotoRef {
    pub fn new(info: &PhotoInfo) -> Self {
        PhotoRef {
            key: format!("photos/{}.{}", info.sha256, info.kind.ext()),
            content_type: info.kind.content_type().into(),
            width: info.width,
            height: info.height,
            bytes: info.bytes,
            sha256: info.sha256.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at_ms: i64,
    pub line: String,
    pub coin_id: String,
    /// First photo: shaft, sibling knob and coin. Optional second: the knob from the side.
    pub photos: Vec<PhotoRef>,
    /// The last finished step: "photo" for now, later "see", "design", "check", "quote".
    pub stage: String,
}

/// The user's line, for example "control knob, electric oven".
pub fn clean_line(raw: &str) -> Result<String, String> {
    if raw.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err("The description has characters we cannot use.".into());
    }
    let s = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = s.chars().count();
    if n < 3 {
        return Err(
            "Say in a few words what the part is, for example: control knob, electric oven.".into(),
        );
    }
    if n > 120 {
        return Err("Keep the description to 120 characters.".into());
    }
    Ok(s)
}

pub fn coin(raw: &str) -> Result<Coin, String> {
    coins::find(raw.trim()).ok_or_else(|| "Pick the coin that is in the photo.".into())
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Day number since the epoch, for the per IP rate limit.
pub fn day(now_ms: i64) -> i64 {
    now_ms.div_euclid(DAY_MS)
}

pub fn expired_before(now_ms: i64) -> i64 {
    now_ms - KEEP_DAYS * DAY_MS
}

impl Session {
    /// What the page needs. No IP, no storage keys.
    pub fn state_json(&self) -> Value {
        let coin = coins::find(&self.coin_id);
        json!({
            "id": self.id,
            "stage": self.stage,
            "line": self.line,
            "coin": coin.map(|c| json!({"id": c.id, "label": c.label, "diameter_mm": c.diameter_mm})),
            "photos": self.photos.iter().enumerate().map(|(i, p)| json!({
                "url": format!("/api/session/{}/photo/{}", self.id, i + 1),
                "width": p.width,
                "height": p.height,
            })).collect::<Vec<_>>(),
            "created_at_ms": self.created_at_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photo::{inspect, tests::fake_jpeg};

    #[test]
    fn line_is_cleaned_and_bounded() {
        assert_eq!(
            clean_line("  control   knob,\telectric oven ").as_deref(),
            Ok("control knob, electric oven")
        );
        assert!(clean_line("ab").is_err());
        assert!(clean_line(&"x".repeat(121)).is_err());
        assert!(clean_line("knob\u{0}x").is_err());
    }

    #[test]
    fn state_hides_storage_keys() {
        let info = inspect(&fake_jpeg(1600, 1200)).expect("jpeg");
        let s = Session {
            id: "0123456789abcdef0123456789abcdef".into(),
            created_at_ms: 1,
            line: "control knob, electric oven".into(),
            coin_id: "eur_1".into(),
            photos: vec![PhotoRef::new(&info)],
            stage: "photo".into(),
        };
        assert!(s.photos[0].key.starts_with("photos/") && s.photos[0].key.ends_with(".jpg"));
        let v = s.state_json();
        assert_eq!(v["coin"]["diameter_mm"], 23.25);
        assert_eq!(
            v["photos"][0]["url"],
            "/api/session/0123456789abcdef0123456789abcdef/photo/1"
        );
        assert!(!v.to_string().contains("photos/"));
    }

    #[test]
    fn ids_and_days() {
        assert!(valid_id("0123456789abcdef0123456789abcdef"));
        assert!(!valid_id("0123456789ABCDEF0123456789abcdef"));
        assert!(!valid_id("../x"));
        assert_eq!(day(DAY_MS * 3 + 5), 3);
        assert_eq!(expired_before(DAY_MS * 10), DAY_MS * 3);
        assert!(coin("eur_050").is_ok() && coin("x").is_err());
    }
}
