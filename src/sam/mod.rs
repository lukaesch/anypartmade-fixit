//! Meta SAM 3.1 through the Meta Model API (`POST /v1/responses`, model `sam-3.1`).
//! Request and reply shapes follow https://dev.meta.ai/docs/sam/segmenting and
//! https://dev.meta.ai/docs/sam/reading-segmentation.

pub mod codec;

use serde_json::{json, Value};

pub const URL: &str = "https://api.meta.ai/v1/responses";
pub const MODEL: &str = "sam-3.1";
/// $2.50 per 1,000 images (https://dev.meta.ai/docs/pricing-rate-limits).
pub const USD_PER_IMAGE: f64 = 0.0025;

/// One concept prompt (a short noun phrase) on one image. Non streaming, not stored
/// (large inline images with `store: true` can be refused with 413), with confidence.
pub fn request(concept: &str, image_data_url: &str) -> Value {
    json!({
        "model": MODEL,
        "input": [{
            "type": "message",
            "role": "user",
            "content": [
                {"type": "input_text", "text": concept},
                {"type": "input_image", "image_url": image_data_url}
            ]
        }],
        "stream": false,
        "store": false,
        "metadata": {"include_confidence": "true"}
    })
}

/// The segmentation text: all `output_text` parts of all output messages, joined.
pub fn output_text(reply: &Value) -> String {
    reply["output"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|c| c["type"] == "output_text")
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

pub fn images_processed(reply: &Value) -> u64 {
    reply["usage"]["images_processed"].as_u64().unwrap_or(1)
}

/// One segmented object: its box in source pixels and its mask raster.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub id: String,
    /// Inclusive box corners, source pixels.
    pub x1: u32,
    pub y1: u32,
    pub x2: u32,
    pub y2: u32,
    /// Source image size.
    pub image_w: u32,
    pub image_h: u32,
    pub confidence: Option<f64>,
    pub mask: Mask,
}

/// Coverage raster (0 to 255) that sits inside the object's box.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<u8>,
}

impl Object {
    pub fn box_w(&self) -> f64 {
        (self.x2 - self.x1 + 1) as f64
    }

    pub fn box_h(&self) -> f64 {
        (self.y2 - self.y1 + 1) as f64
    }

    /// Image position of a point given in mask pixel units (col, row). The raster is
    /// stretched over the box; in every documented example it is exactly box sized.
    pub fn to_image(&self, col: f64, row: f64) -> (f64, f64) {
        let (sx, sy) = self.scale();
        (self.x1 as f64 + col * sx, self.y1 as f64 + row * sy)
    }

    /// Image pixels per mask pixel (x, y).
    pub fn scale(&self) -> (f64, f64) {
        (
            self.box_w() / self.mask.width as f64,
            self.box_h() / self.mask.height as f64,
        )
    }
}

fn fields(body: &str) -> Vec<(&str, &str)> {
    body.split(';')
        .filter_map(|f| {
            let f = f.trim();
            if f.is_empty() {
                return None;
            }
            Some(f.split_once('=').unwrap_or((f, "")))
        })
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect()
}

fn get<'a>(f: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    f.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn num<T: std::str::FromStr>(f: &[(&str, &str)], key: &str) -> Result<T, String> {
    get(f, key)
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("SAM record has no usable {key}."))
}

