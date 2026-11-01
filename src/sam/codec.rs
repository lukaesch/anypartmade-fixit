//! SAM 3.1 mask payload decoder.
//!
//! A Rust port of `mask-codec.js` from Meta's `@meta-sam/parser` 0.1.0
//! (Copyright (c) Meta Platforms, Inc. and affiliates), used under the SAM License
//! (see `LICENSE-SAM`). The wire format is not documented beyond that package:
//! a custom base85 alphabet with a 5 digit length prefix, then a range coded raster.
//! `!` marks one_bit masks (context coded bits), `~` marks lossless masks
//! (8 bit coverage, predicted and range coded).

const RADIX: u64 = 85;
const PREFIX: usize = 5;
const TOP: u32 = 1 << 24;
const BOTTOM: u32 = 1 << 16;
const INCREMENT: u16 = 14;
const COUNT_LIMIT: u32 = 4096;
const CONTEXTS: usize = 1 << 12;
const PADDING: usize = 2;
const SYMBOLS: usize = 256;
const ZERO_INCREMENT: u32 = 32;
const ZERO_LIMIT: u32 = 16384;
const ORDER_ONE_INCREMENT: u16 = 56;
const ORDER_ONE_LIMIT: i32 = 8192;
const ORDER_TWO_STEP: u16 = 14 * 44;
const ORDER_TWO_LIMIT: u16 = 14 * 3584;
const ORDER_ZERO_STEP: u16 = 2 * 16;
const ORDER_ZERO_LIMIT: i64 = 2 * 2048;

/// Printable ASCII `!` to `~` without the wire delimiters `" \ , ; < > |`.
fn alphabet() -> Vec<u8> {
    (0x21u8..=0x7e)
        .filter(|c| !b"\"\\,;<>|".contains(c))
        .collect()
}

fn digit_table() -> &'static [i16; 128] {
    static TABLE: std::sync::OnceLock<[i16; 128]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [-1i16; 128];
        for (i, &c) in alphabet().iter().enumerate() {
            t[c as usize] = i as i16;
        }
        t
    })
}

fn digit(c: u8) -> Result<u64, String> {
    match digit_table().get(c as usize) {
        Some(&v) if v >= 0 && (v as u64) < RADIX => Ok(v as u64),
        _ => Err("Mask payload contains an invalid character.".into()),
    }
}

pub fn pack(input: &[u8]) -> Result<String, String> {
    let a = alphabet();
    let mut out = Vec::with_capacity(PREFIX + input.len() / 4 * 5 + 5);
    let mut len = input.len() as u64;
    let mut prefix = [0u8; PREFIX];
    for p in prefix.iter_mut().rev() {
        *p = a[(len % RADIX) as usize];
        len /= RADIX;
    }
    if len != 0 {
        return Err("Mask payload is too large.".into());
    }
    out.extend_from_slice(&prefix);
    let mut chunks = input.chunks_exact(4);
    for c in &mut chunks {
        let mut v = u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as u64;
        let mut d = [0u8; 5];
        for x in d.iter_mut().rev() {
            *x = a[(v % RADIX) as usize];
            v /= RADIX;
        }
        out.extend_from_slice(&d);
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut b = [0u8; 4];
        b[..rem.len()].copy_from_slice(rem);
        let mut v = u32::from_be_bytes(b) as u64;
        let mut d = [0u8; 5];
        for x in d.iter_mut().rev() {
            *x = a[(v % RADIX) as usize];
            v /= RADIX;
        }
        out.extend_from_slice(&d[..rem.len() + 1]);
    }
    String::from_utf8(out).map_err(|e| e.to_string())
}

