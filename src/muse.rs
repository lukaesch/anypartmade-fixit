//! Muse Spark: designs the knob as a KnobSpec and repairs it from the checker's issues.
//! Chat Completions with a JSON schema, so the reply is always the shape we parse.
//! Source: https://dev.meta.ai/docs/structured-output and /docs/image-understanding

use crate::knob::KnobSpec;
use serde_json::{json, Value};

pub const URL: &str = "https://api.meta.ai/v1/chat/completions";
pub const MODEL: &str = "muse-spark-1.3";
/// Standard mode, per token (https://dev.meta.ai/docs/pricing-rate-limits).
const USD_IN: f64 = 1.25 / 1e6;
const USD_CACHED: f64 = 0.15 / 1e6;
const USD_OUT: f64 = 4.25 / 1e6;
/// Fix rounds after the first design, as the spec says.
pub const MAX_FIXES: usize = 4;

const FRAME: &str = "You design a replacement knob as a KnobSpec, a small JSON feature language. \
You never write geometry or code: Rust builds the mesh from your JSON and Any Part Made checks it for 3D printing (FDM, ASA). \
Frame: millimetres, z up. The knob stands on z = 0 with the D bore opening downwards along the z axis; the top face is at z = height_mm. \
The D flat of the bore faces +x and the pointer sits at +x on the outside. \
Fields: body.outer_diameter_mm 15 to 80, body.height_mm 8 to 50, body.top flat, dome (top_size_mm is the dome rise) or chamfer (top_size_mm is the chamfer), \
optional body.skirt (a wider flange at the bottom, diameter at least outer + 1, height up to half the knob). \
grip.kind none, ribs (6 to 60 rounded grooves around the side, depth 0.3 to 2), knurl (20 to 90 fine grooves), flats (3 to 12 flat faces). \
pointer.kind none, groove (cut into the side) or ridge (raised on the side), width 0.8 to 4, depth 0.3 to 2. \
bore: shape d, depth_mm at least 3 and at most height minus 2, clearance_mm 0 to 1 added to the measured shaft. \
shell.hollow true leaves a cavity under the top with walls of wall_mm (at least 1.6); false is solid. \
The bore diameter_mm and flat_to_round_mm are measured from the real shaft. Copy them exactly; Rust overwrites them anyway.";

fn changes_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "field": {"type": "string", "description": "Dotted path, for example pointer.kind"},
                "from": {"type": "string"},
                "to": {"type": "string"},
                "why": {"type": "string", "description": "One short sentence, citing the issue code when there is one"}
            },
            "required": ["field", "from", "to", "why"],
            "additionalProperties": false
        }
    })
}

fn num(min: f64, max: f64) -> Value {
    json!({"type": "number", "minimum": min, "maximum": max})
}

pub fn spec_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "body": {
                "type": "object",
                "properties": {
                    "outer_diameter_mm": num(15.0, 80.0),
                    "height_mm": num(8.0, 50.0),
                    "top": {"type": "string", "enum": ["flat", "dome", "chamfer"]},
                    "top_size_mm": num(0.0, 15.0),
                    "skirt": {"anyOf": [
                        {"type": "null"},
                        {"type": "object", "properties": {
                            "diameter_mm": num(16.0, 90.0),
                            "height_mm": num(1.0, 25.0)
                        }, "required": ["diameter_mm", "height_mm"], "additionalProperties": false}
                    ]}
                },
                "required": ["outer_diameter_mm", "height_mm", "top", "top_size_mm", "skirt"],
                "additionalProperties": false
            },
            "grip": {
                "type": "object",
                "properties": {
                    "kind": {"type": "string", "enum": ["none", "ribs", "knurl", "flats"]},
                    "count": {"type": "integer", "minimum": 0, "maximum": 90},
                    "depth_mm": num(0.0, 2.0)
                },
                "required": ["kind", "count", "depth_mm"],
                "additionalProperties": false
            },
            "pointer": {
                "type": "object",
                "properties": {
                    "kind": {"type": "string", "enum": ["none", "groove", "ridge"]},
                    "width_mm": num(0.8, 4.0),
                    "depth_mm": num(0.3, 2.0)
                },
                "required": ["kind", "width_mm", "depth_mm"],
                "additionalProperties": false
            },
            "bore": {
                "type": "object",
                "properties": {
                    "shape": {"type": "string", "enum": ["d"]},
                    "diameter_mm": num(3.0, 12.0),
                    "flat_to_round_mm": num(1.5, 12.0),
                    "depth_mm": num(3.0, 48.0),
                    "clearance_mm": num(0.0, 1.0)
                },
                "required": ["shape", "diameter_mm", "flat_to_round_mm", "depth_mm", "clearance_mm"],
                "additionalProperties": false
            },
            "shell": {
                "type": "object",
                "properties": {
                    "hollow": {"type": "boolean"},
                    "wall_mm": num(1.6, 10.0)
                },
                "required": ["hollow", "wall_mm"],
                "additionalProperties": false
            }
        },
        "required": ["body", "grip", "pointer", "bore", "shell"],
        "additionalProperties": false
    })
}

