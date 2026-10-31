//! Photo checks before anything is stored: format from the magic bytes, size and pixel
//! dimensions read from the file header. The page resizes to 2048 px before upload.

use sha2::{Digest, Sha256};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EDGE: u32 = 2048;
pub const MIN_EDGE: u32 = 480;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
}

impl Kind {
    pub fn content_type(self) -> &'static str {
        match self {
            Kind::Jpeg => "image/jpeg",
            Kind::Png => "image/png",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg",
            Kind::Png => "png",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhotoInfo {
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
    /// Hex SHA-256 of the file. Also the cache key for model results.
    pub sha256: String,
}

fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 24 || &b[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    Some((w, h))
}

/// Walks the JPEG markers up to the first start of frame.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let marker = b[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            if i + 9 > b.len() {
                return None;
            }
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

pub fn inspect(b: &[u8]) -> Result<PhotoInfo, String> {
    if b.len() > MAX_BYTES {
        return Err("The photo is larger than 8 MB.".into());
    }
    let (kind, size) = if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        (Kind::Jpeg, jpeg_size(b))
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        (Kind::Png, png_size(b))
    } else {
        return Err("Send the photo as JPEG or PNG.".into());
    };
    let (width, height) = size.ok_or("The photo file looks damaged.")?;
    if width.max(height) > MAX_EDGE {
        return Err(format!(
            "The photo is {width} x {height} px. The longest side can be at most {MAX_EDGE} px."
        ));
    }
    if width.min(height) < MIN_EDGE {
        return Err(format!(
            "The photo is {width} x {height} px. It needs at least {MIN_EDGE} px on each side to measure."
        ));
    }
    Ok(PhotoInfo {
        kind,
        width,
        height,
        bytes: b.len(),
        sha256: hex::encode(Sha256::digest(b)),
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Smallest JPEG header the parser needs: SOI, an APP0 segment, SOF0 with the size.
    pub fn fake_jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        b.extend([0xFF, 0xC0, 0x00, 0x11, 0x08]);
        b.extend(h.to_be_bytes());
        b.extend(w.to_be_bytes());
        b.extend([0x03; 12]);
        b.extend([0xFF, 0xD9]);
        b
    }

    fn fake_png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 2, 0, 0, 0]);
        b
    }

    #[test]
    fn reads_jpeg_and_png_sizes() {
        let j = inspect(&fake_jpeg(2048, 1536)).expect("jpeg");
        assert_eq!((j.kind, j.width, j.height), (Kind::Jpeg, 2048, 1536));
        assert_eq!(j.sha256.len(), 64);
        let p = inspect(&fake_png(1000, 800)).expect("png");
        assert_eq!((p.kind, p.width, p.height), (Kind::Png, 1000, 800));
    }

    #[test]
    fn refuses_bad_photos() {
        assert!(inspect(b"GIF89a....")
            .expect_err("gif")
            .contains("JPEG or PNG"));
        assert!(inspect(&fake_jpeg(4032, 3024))
            .expect_err("big")
            .contains("at most 2048"));
        assert!(inspect(&fake_jpeg(300, 200))
            .expect_err("small")
            .contains("at least 480"));
        assert!(inspect(&[0xFF, 0xD8, 0xFF, 0x00])
            .expect_err("cut")
            .contains("damaged"));
        assert!(inspect(&vec![0u8; MAX_BYTES + 1])
            .expect_err("huge")
            .contains("8 MB"));
    }
}