pub fn unpack(payload: &str) -> Result<Vec<u8>, String> {
    let p = payload.as_bytes();
    if p.len() < PREFIX {
        return Err("Mask payload is missing its length prefix.".into());
    }
    let mut len: u64 = 0;
    for &c in &p[..PREFIX] {
        len = len * RADIX + digit(c)?;
    }
    let len = len as usize;
    let rem = len % 4;
    let expected = PREFIX + len / 4 * 5 + if rem == 0 { 0 } else { rem + 1 };
    if p.len() != expected {
        return Err("Mask payload length does not match its prefix.".into());
    }
    let mut out = vec![0u8; len];
    let mut src = PREFIX;
    let full = len - rem;
    let mut dst = 0;
    while dst < full {
        let mut v: u64 = 0;
        for i in 0..5 {
            v = v * RADIX + digit(p[src + i])?;
        }
        if v > u32::MAX as u64 {
            return Err("Mask payload contains an out-of-range group.".into());
        }
        out[dst..dst + 4].copy_from_slice(&(v as u32).to_be_bytes());
        src += 5;
        dst += 4;
    }
    if rem > 0 {
        let mut v: u64 = 0;
        for i in 0..5 {
            v = v * RADIX
                + if i < rem + 1 {
                    digit(p[src + i])?
                } else {
                    RADIX - 1
                };
        }
        if v > u32::MAX as u64 {
            return Err("Mask payload contains an out-of-range tail.".into());
        }
        let b = (v as u32).to_be_bytes();
        out[dst..dst + rem].copy_from_slice(&b[..rem]);
    }
    if pack(&out)? != payload {
        return Err("Mask payload is not in canonical form.".into());
    }
    Ok(out)
}

/// Shared renormalisation of the range coder: shift out a byte while the top byte is
/// settled, or while the range is too small (then the range is clamped to the boundary).
fn needs_shift(low: u32, range: &mut u32) -> bool {
    if (low ^ low.wrapping_add(*range)) < TOP {
        return true;
    }
    if *range < BOTTOM {
        *range = low.wrapping_neg() & (BOTTOM - 1);
        return true;
    }
    false
}

#[cfg(test)]
struct RangeEncoder {
    low: u32,
    range: u32,
    out: Vec<u8>,
}

#[cfg(test)]
impl RangeEncoder {
    fn new() -> Self {
        RangeEncoder {
            low: 0,
            range: u32::MAX,
            out: Vec::new(),
        }
    }

    fn encode(&mut self, cumulative: u32, frequency: u32, total: u32) {
        let scaled = self.range / total;
        self.low = self.low.wrapping_add(scaled.wrapping_mul(cumulative));
        self.range = scaled.wrapping_mul(frequency);
        while needs_shift(self.low, &mut self.range) {
            self.out.push((self.low >> 24) as u8);
            self.low <<= 8;
            self.range <<= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        for _ in 0..4 {
            self.out.push((self.low >> 24) as u8);
            self.low <<= 8;
        }
        self.out
    }
}

struct RangeDecoder<'a> {
    input: &'a [u8],
    pos: usize,
    low: u32,
    range: u32,
    code: u32,
    scaled: u32,
}

impl<'a> RangeDecoder<'a> {
    fn new(input: &'a [u8]) -> Result<Self, String> {
        if input.len() < 4 {
            return Err("Mask payload is missing its finalization.".into());
        }
        let mut d = RangeDecoder {
            input,
            pos: 0,
            low: 0,
            range: u32::MAX,
            code: 0,
            scaled: 0,
        };
        for _ in 0..4 {
            d.code = (d.code << 8) | d.read()? as u32;
        }
        Ok(d)
    }

    fn read(&mut self) -> Result<u8, String> {
        let b = self
            .input
            .get(self.pos)
            .copied()
            .ok_or("Mask payload ended before decoding completed.")?;
        self.pos += 1;
        Ok(b)
    }

    fn frequency(&mut self, total: u32) -> Result<u32, String> {
        self.scaled = self.range / total;
        if self.scaled == 0 {
            return Err("Mask payload has invalid coding state.".into());
        }
        let v = self.code.wrapping_sub(self.low) / self.scaled;
        Ok(v.min(total - 1))
    }

    fn decode(&mut self, cumulative: u32, frequency: u32) -> Result<(), String> {
        self.low = self.low.wrapping_add(self.scaled.wrapping_mul(cumulative));
        self.range = self.scaled.wrapping_mul(frequency);
        while needs_shift(self.low, &mut self.range) {
            self.code = (self.code << 8) | self.read()? as u32;
            self.low <<= 8;
            self.range <<= 8;
        }
        Ok(())
    }
}

