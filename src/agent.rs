//! Agent mode: Muse Spark talks with the user, measures with SAM 3.1 and Rust, designs
//! a KnobSpec and submits it to Any Part Made, through tools. Chat Completions tool
//! calling (https://dev.meta.ai/docs/tool-calling). This module holds the instructions,
//! the tools, and the rules that keep numbers honest; the Worker runs the tools.

use crate::coins::COINS;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Model calls per user turn before the agent stops and reports.
pub const MAX_STEPS: usize = 14;
pub const MAX_PHOTOS: usize = 6;

pub const SYSTEM: &str = "You are the repair assistant of \"Fix it with a photo\". The user lost a part, for now a knob that pushes onto a D shaped shaft (oven, cooker, washing machine, radio). \
Your job: help the user measure what the replacement must fit as well as possible from phone photos, design the knob, and get it checked, priced and guaranteed by Any Part Made. \
Rules. \
1. Numbers come from measurements, never from your eye. Use measure_photo: SAM 3.1 segments the scale reference and the shaft, Rust turns masks and ruler ticks into millimetres. Then record each number you rely on with set_dimension, citing the measurement (for example m2.shaft.diameter_mm) or the user's own words. \
2. Ask only for what you need to rebuild the part well, one clear request at a time, and say why. Typical needs: a photo straight at the shaft end with a ruler or coin right next to it (diameter and the D flat), a photo from above or the side with the ruler (how far the shaft sticks out, which sets the bore depth), a photo of a sibling knob (size, top, grip, pointer). If a photo is dark, blurred or the ruler is far from the shaft, say so and ask for a better one. Do not ask about anything the part does not need. \
3. Finding the shaft: SAM's word search often misses a small shaft, and your own pixel guesses are rough. So when the shaft is not found, or the outline is clearly wrong (look at shaft_box_mm: a knob shaft is about 4 to 12 mm wide; a box of 14 x 60 mm is the recess or the panel, not the shaft), do not blame the photo: ask the user to tap the middle of the shaft end in that photo. Their tap arrives as a message with the photo number and position; then call measure_photo with that point as shaft_point. Only ask for a new photo when the photo itself is the problem: the ruler or coin not found, the shaft out of focus, or the wrong view for what you need. 4. Read the measurement notes and confidence. A side view gives the diameter and the visible length but not the flat. An end view gives diameter and flat. Prefer the measurement with the higher confidence and say which one you used. \
5. When shaft diameter and flat are set, call build_and_check with a full spec. bore.depth_mm must stay at least 1 mm under the shaft length when that is known, so the knob does not rub on the panel. FDM holes come out small: clearance_mm 0.1 to 0.3. Match the sibling knob if you saw one, else a plain 30 to 40 mm knob. \
6. Any Part Made answers with a verdict and issues with exact fixes and locations (frame: millimetres, z up, the knob stands on z = 0 with the bore opening down along z, the D flat and the pointer face +x). Fix every error with the smallest change and call build_and_check again, at most 4 fixes. If the verdict is refused, tell the user the reason as given and stop: never resubmit with other wording. \
7. When the verdict is ok, tell the user exactly the dimensions listed in the guarantee and nothing else as guaranteed (outer size, height and depth are design choices, say them as such: diameter is across, height is top to bottom), then ask where to ship and call quote, and order_link for the option they pick. Never say the part is ordered: the user approves on anypartmade.com. \
Write short, plain sentences. No dashes as punctuation. Use the user's language.";