fn response_format(name: &str) -> Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": name,
            "schema": {
                "type": "object",
                "properties": {
                    "spec": spec_schema(),
                    "changes": changes_schema(),
                    "summary": {"type": "string", "description": "One or two plain sentences for the user"}
                },
                "required": ["spec", "changes", "summary"],
                "additionalProperties": false
            }
        }
    })
}

/// First design: the photo, what SAM and Rust measured, and the user's line.
pub fn design_request(line: &str, facts: &Value, image_data_url: &str, start: &KnobSpec) -> Value {
    let text = format!(
        "The user wants: {line}.\n\
         Measured from the photo and calipers (trust these numbers over your eye): {facts}\n\
         A plain starting spec: {start}\n\
         Look at the photo: the empty shaft, any sibling knob, the panel. Design the knob so it matches the sibling knobs \
         (size, top, grip, pointer) and fits the shaft. If no sibling knob is visible, keep a plain oven knob of 30 to 40 mm. \
         FDM printed holes come out a little small, so choose clearance_mm from 0.1 to 0.3 for a push fit. \
         In changes, list each field you set differently from the starting spec and why.",
        start = serde_json::to_string(start).unwrap_or_default()
    );
    json!({
        "model": MODEL,
        "messages": [
            {"role": "system", "content": FRAME},
            {"role": "user", "content": [
                {"type": "text", "text": text},
                {"type": "image_url", "image_url": {"url": image_data_url, "detail": "high"}}
            ]}
        ],
        "response_format": response_format("knob_design"),
        "max_completion_tokens": 12000,
    })
}

/// A repair round: the current spec and every issue with the checker's fix text.
/// `notes` carries what Rust refused last time, if anything.
pub fn fix_request(line: &str, spec: &KnobSpec, check: &Value, notes: &str) -> Value {
    let issues: Vec<Value> = check["manufacturability"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|i| i["severity"] != "info")
                .map(|i| json!({"severity": i["severity"], "code": i["code"], "message": i["message"], "fix": i["fix"]}))
                .collect()
        })
        .unwrap_or_default();
    let text = format!(
        "The part: {line}.\n\
         Current spec: {spec}\n\
         Any Part Made checked the printed file and answered verdict {verdict} with these issues (locations in the frame above):\n{issues}\n\
         {notes}\
         Change the spec so every error goes away, with the smallest change that keeps the knob's look. \
         Map each location to the feature that makes it (side wall, pointer, grip, top, cavity, bore). \
         Fix warnings too when it costs nothing. Do not touch the bore diameter or flat. \
         In changes, list every field you changed, from and to, and which issue it fixes.",
        spec = serde_json::to_string(spec).unwrap_or_default(),
        verdict = check["verdict"].as_str().unwrap_or("?"),
        issues = serde_json::to_string_pretty(&issues).unwrap_or_default(),
    );
    json!({
        "model": MODEL,
        "messages": [
            {"role": "system", "content": FRAME},
            {"role": "user", "content": text}
        ],
        "response_format": response_format("knob_fix"),
        "max_completion_tokens": 12000,
    })
}

#[derive(Debug)]
pub struct Answer {
    pub spec: KnobSpec,
    pub changes: Value,
    pub summary: String,
}

