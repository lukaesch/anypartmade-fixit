//! Any Part Made through its public MCP endpoint, like any other agent:
//! JSON-RPC `tools/call` over HTTP. Only request shapes and reply reading live here.

use crate::knob::KnobSpec;
use serde_json::{json, Value};

/// Library D shafts the core knows. Anything else is declared by its bore size only.
const D_SHAFTS: [(&str, f64); 2] = [("d_shaft_6mm", 6.0), ("d_shaft_6_35mm", 6.35)];

pub fn rpc(id: u64, tool: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {"name": tool, "arguments": arguments},
    })
}

/// The library D shaft within 0.1 mm of the measured diameter, if any.
pub fn library_shaft(diameter_mm: f64) -> Option<&'static str> {
    D_SHAFTS
        .iter()
        .find(|(_, d)| (d - diameter_mm).abs() <= 0.1)
        .map(|(id, _)| *id)
}

/// `check_part` for a knob file. The bore axis is z, opening at z = 0.
pub fn check_args(spec: &KnobSpec, line: &str, file_base64: &str) -> Value {
    let point = [0.0, 0.0, spec.bore.depth_mm / 2.0];
    let axis = [0.0, 0.0, 1.0];
    let mut part = json!({
        "file_base64": file_base64,
        "process": "fdm",
        "material": "asa",
        "use": {
            "description": format!("Replacement {line}. Printed knob that pushes onto the D shaft."),
            "energy_source": energy_source(line),
        },
        "critical_dimensions": [{
            "name": "bore",
            "feature": "D bore, round side",
            "nominal_mm": round2(spec.bore.diameter_mm + spec.bore.clearance_mm),
            "locator": {"kind": "hole", "point_mm": point, "axis": axis},
        }],
    });
    if let Some(id) = library_shaft(spec.bore.diameter_mm) {
        part["interfaces"] = json!([{ "id": id, "point_mm": point, "axis": axis }]);
    }
    json!({ "part": part })
}

/// Only what the user wrote. Unknown makes the core ask, which is right for a cooker.
fn energy_source(line: &str) -> &'static str {
    let l = line.to_lowercase();
    if l.contains("gas") {
        "gas"
    } else if l.contains("electric") || l.contains("induction") || l.contains("ceramic") {
        "electric"
    } else {
        "unknown"
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The HTTP body as JSON-RPC: plain JSON, or the last `data:` line of an event stream.
pub fn parse_body(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str(text) {
        return Some(v);
    }
    text.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str(d.trim()).ok())
        .next_back()
}

/// The tool result as JSON: `structuredContent`, else the first text block parsed.
/// A JSON-RPC error or `isError` becomes Err with its message.
pub fn tool_result(reply: &Value) -> Result<Value, String> {
    if let Some(e) = reply.get("error") {
        return Err(e["message"]
            .as_str()
            .unwrap_or("The Any Part Made server answered with an error.")
            .chars()
            .take(300)
            .collect());
    }
    let r = &reply["result"];
    let text = r["content"]
        .as_array()
        .and_then(|c| c.iter().find_map(|b| b["text"].as_str()))
        .unwrap_or("");
    if r["isError"].as_bool() == Some(true) {
        return Err(text.chars().take(300).collect());
    }
    if r["structuredContent"].is_object() {
        return Ok(r["structuredContent"].clone());
    }
    serde_json::from_str(text).map_err(|_| "The Any Part Made reply could not be read.".into())
}

/// What the page keeps of a check: everything the core said (a refusal is shown as is),
/// without the stored file ids.
pub fn summary(check: &Value) -> Value {
    let mut v = check.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("file");
        o.remove("file_id");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_declares_the_bore_and_the_library_shaft() {
        let mut k = KnobSpec::around_shaft(6.0, 4.6);
        k.bore.clearance_mm = 0.15;
        let a = check_args(&k, "control knob, electric oven", "UEs=");
        let p = &a["part"];
        assert_eq!(p["process"], "fdm");
        assert_eq!(p["material"], "asa");
        assert_eq!(p["use"]["energy_source"], "electric");
        assert_eq!(p["critical_dimensions"][0]["nominal_mm"], 6.15);
        assert_eq!(p["critical_dimensions"][0]["locator"]["kind"], "hole");
        assert_eq!(p["interfaces"][0]["id"], "d_shaft_6mm");
        let a = check_args(&KnobSpec::around_shaft(8.0, 6.0), "knob", "UEs=");
        assert!(a["part"].get("interfaces").is_none());
        assert_eq!(a["part"]["use"]["energy_source"], "unknown");
        assert_eq!(library_shaft(6.32), Some("d_shaft_6_35mm"));
    }

    #[test]
    fn body_is_json_or_event_stream() {
        assert_eq!(parse_body("{\"id\":1}").unwrap()["id"], 1);
        let sse = "event: message\ndata: {\"id\":2}\n\n";
        assert_eq!(parse_body(sse).unwrap()["id"], 2);
        assert!(parse_body("<html>").is_none());
    }

    #[test]
    fn tool_result_reads_structured_text_and_errors() {
        let ok = json!({"result": {"structuredContent": {"verdict": "ok"}}});
        assert_eq!(tool_result(&ok).unwrap()["verdict"], "ok");
        let text =
            json!({"result": {"content": [{"type": "text", "text": "{\"verdict\":\"refused\"}"}]}});
        assert_eq!(tool_result(&text).unwrap()["verdict"], "refused");
        let err =
            json!({"result": {"isError": true, "content": [{"type": "text", "text": "bad file"}]}});
        assert_eq!(tool_result(&err).unwrap_err(), "bad file");
        let rpc_err = json!({"error": {"code": -32602, "message": "Invalid params"}});
        assert_eq!(tool_result(&rpc_err).unwrap_err(), "Invalid params");
    }
}
