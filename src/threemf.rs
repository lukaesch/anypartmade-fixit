//! A mesh as a 3MF file: a plain zip (deflate, no Zip64) with the three parts a 3MF needs.

use crate::knob::Mesh;
use std::fmt::Write;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#;

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;

/// The 3D model XML, in millimetres.
pub fn model_xml(m: &Mesh, name: &str) -> String {
    let mut s = String::with_capacity(m.vertices.len() * 48 + m.triangles.len() * 40);
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><metadata name="Title">"#);
    s.push_str(&name.replace(['<', '>', '&', '"'], ""));
    s.push_str(r#"</metadata><resources><object id="1" type="model"><mesh><vertices>"#);
    for v in &m.vertices {
        let _ = write!(
            s,
            r#"<vertex x="{:.4}" y="{:.4}" z="{:.4}"/>"#,
            v[0], v[1], v[2]
        );
    }
    s.push_str("</vertices><triangles>");
    for t in &m.triangles {
        let _ = write!(
            s,
            r#"<triangle v1="{}" v2="{}" v3="{}"/>"#,
            t[0], t[1], t[2]
        );
    }
    s.push_str(
        r#"</triangles></mesh></object></resources><build><item objectid="1"/></build></model>"#,
    );
    s
}

pub fn three_mf(m: &Mesh, name: &str) -> Vec<u8> {
    zip(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", model_xml(m, name).as_bytes()),
    ])
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, e) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *e = c;
    }
    let mut c = !0u32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// A zip with deflated entries. Small parts stay well under 4 GB, so no Zip64.
pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let packed = miniz_oxide::deflate::compress_to_vec(data, 6);
        let packed_size = packed.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend(sig.to_le_bytes());
            if central {
                h.extend(20u16.to_le_bytes()); // made by
            }
            h.extend(20u16.to_le_bytes()); // version needed
            h.extend(0u16.to_le_bytes()); // flags
            h.extend(8u16.to_le_bytes()); // deflate
            h.extend(0u16.to_le_bytes()); // time
            h.extend(0x5b61u16.to_le_bytes()); // date: 2025-11-01
            h.extend(crc.to_le_bytes());
            h.extend(packed_size.to_le_bytes());
            h.extend(size.to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra
            if central {
                h.extend(0u16.to_le_bytes()); // comment
                h.extend(0u16.to_le_bytes()); // disk
                h.extend(0u16.to_le_bytes()); // internal attributes
                h.extend(0u32.to_le_bytes()); // external attributes
                h.extend(offset.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend(packed);
        central.extend(header(0x0201_4b50, true));
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]); // disk numbers
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(cd_size.to_le_bytes());
    out.extend(cd_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn knob_3mf_is_a_small_zip() {
        let k = crate::knob::KnobSpec::around_shaft(6.0, 4.6);
        let f = three_mf(&k.mesh(), "Oven knob");
        assert_eq!(&f[..4], b"PK\x03\x04");
        assert_eq!(&f[f.len() - 22..f.len() - 18], b"PK\x05\x06");
        assert!(f.len() < 512 * 1024, "{} bytes", f.len());
    }

    #[test]
    fn entries_inflate_back() {
        let data = b"knob knob knob knob knob".repeat(50);
        let z = zip(&[("a.txt", &data)]);
        let n = 30 + 5;
        let packed = u32::from_le_bytes(z[18..22].try_into().unwrap()) as usize;
        let back = miniz_oxide::inflate::decompress_to_vec(&z[n..n + packed]).unwrap();
        assert_eq!(back, data);
        assert_eq!(
            u32::from_le_bytes(z[14..18].try_into().unwrap()),
            crc32(&data)
        );
    }
}