pub fn tools() -> Value {
    let scales: Vec<Value> = std::iter::once(json!("ruler"))
        .chain(COINS.iter().map(|c| json!(c.id)))
        .collect();
    json!([
        {"type": "function", "function": {
            "name": "measure_photo",
            "description": "Measure one photo with SAM 3.1 and Rust geometry: the scale (ruler ticks or a coin), the shaft (end view: diameter and D flat; side view: diameter and visible length) and a sibling knob if one is in view. Returns an id like m3 and the numbers with confidence and notes.",
            "parameters": {"type": "object", "properties": {
                "photo": {"type": "integer", "minimum": 1, "description": "Photo number as listed in the conversation"},
                "scale": {"type": "string", "enum": scales, "description": "What gives the scale in this photo"},
                "shaft_point": {"type": "object", "description": "A pixel on the shaft in this photo, when the word search may miss it", "properties": {
                    "x": {"type": "integer", "minimum": 0}, "y": {"type": "integer", "minimum": 0}
                }, "required": ["x", "y"]}
            }, "required": ["photo", "scale"]}
        }},
        {"type": "function", "function": {
            "name": "set_dimension",
            "description": "Record one number the part must fit. Give either from (a measured value, like m2.shaft.diameter_mm) or value plus user_quote (the user's own words that state it).",
            "parameters": {"type": "object", "properties": {
                "name": {"type": "string", "enum": ["shaft_diameter_mm", "flat_to_round_mm", "shaft_length_mm"]},
                "from": {"type": "string"},
                "value": {"type": "number"},
                "user_quote": {"type": "string"},
                "why": {"type": "string", "description": "One sentence: why this value"}
            }, "required": ["name", "why"]}
        }},
        {"type": "function", "function": {
            "name": "build_and_check",
            "description": "Build the knob from a KnobSpec (Rust: watertight mesh, 3MF) and check it with Any Part Made. The bore numbers are taken from the recorded dimensions. Returns the round number, verdict, issues with fixes, and on ok the guarantee.",
            "parameters": {"type": "object", "properties": {
                "spec": crate::muse::spec_schema(),
                "changes": {"type": "array", "items": {"type": "object", "properties": {
                    "field": {"type": "string"}, "from": {"type": "string"}, "to": {"type": "string"}, "why": {"type": "string"}
                }, "required": ["field", "from", "to", "why"]}},
                "summary": {"type": "string", "description": "One or two sentences for the user about this design or change"}
            }, "required": ["spec", "changes", "summary"]}
        }},
        {"type": "function", "function": {
            "name": "quote",
            "description": "Prices and delivery dates from Any Part Made for the last design that passed (verdict ok).",
            "parameters": {"type": "object", "properties": {
                "country": {"type": "string", "enum": crate::session::COUNTRIES.iter().map(|c| c.0).collect::<Vec<_>>()}
            }, "required": ["country"]}
        }},
        {"type": "function", "function": {
            "name": "order_link",
            "description": "The approval link on anypartmade.com for the quote option the user picked. Nothing is ordered until they approve there.",
            "parameters": {"type": "object", "properties": {
                "option_id": {"type": "string", "enum": crate::mcp::OPTIONS}
            }, "required": ["option_id"]}
        }}
    ])
}

/// A number the part must fit, and where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub value: f64,
    pub source: String,
    pub why: String,
}

/// Looks up a measured value like "m2.shaft.diameter_mm" in the stored measurements.
pub fn lookup(measurements: &[Value], path: &str) -> Option<f64> {
    let mut parts = path.split('.');
    let id = parts.next()?;
    let m = measurements.iter().find(|m| m["id"] == id)?;
    let mut v = &m["result"];
    for p in parts {
        v = v.get(p)?;
    }
    v.as_f64()
}