/// Parses SAM output text (`<0f>id<|box;...|><|mask;...|>,id...`) for a single image.
/// Empty text means no match, which is not an error.
pub fn parse(text: &str) -> Result<Vec<Object>, String> {
    let mut objects = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let rest = line
            .strip_prefix('<')
            .and_then(|l| l.split_once('>'))
            .filter(|(head, _)| head.split(';').next().is_some_and(|h| h.ends_with('f')))
            .map(|(_, r)| r)
            .ok_or("SAM output has no frame marker.")?;
        let mut s = rest;
        while !s.is_empty() {
            s = s.strip_prefix(',').unwrap_or(s);
            let id_len = s.bytes().take_while(u8::is_ascii_digit).count();
            if id_len == 0 {
                return Err("SAM record has no object id.".into());
            }
            let id = s[..id_len].to_string();
            s = &s[id_len..];
            let (mut boxf, mut maskf) = (None, None);
            while let Some(t) = s.strip_prefix("<|") {
                let end = t.find("|>").ok_or("SAM token is not closed.")?;
                let token = &t[..end];
                let (name, body) = token.split_once(';').unwrap_or((token, ""));
                match name {
                    "box" => boxf = Some(fields(body)),
                    "mask" => maskf = Some(fields(body)),
                    _ => {}
                }
                s = &t[end + 2..];
            }
            let b = boxf.ok_or("SAM record has no box.")?;
            let m = maskf.ok_or("SAM record has no mask.")?;
            let data = get(&m, "data").ok_or("SAM mask has no data.")?;
            let mut parts = data.splitn(3, ',');
            let mh: usize = parts
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or("SAM mask has no height.")?;
            let mw: usize = parts
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or("SAM mask has no width.")?;
            let payload = parts.next().ok_or("SAM mask has no payload.")?;
            let o = Object {
                id,
                x1: num(&b, "x1")?,
                y1: num(&b, "y1")?,
                x2: num(&b, "x2")?,
                y2: num(&b, "y2")?,
                image_w: num(&b, "w")?,
                image_h: num(&b, "h")?,
                confidence: get(&b, "c").and_then(|c| c.parse().ok()),
                mask: Mask {
                    width: mw,
                    height: mh,
                    coverage: codec::decode(payload, mw, mh)?,
                },
            };
            if o.x2 < o.x1 || o.y2 < o.y1 || o.x2 >= o.image_w || o.y2 >= o.image_h {
                return Err("SAM box is outside the image.".into());
            }
            objects.push(o);
        }
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const DOC_LINE: &str = "<0f>0<|box;x1=211;y1=228;x2=270;y2=254;w=320;h=334|><|mask;x=0;y=0;data=27,60,~!!!!M!0c[0o91w?q1!pIH4pPRVp2B3'7`e.ioeAf6-k/#Xd8%dX9x(|>,1<|box;x1=155;y1=228;x2=202;y2=254;w=320;h=334;c=0.96|><|mask;x=0;y=0;c=0.96;data=27,48,~!!!!J!0c[q=Pj_zs=*4C4(/./x#:/`S_GnD`=o3?X{emCgO$y@|>";

    #[test]
    fn parses_the_documented_line() {
        let o = parse(DOC_LINE).expect("parse");
        assert_eq!(o.len(), 2);
        assert_eq!(
            (o[0].id.as_str(), o[0].x1, o[0].y2, o[0].image_w),
            ("0", 211, 254, 320)
        );
        assert_eq!((o[0].mask.width, o[0].mask.height), (60, 27));
        assert_eq!(o[0].box_w(), 60.0);
        assert_eq!(o[1].confidence, Some(0.96));
        assert_eq!(o[0].confidence, None);
        assert_eq!(o[1].to_image(0.5, 0.5), (155.5, 228.5));
        assert!(parse("").expect("empty").is_empty());
        assert!(parse("<0f>0<|box;x1=1|>").is_err());
    }

    #[test]
    fn reads_the_reply() {
        let reply = json!({
            "output": [{"type": "message", "content": [
                {"type": "output_text", "text": "<0f>"},
                {"type": "output_text", "text": "0<|box"}
            ]}],
            "usage": {"images_processed": 1}
        });
        assert_eq!(output_text(&reply), "<0f>0<|box");
        assert_eq!(images_processed(&reply), 1);
        let r = request("coin", "data:image/jpeg;base64,AAAA");
        assert_eq!(r["model"], "sam-3.1");
        assert_eq!(
            r["input"][0]["content"][1]["image_url"],
            "data:image/jpeg;base64,AAAA"
        );
        assert_eq!(r["metadata"]["include_confidence"], "true");
    }
}