/// Reads the reply. Missing optional parts (skirt null) map onto the Rust spec.
pub fn parse(reply: &Value) -> Result<Answer, String> {
    let choice = &reply["choices"][0];
    let content = choice["message"]["content"].as_str().ok_or_else(|| {
        format!(
            "Muse sent no answer (finish reason {}).",
            choice["finish_reason"].as_str().unwrap_or("unknown")
        )
    })?;
    let v: Value =
        serde_json::from_str(content).map_err(|_| "Muse's answer is not JSON.".to_string())?;
    let mut spec = v["spec"].clone();
    if spec["body"]["skirt"].is_null() {
        if let Some(b) = spec["body"].as_object_mut() {
            b.remove("skirt");
        }
    }
    let spec: KnobSpec =
        serde_json::from_value(spec).map_err(|e| format!("Muse's spec does not fit: {e}"))?;
    Ok(Answer {
        spec,
        changes: v["changes"].clone(),
        summary: v["summary"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(400)
            .collect(),
    })
}

/// Estimated cost of one reply from its token usage.
pub fn cost_usd(reply: &Value) -> f64 {
    let u = &reply["usage"];
    let prompt = u["prompt_tokens"].as_f64().unwrap_or(0.0);
    let cached = u["prompt_tokens_details"]["cached_tokens"]
        .as_f64()
        .unwrap_or(0.0)
        .min(prompt);
    let out = u["completion_tokens"].as_f64().unwrap_or(0.0);
    (prompt - cached) * USD_IN + cached * USD_CACHED + out * USD_OUT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(content: &Value) -> Value {
        json!({
            "choices": [{"message": {"role": "assistant", "content": content.to_string()}}],
            "usage": {"prompt_tokens": 2000, "completion_tokens": 500, "prompt_tokens_details": {"cached_tokens": 1000}}
        })
    }

    #[test]
    fn answer_parses_into_a_valid_spec() {
        let mut spec = serde_json::to_value(KnobSpec::around_shaft(6.0, 4.6)).unwrap();
        spec["body"]["skirt"] = Value::Null;
        let r = reply(&json!({
            "spec": spec,
            "changes": [{"field": "pointer.kind", "from": "groove", "to": "ridge", "why": "THIN_WALL at the groove"}],
            "summary": "Ridge instead of groove."
        }));
        let a = parse(&r).unwrap();
        assert!(a.spec.body.skirt.is_none());
        a.spec.validate().unwrap();
        assert_eq!(a.changes[0]["to"], "ridge");
        let c = cost_usd(&r);
        assert!((c - (1000.0 * 1.25 + 1000.0 * 0.15 + 500.0 * 4.25) / 1e6).abs() < 1e-12);
    }

    #[test]
    fn unknown_fields_and_bad_json_are_refused() {
        let mut spec = serde_json::to_value(KnobSpec::around_shaft(6.0, 4.6)).unwrap();
        spec["code"] = json!("import os");
        assert!(parse(&reply(&json!({"spec": spec, "changes": [], "summary": ""}))).is_err());
        let bad = json!({"choices": [{"message": {"content": "not json"}}]});
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn requests_have_the_documented_shape() {
        let k = KnobSpec::around_shaft(6.0, 4.6);
        let d = design_request(
            "control knob, electric oven",
            &json!({"shaft": [6.0, 4.6]}),
            "data:image/jpeg;base64,AA",
            &k,
        );
        assert_eq!(d["model"], MODEL);
        assert_eq!(d["messages"][1]["content"][1]["type"], "image_url");
        assert_eq!(d["response_format"]["type"], "json_schema");
        let check = json!({"verdict": "needs_changes", "manufacturability": [
            {"severity": "error", "code": "THIN_WALL", "message": "0.22 mm", "fix": "thicken"},
            {"severity": "info", "code": "HOLES_FOUND", "message": "", "fix": ""}
        ]});
        let f = fix_request("knob", &k, &check, "");
        let text = f["messages"][1]["content"].as_str().unwrap();
        assert!(text.contains("THIN_WALL") && !text.contains("HOLES_FOUND"));
        assert!(!serde_json::to_string(&f).unwrap().contains('\u{2014}'));
    }
}