fn context_at(r: &[u8], pos: usize, prior: usize, earlier: usize) -> usize {
    (r[pos - 1] as usize)
        | (r[pos - 2] as usize) << 1
        | (r[prior - 2] as usize) << 2
        | (r[prior - 1] as usize) << 3
        | (r[prior] as usize) << 4
        | (r[prior + 1] as usize) << 5
        | (r[prior + 2] as usize) << 6
        | (r[earlier - 2] as usize) << 7
        | (r[earlier - 1] as usize) << 8
        | (r[earlier] as usize) << 9
        | (r[earlier + 1] as usize) << 10
        | (r[earlier + 2] as usize) << 11
}

fn update_counts(counts: &mut [u16], off: usize, bit: usize) {
    counts[off + bit] = counts[off + bit].wrapping_add(INCREMENT);
    if counts[off] as u32 + counts[off + 1] as u32 >= COUNT_LIMIT {
        counts[off] = (counts[off] >> 1).max(1);
        counts[off + 1] = (counts[off + 1] >> 1).max(1);
    }
}

/// one_bit masks: a binary raster, row major, values 0 or 1.
fn decode_one_bit(payload: &str, width: usize, height: usize) -> Result<Vec<u8>, String> {
    let packed = unpack(&payload[1..])?;
    let mut dec = RangeDecoder::new(&packed)?;
    let mut counts = vec![1u16; CONTEXTS * 2];
    let pw = width + 2 * PADDING;
    let mut padded = vec![0u8; pw * (height + PADDING)];
    let mut out = vec![0u8; width * height];
    for y in 0..height {
        let row = (y + PADDING) * pw + PADDING;
        for x in 0..width {
            let pos = row + x;
            let off = context_at(&padded, pos, pos - pw, pos - 2 * pw) << 1;
            let (zero, one) = (counts[off] as u32, counts[off + 1] as u32);
            let v = dec.frequency(zero + one)?;
            let bit = if v < zero { 0 } else { 1 };
            if bit == 0 {
                dec.decode(0, zero)?;
            } else {
                dec.decode(zero, one)?;
            }
            padded[pos] = bit as u8;
            out[y * width + x] = bit as u8;
            update_counts(&mut counts, off, bit);
        }
    }
    Ok(out)
}

#[cfg(test)]
pub fn encode_one_bit(raster: &[u8], width: usize, height: usize) -> String {
    let mut enc = RangeEncoder::new();
    let mut counts = vec![1u16; CONTEXTS * 2];
    let pw = width + 2 * PADDING;
    let mut padded = vec![0u8; pw * (height + PADDING)];
    for y in 0..height {
        let row = (y + PADDING) * pw + PADDING;
        for x in 0..width {
            let pos = row + x;
            let off = context_at(&padded, pos, pos - pw, pos - 2 * pw) << 1;
            let (zero, one) = (counts[off] as u32, counts[off + 1] as u32);
            let bit = raster[y * width + x] as usize;
            if bit == 0 {
                enc.encode(0, zero, zero + one);
            } else {
                enc.encode(zero, one, zero + one);
            }
            padded[pos] = bit as u8;
            update_counts(&mut counts, off, bit);
        }
    }
    format!("!{}", pack(&enc.finish()).expect("pack"))
}

fn paeth(left: i32, up: i32, ul: i32) -> i32 {
    let e = left + up - ul;
    let (dl, du, dul) = ((e - left).abs(), (e - up).abs(), (e - ul).abs());
    if dl <= du && dl <= dul {
        left
    } else if du <= dul {
        up
    } else {
        ul
    }
}

fn predict(
    out: &[u8],
    off: usize,
    x: usize,
    y: usize,
    mode: u8,
    width: usize,
) -> Result<i32, String> {
    let left = if x > 0 { out[off - 1] as i32 } else { 0 };
    let up = if y > 0 { out[off - width] as i32 } else { 0 };
    let ul = if x > 0 && y > 0 {
        out[off - width - 1] as i32
    } else {
        0
    };
    Ok(match mode {
        0 => paeth(left, up, ul),
        1 => up,
        2 => left,
        3 => (left + up) >> 1,
        4 => {
            if ul >= left.max(up) {
                left.min(up)
            } else if ul <= left.min(up) {
                left.max(up)
            } else {
                left + up - ul
            }
        }
        _ => return Err("Lossless mask payload uses an invalid predictor.".into()),
    })
}

