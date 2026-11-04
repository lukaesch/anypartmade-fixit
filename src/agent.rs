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

pub const SYSTEM: &str = "You are the designer of \"Fix it with a photo\". People show you a part they need: a lost knob, an enclosure for a circuit board, a cap, a bracket, a spacer. \
You design it in a CAD language, and Any Part Made checks, prices and guarantees it. You are responsible for the design; Rust builds exactly what you write and fills in the measured fit sizes. \
Rules. \
1. Numbers that must fit real things come from measurements, never from your eye or memory. measure_photo measures a D shaft (end view: diameter and flat; side view: diameter and visible length). measure_object measures anything else (a board, a hole, a tube) by a short noun phrase or a point: length, width and round diameter against the ruler or coin. Record every number you rely on with set_dimension, citing the measurement (like m2.length_mm) or the user's own words. \
2. Ask only for what you need to build the part well, one clear request at a time, and say why. For a knob: a photo straight at the shaft end with a ruler next to it, a view from the side or above for the shaft length, a sibling knob for the look. For an enclosure: the board from above with a ruler, the board from the side for its height and where the connectors sit, and how the user wants to use it. If a photo is dark, blurred or the ruler is far away, say so and ask for a better one. \
3. SAM's word search can miss small things and your pixel guesses are rough. When something is not found, or the outline is clearly wrong (check the sizes in mm against what the object should be), ask the user to mark it in that photo. For a simple solid thing (a shaft) one tap in its middle is enough. For a thing made of parts (a board with chips, pins and a label) a tap picks just one part, so ask the user to tap two opposite corners: that marks a box, which outlines the whole object; pass it as box to measure_object. Taps and boxes arrive as messages with the photo number and positions. Ask for a new photo only when the photo itself is the problem, and say what is wrong with it. \
4. Design with build_and_check. Use fit features for everything that must fit: d_hole for a measured shaft, pocket for a measured board, usb_c_cutout where a USB C plug goes in. Their sizes come from the recorded dimensions, and Any Part Made checks them. Choose the rest (outer shape, walls, grip, lid, fillets) so it works and looks right; say these are design choices. Walls at least 1.6 mm; the part prints standing on z = 0. Material asa for heat (cooking appliances, cars), else petg. \
5. Any Part Made answers with a verdict and issues with exact fixes and locations in the part's coordinates. Fix every error with the smallest change and check again, at most 4 fixes. If the verdict is refused, tell the user the reason as given and stop: never resubmit with other wording. \
6. When the verdict is ok, tell the user exactly which sizes and interfaces are guaranteed (only those Any Part Made lists), then ask where to ship and call quote, and order_link for the option they pick. Never say the part is ordered: the user approves on anypartmade.com. When they say they approved or declined, call order_status and report only what it says. \
When you ask the user something with a few likely answers, end your message with one line: Choices: first | second | third (two to four short answers, the page shows them as buttons). \
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
            "name": "measure_object",
            "description": "Measure any object in one photo with SAM 3.1 and Rust: give object (a short noun phrase like circuit board) or point (a pixel on it, for example from the user's tap). Returns length_mm, width_mm (rectangle sides), round_diameter_mm, angle, how rectangular it is, the scale used and an id like m4.",
            "parameters": {"type": "object", "properties": {
                "photo": {"type": "integer", "minimum": 1},
                "scale": {"type": "string", "enum": scales.clone()},
                "object": {"type": "string"},
                "point": {"type": "object", "properties": {"x": {"type": "integer", "minimum": 0}, "y": {"type": "integer", "minimum": 0}}, "required": ["x", "y"]},
                "box": {"type": "object", "description": "Two opposite corners around the whole object, for example from the user's box", "properties": {
                    "x1": {"type": "integer", "minimum": 0}, "y1": {"type": "integer", "minimum": 0}, "x2": {"type": "integer", "minimum": 0}, "y2": {"type": "integer", "minimum": 0}
                }, "required": ["x1", "y1", "x2", "y2"]}
            }, "required": ["photo", "scale"]}
        }},
        {"type": "function", "function": {
            "name": "set_dimension",
            "description": "Record one number the part must fit. Give either from (a measured value, like m2.shaft.diameter_mm) or value plus user_quote (the user's own words that state it).",
            "parameters": {"type": "object", "properties": {
                "name": {"type": "string", "enum": DIMENSIONS.iter().map(|d| d.0).collect::<Vec<_>>()},
                "from": {"type": "string"},
                "value": {"type": "number"},
                "user_quote": {"type": "string"},
                "why": {"type": "string", "description": "One sentence: why this value"}
            }, "required": ["name", "why"]}
        }},
        {"type": "function", "function": {
            "name": "build_and_check",
            "description": crate::cad::GUIDE,
            "parameters": {"type": "object", "properties": {
                "part": crate::cad::schema(),
                "material": {"type": "string", "enum": ["petg", "asa"]},
                "use_description": {"type": "string", "description": "What the part is for, for the safety screen, like: Replacement control knob for an electric oven"},
                "changes": {"type": "array", "items": {"type": "object", "properties": {
                    "field": {"type": "string"}, "from": {"type": "string"}, "to": {"type": "string"}, "why": {"type": "string"}
                }, "required": ["field", "from", "to", "why"]}},
                "summary": {"type": "string", "description": "One or two sentences for the user about this design or change"}
            }, "required": ["part", "material", "use_description", "changes", "summary"]}
        }},
        {"type": "function", "function": {
            "name": "quote",
            "description": "Prices and delivery dates from Any Part Made for the last design that passed (verdict ok).",
            "parameters": {"type": "object", "properties": {
                "country": {"type": "string", "enum": crate::session::COUNTRIES.iter().map(|c| c.0).collect::<Vec<_>>()}
            }, "required": ["country"]}
        }},
        {"type": "function", "function": {
            "name": "order_status",
            "description": "The real state of the order on anypartmade.com: pending approval, approved, declined or expired. Call it when the user says they approved or declined, and say what happens next in one or two sentences.",
            "parameters": {"type": "object", "properties": {}}
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

/// The numbers a part can be fitted to, with their bounds in mm.
pub const DIMENSIONS: [(&str, f64, f64); 6] = [
    ("shaft_diameter_mm", 2.0, 12.0),
    ("flat_to_round_mm", 1.0, 12.0),
    ("shaft_length_mm", 2.0, 60.0),
    ("board_length_mm", 5.0, 180.0),
    ("board_width_mm", 5.0, 180.0),
    ("board_height_mm", 0.5, 60.0),
];

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
    let Some(&(_, lo, hi)) = DIMENSIONS.iter().find(|d| d.0 == name) else {
        return Err(format!("unknown dimension {name}"));
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

/// A tap on a photo: (photo, x, y), and the opposite corner when it marks a box.
pub type Mark = (usize, u32, u32, Option<(u32, u32)>);

/// A user message: text, the photos it adds (by number and size), and an optional tap.
/// Photos are references here; the Worker swaps in the image data for each request.
pub fn user_message(text: &str, photos: &[(usize, u32, u32)], tap: Option<Mark>) -> Value {
    let mut words = text.trim().to_string();
    for (n, w, h) in photos {
        words.push_str(&format!("\n[Photo {n}: {w} x {h} px]"));
    }
    match tap {
        Some((n, x, y, None)) => {
            words.push_str(&format!("\n[The user tapped photo {n} at x={x}, y={y}]"))
        }
        Some((n, x, y, Some((x2, y2)))) => words.push_str(&format!(
            "\n[The user marked a box on photo {n} from x={x}, y={y} to x={x2}, y={y2}]"
        )),
        None => {}
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
        let u = user_message(
            "Here is the shaft",
            &[(1, 1152, 2048)],
            Some((1, 700, 960, None)),
        );
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
                "measure_object",
                "set_dimension",
                "build_and_check",
                "quote",
                "order_status",
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
