//! Any Part Made through its public MCP endpoint, like any other agent:
//! JSON-RPC `tools/call` over HTTP. Only request shapes and reply reading live here.

use crate::knob::KnobSpec;
use serde_json::{json, Value};

pub fn rpc(id: u64, tool: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {"name": tool, "arguments": arguments},
    })
}

/// `check_part` for a knob file. The bore axis is z, opening at z = 0, flat towards +x.
/// The D bore is declared as two straight distances, which the core measures exactly:
/// the round diameter across y, and round side to flat along x. (The library D shaft
/// interface would send every knob to a person, because a D cannot be measured as a hole.)
pub fn check_args(spec: &KnobSpec, line: &str, file_base64: &str) -> Value {
    let z = round2(spec.bore.depth_mm / 2.0);
    let a = spec.bore_radius(std::f64::consts::PI);
    let flat = spec.bore_radius(0.0);
    let dims = json!([
        {
            "name": "bore_round",
            "feature": "D bore, round diameter",
            "nominal_mm": round2(2.0 * a),
            "locator": {"kind": "distance", "point_mm": [0.0, -round2(a), z], "to_point_mm": [0.0, round2(a), z]},
        },
        {
            "name": "bore_flat",
            "feature": "D bore, flat to round",
            "nominal_mm": round2(a + flat),
            "locator": {"kind": "distance", "point_mm": [-round2(a), 0.0, z], "to_point_mm": [round2(flat), 0.0, z]},
        },
    ]);
    json!({ "part": {
        "file_base64": file_base64,
        "process": "fdm",
        "material": "asa",
        "use": {
            "description": format!("Replacement {line}. Printed knob that pushes onto the D shaft."),
            "energy_source": energy_source(line),
        },
        "critical_dimensions": dims,
    }})
}

/// `check_part` for a part from the CAD language: the file inline when small, else by
/// URL; the checks come from its fit features.
pub fn check_args_part(
    use_description: &str,
    line: &str,
    material: &str,
    file: Result<&str, &str>,
    critical_dimensions: &[Value],
    interfaces: &[Value],
) -> Value {
    let mut part = json!({
        "process": "fdm",
        "material": if material == "asa" { "asa" } else { "petg" },
        "use": {
            "description": use_description.chars().take(400).collect::<String>(),
            "energy_source": energy_source(&format!("{line} {use_description}")),
        },
    });
    match file {
        Ok(b64) => part["file_base64"] = json!(b64),
        Err(url) => part["file_url"] = json!(url),
    }
    if !critical_dimensions.is_empty() {
        part["critical_dimensions"] = json!(critical_dimensions);
    }
    if !interfaces.is_empty() {
        part["interfaces"] = json!(interfaces);
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

pub const OPTIONS: [&str; 3] = ["opt_express", "opt_standard", "opt_economy"];

/// What the page shows of a quote: options with price and dates, the guarantee, the notes.
pub fn quote_view(q: &Value) -> Value {
    let options: Vec<Value> = q["options"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|o| {
                    json!({
                        "option_id": o["option_id"],
                        "lead_time": o["lead_time"],
                        "price_display": o["price_display"],
                        "price_total": o["price_total"],
                        "delivery_estimate": o["delivery_estimate"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "quote_id": q["quote_id"],
        "currency": q["currency"],
        "ship_to_country": q["ship_to_country"],
        "valid_until": q["valid_until"],
        "test_mode": q["test_mode"],
        "options": options,
        "unavailable_options": q["unavailable_options"],
        "guarantee": q["guarantee"],
        "notes": q["notes"],
    })
}

/// What the page shows of an order link.
pub fn order_view(o: &Value) -> Value {
    json!({
        "order_id": o["order_id"],
        "order_url": o["order_url"],
        "status": o["status"],
        "status_label": o["status_label"],
        "price_display": o["price_display"],
        "lead_time": o["lead_time"],
        "delivery_estimate": o["delivery_estimate"],
        "link_expires": o["link_expires"],
        "test_mode": o["test_mode"],
    })
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
    fn check_declares_the_d_bore_as_two_distances() {
        let mut k = KnobSpec::around_shaft(6.0, 4.6);
        k.bore.clearance_mm = 0.2;
        let a = check_args(&k, "control knob, electric oven", "UEs=");
        let p = &a["part"];
        assert_eq!(p["process"], "fdm");
        assert_eq!(p["material"], "asa");
        assert_eq!(p["use"]["energy_source"], "electric");
        let d = &p["critical_dimensions"];
        assert_eq!(d[0]["nominal_mm"], 6.2);
        assert_eq!(d[0]["locator"]["point_mm"], json!([0.0, -3.1, 6.0]));
        assert_eq!(d[1]["nominal_mm"], 4.8);
        assert_eq!(d[1]["locator"]["to_point_mm"], json!([1.7, 0.0, 6.0]));
        assert!(p.get("interfaces").is_none());
        let a = check_args(&KnobSpec::around_shaft(8.0, 6.0), "knob", "UEs=");
        assert_eq!(a["part"]["use"]["energy_source"], "unknown");
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