fn unzig(value: usize, prediction: i32) -> u8 {
    let v = value as i32;
    let delta = if v & 1 == 1 { -((v + 1) >> 1) } else { v >> 1 };
    ((prediction + delta) & 0xff) as u8
}

fn zero_context(res: &[u8], off: usize, x: usize, y: usize, width: usize) -> usize {
    let z = |b: bool, i: usize| if b { (res[i] == 0) as usize } else { 1 };
    let left = z(x > 0, off.wrapping_sub(1));
    let up = z(y > 0, off.wrapping_sub(width));
    let ul = z(x > 0 && y > 0, off.wrapping_sub(width + 1));
    let ur = z(
        y > 0 && x + 1 < width,
        off.wrapping_sub(width).wrapping_add(1),
    );
    left | (up << 1) | (ul << 2) | (ur << 3)
}

/// lossless masks: 8 bit coverage per pixel, row major.
fn decode_lossless(payload: &str, width: usize, height: usize) -> Result<Vec<u8>, String> {
    let packed = unpack(&payload[1..])?;
    if packed.len() < 5 {
        return Err("Lossless mask payload is truncated.".into());
    }
    let selector = packed[0];
    let mut dec = RangeDecoder::new(&packed[1..])?;
    let n = width * height;
    let mut out = vec![0u8; n];
    let mut res = vec![0u8; n];
    let mut zero_counts = [1u32; 32];
    let mut order_one = vec![1u16; SYMBOLS * SYMBOLS];
    let mut order_one_totals = vec![SYMBOLS as i32; SYMBOLS];
    let mut order_zero = [2u16; SYMBOLS];
    let mut order_zero_total: i64 = SYMBOLS as i64 * 2;
    let mut order_two: std::collections::HashMap<usize, Vec<u16>> = Default::default();
    let mut previous = 0usize;
    for off in 0..n {
        let x = off % width;
        let y = off / width;
        let zo = zero_context(&res, off, x, y, width) << 1;
        let (zero, nonzero) = (zero_counts[zo], zero_counts[zo + 1]);
        let f = dec.frequency(zero + nonzero)?;
        let bit = if f < zero { 0 } else { 1 };
        if bit == 0 {
            dec.decode(0, zero)?;
        } else {
            dec.decode(zero, nonzero)?;
        }
        zero_counts[zo + bit] += ZERO_INCREMENT;
        if zero_counts[zo] + zero_counts[zo + 1] >= ZERO_LIMIT {
            zero_counts[zo] = (zero_counts[zo] >> 1).max(1);
            zero_counts[zo + 1] = (zero_counts[zo + 1] >> 1).max(1);
        }
        let mut symbol = 0usize;
        if bit == 1 {
            let above = if off >= width {
                res[off - width] as usize
            } else {
                0
            };
            let second = order_two
                .entry((previous << 8) | above)
                .or_insert_with(|| vec![0u16; SYMBOLS + 1]);
            let first = previous << 8;
            let excluded = second[0] as i64 + order_one[first] as i64 + order_zero[0] as i64;
            let total =
                second[SYMBOLS] as i64 + order_one_totals[previous] as i64 + order_zero_total
                    - excluded;
            if total <= 0 || total > u32::MAX as i64 {
                return Err("Lossless mask payload is malformed.".into());
            }
            let target = dec.frequency(total as u32)? as i64;
            let mut cumulative: i64 = 0;
            symbol = 1;
            let freq = |s: usize, second: &[u16]| {
                second[s] as i64 + order_one[first + s] as i64 + order_zero[s] as i64
            };
            let mut frequency = freq(1, second);
            while cumulative + frequency <= target {
                cumulative += frequency;
                symbol += 1;
                if symbol >= SYMBOLS {
                    return Err("Lossless mask payload is malformed.".into());
                }
                frequency = freq(symbol, second);
            }
            dec.decode(cumulative as u32, frequency as u32)?;
            second[symbol] = second[symbol].wrapping_add(ORDER_TWO_STEP);
            second[SYMBOLS] = second[SYMBOLS].wrapping_add(ORDER_TWO_STEP);
            if second[SYMBOLS] >= ORDER_TWO_LIMIT {
                let mut t: u32 = 0;
                for v in second.iter_mut().take(SYMBOLS) {
                    *v >>= 1;
                    t += *v as u32;
                }
                second[SYMBOLS] = t as u16;
            }
            order_one[first + symbol] = order_one[first + symbol].wrapping_add(ORDER_ONE_INCREMENT);
            order_one_totals[previous] += ORDER_ONE_INCREMENT as i32;
            if order_one_totals[previous] >= ORDER_ONE_LIMIT {
                let mut t = 0i32;
                for v in &mut order_one[first..first + SYMBOLS] {
                    *v = (*v >> 1).max(1);
                    t += *v as i32;
                }
                order_one_totals[previous] = t;
            }
            order_zero[symbol] = order_zero[symbol].wrapping_add(ORDER_ZERO_STEP);
            order_zero_total += ORDER_ZERO_STEP as i64;
            if order_zero_total >= ORDER_ZERO_LIMIT {
                let mut t = 0i64;
                for v in order_zero.iter_mut() {
                    *v >>= 1;
                    t += *v as i64;
                }
                order_zero_total = t;
            }
        }
        res[off] = symbol as u8;
        out[off] = unzig(symbol, predict(&out, off, x, y, selector, width)?);
        previous = symbol;
    }
    Ok(out)
}