/// Checks a set_dimension call. A value must come from a measurement, or be stated in
/// the user's own words (the quote must appear in what the user wrote).
pub fn dimension(
    args: &Value,
    measurements: &[Value],
    user_text: &str,
) -> Result<(String, Dimension), String> {
    let name = args["name"].as_str().ok_or("name is missing")?;
    let (lo, hi) = match name {
        "shaft_diameter_mm" => (2.0, 12.0),
        "flat_to_round_mm" => (1.0, 12.0),
        "shaft_length_mm" => (2.0, 60.0),
        _ => return Err(format!("unknown dimension {name}")),
    };
    let why = args["why"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    let (value, source) = if let Some(from) = args["from"].as_str().filter(|s| !s.is_empty()) {
        let v = lookup(measurements, from).ok_or_else(|| {
            format!(
                "{from} is not a measured value. Use an id and path from a measure_photo result."
            )
        })?;
        (v, from.to_string())
    } else {
        let v = args["value"]
            .as_f64()
            .ok_or("give from, or value with user_quote")?;
        let quote = args["user_quote"].as_str().unwrap_or("").trim();
        let norm = |s: &str| {
            s.to_lowercase()
                .replace(',', ".")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        if quote.len() < 2 || !norm(user_text).contains(&norm(quote)) {
            return Err("user_quote must be the user's own words that state this number.".into());
        }
        (v, format!("user: \"{quote}\""))
    };
    if !(value >= lo && value <= hi) {
        return Err(format!("{name} must be from {lo} to {hi} mm, not {value}."));
    }
    Ok((name.to_string(), Dimension { value, source, why }))
}

/// The numbers of a measure_photo result that the agent needs, without outlines.
pub fn measurement_view(id: &str, photo: usize, see: &Value) -> Value {
    let pick = |v: &Value, keys: &[&str]| -> Value {
        let mut o = serde_json::Map::new();
        for k in keys {
            if !v[*k].is_null() {
                o.insert((*k).into(), v[*k].clone());
            }
        }
        Value::Object(o)
    };
    let shaft_box = see["overlays"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["label"] == "shaft"))
        .map(|o| o["box"].clone())
        .unwrap_or(Value::Null);
    json!({
        "id": id,
        "photo": photo,
        "result": {
            "scale": pick(&see["scale"], &["source", "px_per_mm", "mm_per_px", "confidence", "scale_change"]),
            "shaft": pick(&see["shaft"], &["view", "diameter_mm", "flat_to_round_mm", "has_flat", "length_mm", "plus_minus_mm", "confidence"]),
            "shaft_box_px": shaft_box,
            "shaft_box_mm": box_mm(&shaft_box, see["scale"]["mm_per_px"].as_f64()),
            "knob": pick(&see["knob"], &["outer_diameter_mm", "rib_count", "rib_depth_mm", "confidence"]),
            "notes": see["notes"],
        }
    })
}

fn box_mm(b: &Value, mm_per_px: Option<f64>) -> Value {
    match (b.as_array(), mm_per_px) {
        (Some(a), Some(k)) if a.len() == 4 => {
            let f = |i: usize| a[i].as_f64().unwrap_or(0.0);
            let r = |v: f64| (v * 10.0).round() / 10.0;
            json!([r((f(2) - f(0)) * k), r((f(3) - f(1)) * k)])
        }
        _ => Value::Null,
    }
}

/// A user message: text, the photos it adds (by number and size), and an optional tap.
/// Photos are references here; the Worker swaps in the image data for each request.
pub fn user_message(
    text: &str,
    photos: &[(usize, u32, u32)],
    tap: Option<(usize, u32, u32)>,
) -> Value {
    let mut words = text.trim().to_string();
    for (n, w, h) in photos {
        words.push_str(&format!("\n[Photo {n}: {w} x {h} px]"));
    }
    if let Some((n, x, y)) = tap {
        words.push_str(&format!("\n[The user tapped photo {n} at x={x}, y={y}]"));
    }
    let mut content = vec![json!({"type": "text", "text": words})];
    for (n, _, _) in photos {
        content.push(json!({"type": "photo_ref", "photo": n}));
    }
    json!({"role": "user", "content": content})
}

/// Swaps photo references for image parts with data URLs, for the model request.
pub fn materialize(messages: &[Value], data_url: &dyn Fn(usize) -> Option<String>) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let Some(parts) = m["content"].as_array() else {
                return m.clone();
            };
            let parts: Vec<Value> = parts
                .iter()
                .filter_map(|p| {
                    if p["type"] == "photo_ref" {
                        let n = p["photo"].as_u64()? as usize;
                        Some(json!({"type": "image_url", "image_url": {"url": data_url(n)?, "detail": "high"}}))
                    } else {
                        Some(p.clone())
                    }
                })
                .collect();
            let mut m = m.clone();
            m["content"] = Value::Array(parts);
            m
        })
        .collect()
}

/// Everything the user typed, for checking user_quote.
pub fn user_words(messages: &[Value]) -> String {
    messages
        .iter()
        .filter(|m| m["role"] == "user")
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter_map(|p| p["text"].as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn request(messages: Vec<Value>) -> Value {
    let mut all = vec![json!({"role": "system", "content": SYSTEM})];
    all.extend(messages);
    json!({
        "model": crate::muse::MODEL,
        "messages": all,
        "tools": tools(),
        "parallel_tool_calls": false,
        "max_completion_tokens": 12000,
    })
}

/// What the page shows of the conversation: no image data, tool calls with short results.
pub fn transcript(messages: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in messages {
        match m["role"].as_str() {
            Some("user") => {
                let parts = m["content"].as_array().cloned().unwrap_or_default();
                let text = parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let text: String = text
                    .lines()
                    .filter(|l| !l.starts_with("[Photo "))
                    .collect::<Vec<_>>()
                    .join("\n");
                let photos: Vec<Value> = parts
                    .iter()
                    .filter(|p| p["type"] == "photo_ref")
                    .map(|p| p["photo"].clone())
                    .collect();
                out.push(json!({"role": "user", "text": text.trim(), "photos": photos}));
            }
            Some("assistant") => {
                if let Some(t) = m["content"].as_str().filter(|t| !t.trim().is_empty()) {
                    out.push(json!({"role": "assistant", "text": t}));
                }
                for c in m["tool_calls"].as_array().into_iter().flatten() {
                    let args: Value =
                        serde_json::from_str(c["function"]["arguments"].as_str().unwrap_or("{}"))
                            .unwrap_or(Value::Null);
                    out.push(json!({"role": "tool_call", "id": c["id"], "name": c["function"]["name"], "args": args}));
                }
            }
            Some("tool") => {
                let v: Value = serde_json::from_str(m["content"].as_str().unwrap_or("{}"))
                    .unwrap_or(Value::Null);
                out.push(json!({"role": "tool_result", "id": m["tool_call_id"], "result": v}));
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured() -> Vec<Value> {
        vec![
            json!({"id": "m1", "photo": 2, "result": {"shaft": {"diameter_mm": 5.96, "flat_to_round_mm": 4.37}}}),
        ]
    }

    #[test]
    fn dimensions_come_from_measurements_or_the_users_words() {
        let m = measured();
        let (n, d) = dimension(&json!({"name": "shaft_diameter_mm", "from": "m1.shaft.diameter_mm", "why": "end view"}), &m, "").unwrap();
        assert_eq!((n.as_str(), d.value), ("shaft_diameter_mm", 5.96));
        assert!(dimension(
            &json!({"name": "shaft_diameter_mm", "from": "m9.shaft.diameter_mm", "why": ""}),
            &m,
            ""
        )
        .is_err());
        // A number the user never said is refused.
        assert!(dimension(&json!({"name": "shaft_length_mm", "value": 11.0, "user_quote": "about 11 mm", "why": ""}), &m, "the knob is gone").is_err());
        let (_, d) = dimension(
            &json!({"name": "shaft_length_mm", "value": 11.0, "user_quote": "sticks out 11 mm", "why": ""}),
            &m,
            "The shaft sticks out 11 mm from the panel",
        )
        .unwrap();
        assert_eq!(d.value, 11.0);
        assert!(d.source.starts_with("user:"));
        assert!(dimension(
            &json!({"name": "shaft_diameter_mm", "value": 40.0, "user_quote": "40", "why": ""}),
            &m,
            "40"
        )
        .is_err());
    }

    #[test]
    fn photos_are_references_until_the_request() {
        let u = user_message("Here is the shaft", &[(1, 1152, 2048)], Some((1, 700, 960)));
        let text = u["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("[Photo 1: 1152 x 2048 px]") && text.contains("x=700, y=960"));
        let m = materialize(std::slice::from_ref(&u), &|n| {
            Some(format!("data:image/jpeg;base64,{n}"))
        });
        assert_eq!(m[0]["content"][1]["type"], "image_url");
        assert_eq!(
            m[0]["content"][1]["image_url"]["url"],
            "data:image/jpeg;base64,1"
        );
        let t = transcript(&[u]);
        assert_eq!(t[0]["photos"], json!([1]));
        assert!(!t[0]["text"].as_str().unwrap().contains("[Photo"));
    }

    #[test]
    fn request_has_tools_and_no_dashes_in_the_instructions() {
        let r = request(vec![]);
        let names: Vec<&str> = r["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "measure_photo",
                "set_dimension",
                "build_and_check",
                "quote",
                "order_link"
            ]
        );
        assert!(!SYSTEM.contains('\u{2014}') && !SYSTEM.contains('\u{2013}'));
        assert_eq!(r["parallel_tool_calls"], false);
    }

    #[test]
    fn measurement_view_keeps_numbers_and_the_shaft_box() {
        let see = json!({
            "scale": {"source": "ruler_ticks", "px_per_mm": 10.09, "confidence": 0.79},
            "shaft": {"view": "end", "diameter_mm": 5.96, "flat_to_round_mm": 4.37, "confidence": 0.58, "diameter_px": 60.1},
            "knob": null,
            "overlays": [{"label": "shaft", "box": [671, 937, 732, 988], "outline": [[1, 2]]}],
            "notes": ["n"]
        });
        let v = measurement_view("m1", 2, &see);
        assert_eq!(v["result"]["shaft"]["diameter_mm"], 5.96);
        assert!(v["result"]["shaft"].get("diameter_px").is_none());
        assert_eq!(v["result"]["shaft_box_px"], json!([671, 937, 732, 988]));
        let see2 = json!({"scale": {"mm_per_px": 0.1}, "overlays": [{"label": "shaft", "box": [0, 0, 60, 50]}]});
        assert_eq!(
            measurement_view("m2", 1, &see2)["result"]["shaft_box_mm"],
            json!([6.0, 5.0])
        );
        assert_eq!(lookup(&[v], "m1.shaft.flat_to_round_mm"), Some(4.37));
    }
}