/// Decodes a mask payload to coverage per pixel: 0 to 255, row major, `width * height`.
/// one_bit masks give 0 or 255. Coverage at 129 or more counts as inside, as in the
/// reference parser.
pub fn decode(payload: &str, width: usize, height: usize) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || width.checked_mul(height).is_none_or(|n| n > 16_000_000) {
        return Err("Mask dimensions are not usable.".into());
    }
    match payload.as_bytes().first() {
        Some(b'~') => decode_lossless(payload, width, height),
        Some(b'!') => Ok(decode_one_bit(payload, width, height)?
            .into_iter()
            .map(|b| b * 255)
            .collect()),
        _ => Err("Unknown mask encoding.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphabet_has_85_digits() {
        let a = alphabet();
        assert_eq!(a.len(), 87);
        assert!(!a.contains(&b'|') && !a.contains(&b',') && a.contains(&b'~'));
    }

    #[test]
    fn pack_round_trips() {
        for n in 0..12 {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let p = pack(&data).expect("pack");
            assert_eq!(unpack(&p).expect("unpack"), data);
        }
        assert!(unpack("!!!!").is_err());
    }

    #[test]
    fn one_bit_round_trips() {
        let (w, h) = (37, 23);
        let raster: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f64 - 18.0, (i / w) as f64 - 11.0);
                (x * x + y * y < 90.0) as u8
            })
            .collect();
        let payload = encode_one_bit(&raster, w, h);
        let got = decode(&payload, w, h).expect("decode");
        assert_eq!(got, raster.iter().map(|b| b * 255).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod reference {
    use super::decode;
    use serde_json::Value;

    /// The two lossless payloads from Meta's docs decode to the same rasters as
    /// Meta's own JavaScript parser.
    #[test]
    fn doc_payloads_match_the_reference_parser() {
        let f: Value =
            serde_json::from_str(include_str!("fixtures/doc_masks.json")).expect("fixture");
        for c in f["cases"].as_array().expect("cases") {
            let (w, h) = (
                c["width"].as_u64().unwrap() as usize,
                c["height"].as_u64().unwrap() as usize,
            );
            let cov = decode(c["payload"].as_str().unwrap(), w, h).expect("decode");
            let rows: Vec<String> = cov
                .chunks(w)
                .map(|r| {
                    r.iter()
                        .map(|&v| if v >= 129 { '1' } else { '0' })
                        .collect()
                })
                .collect();
            let want: Vec<String> = c["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap().to_string())
                .collect();
            assert_eq!(rows, want);
        }
    }
}
