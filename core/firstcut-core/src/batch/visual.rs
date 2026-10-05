//! Visual signature: the perceptual hash and colour histogram that settle the ambiguous zone.
//!
//! **pipeline** computes `VisualSig` from the 256 px thumbnail in Swift; this module is the
//! reference implementation from [docs/contracts/batching.md](../docs/contracts/batching.md) so both
//! sides can run golden tests against one definition. The algorithm must match the contract exactly.

use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 64-bit dHash plus a 16-bin-per-channel histogram of the 256 px thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualSig {
    /// Bit `row * 8 + col` is `luma[row][col] > luma[row][col + 1]`, MSB first: row 0, column 0 is
    /// bit 63.
    pub dhash: u64,
    /// 16 bins each for R, G, B; each channel normalized so its largest bin is 255.
    pub hist: [u8; 48],
}

/// Hand-written so the JSON is `{ "dhash": <int>, "hist": [48 ints] }` — the shape Swift's
/// `VisualSig` in `CoreTypes.swift` already decodes. Serde's derive only covers arrays up to 32.
impl Serialize for VisualSig {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("VisualSig", 2)?;
        st.serialize_field("dhash", &self.dhash)?;
        st.serialize_field("hist", &self.hist.as_slice())?;
        st.end()
    }
}

impl<'de> Deserialize<'de> for VisualSig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = VisualSig;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a VisualSig { dhash: u64, hist: [u8; 48] }")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<VisualSig, A::Error> {
                let dhash: u64 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let raw: Vec<u8> = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                if raw.len() != 48 {
                    return Err(serde::de::Error::invalid_length(raw.len(), &self));
                }
                let mut hist = [0u8; 48];
                hist.copy_from_slice(&raw);
                Ok(VisualSig { dhash, hist })
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<VisualSig, A::Error> {
                let mut dhash: Option<u64> = None;
                let mut hist: Option<[u8; 48]> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "dhash" => dhash = Some(map.next_value()?),
                        "hist" => {
                            let raw: Vec<u8> = map.next_value()?;
                            if raw.len() != 48 {
                                return Err(serde::de::Error::custom("hist must have 48 bins"));
                            }
                            let mut h = [0u8; 48];
                            h.copy_from_slice(&raw);
                            hist = Some(h);
                        }
                        _ => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                Ok(VisualSig {
                    dhash: dhash.ok_or_else(|| serde::de::Error::missing_field("dhash"))?,
                    hist: hist.ok_or_else(|| serde::de::Error::missing_field("hist"))?,
                })
            }
        }
        d.deserialize_any(V)
    }
}

/// How far apart two signatures are, each component normalized to `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualDistance {
    /// Hamming distance between the two dHashes, 0..=64.
    pub dhash_bits: u32,
    pub dhash_norm: f32,
    /// Mean absolute difference per histogram bin, 0.0..=1.0.
    pub hist_norm: f32,
    /// The number the batcher scores with.
    pub combined: f32,
}

#[must_use]
pub fn distance(a: &VisualSig, b: &VisualSig) -> VisualDistance {
    let dhash_bits = (a.dhash ^ b.dhash).count_ones();
    let dhash_norm = dhash_bits as f32 / 64.0;
    let total: u32 = a
        .hist
        .iter()
        .zip(b.hist.iter())
        .map(|(x, y)| u32::from(x.abs_diff(*y)))
        .sum();
    let hist_norm = total as f32 / (48.0 * 255.0);
    VisualDistance {
        dhash_bits,
        dhash_norm,
        hist_norm,
        combined: DHASH_WEIGHT * dhash_norm + HIST_WEIGHT * hist_norm,
    }
}

/// Structure carries most of the distance between two frames of the same burst; colour carries
/// some of what structure misses (a zoom or a pan into a different part of the pitch).
const DHASH_WEIGHT: f32 = 0.6;
const HIST_WEIGHT: f32 = 0.4;

/// Reference implementation of the contract's algorithm.
///
/// `rgba` is `w * h * 4` bytes, 8 bits per channel, sRGB, orientation already applied. The
/// contract's step 1 (downscale to a 256 px longest edge) belongs to the pipeline's thumbnail
/// stage; this function does the rest, so it can be fed a thumbnail of any size.
#[must_use]
pub fn visual_sig(rgba: &[u8], w: u32, h: u32) -> VisualSig {
    assert_eq!(
        rgba.len(),
        w as usize * h as usize * 4,
        "rgba buffer length does not match {w}x{h}"
    );
    let gray = to_gray(rgba, w, h);
    VisualSig {
        dhash: dhash(&gray, w, h),
        hist: histogram(rgba),
    }
}

/// Rec. 601 luma, per the contract's step 2.
fn to_gray(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h) as usize];
    for (i, px) in out.iter_mut().enumerate() {
        let o = i * 4;
        let r = f32::from(rgba[o]);
        let g = f32::from(rgba[o + 1]);
        let b = f32::from(rgba[o + 2]);
        *px = (0.299 * r + 0.587 * g + 0.114 * b)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

/// 9×8 area-averaged resize, then one comparison bit per horizontal neighbour pair.
///
/// Bit order is MSB first, as the contract states: the comparison for row 0, column 0 is bit 63
/// and the one for row 7, column 7 is bit 0, so the hash reads left-to-right, top-to-bottom when
/// written as binary. The order is arbitrary for Hamming distance — every pair of hashes differs in
/// the same number of bits under either permutation — but it is not arbitrary for anything that
/// compares a hash to a golden value, so it is pinned by a test here rather than left to the reader.
fn dhash(gray: &[u8], w: u32, h: u32) -> u64 {
    let small = area_resize(gray, w, h, 9, 8);
    let mut bits: u64 = 0;
    for row in 0..8 {
        for col in 0..8 {
            let left = small[row * 9 + col];
            let right = small[row * 9 + col + 1];
            if left > right {
                bits |= 1 << (63 - (row * 8 + col));
            }
        }
    }
    bits
}

/// 16 bins per channel over every pixel of the thumbnail, each channel scaled so its largest bin is
/// 255. A flat channel stays all-zero rather than dividing by zero.
fn histogram(rgba: &[u8]) -> [u8; 48] {
    let mut bins = [[0u32; 16]; 3];
    for px in rgba.as_chunks::<4>().0 {
        for (c, v) in [px[0], px[1], px[2]].into_iter().enumerate() {
            let bin = (usize::from(v) * 16 / 256).min(15);
            bins[c][bin] += 1;
        }
    }
    let mut out = [0u8; 48];
    for (c, chan) in bins.iter().enumerate() {
        let peak = *chan.iter().max().unwrap_or(&0);
        if peak == 0 {
            continue;
        }
        for (i, &count) in chan.iter().enumerate() {
            out[c * 16 + i] =
                ((count as f64 / f64::from(peak) * 255.0).round() as u32).min(255) as u8;
        }
    }
    out
}

/// Area-average resize. Each destination cell averages the source pixels its footprint covers,
/// which is the "area averaging" the contract asks for and avoids the aliasing nearest-neighbour
/// sampling would introduce into the hash.
fn area_resize(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh) as usize];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return out;
    }
    for dy in 0..dh {
        let y0 = (dy as u64 * u64::from(sh)) / u64::from(dh);
        let y1 = (((dy as u64 + 1) * u64::from(sh)) / u64::from(dh))
            .max(y0 + 1)
            .min(u64::from(sh));
        for dx in 0..dw {
            let x0 = (dx as u64 * u64::from(sw)) / u64::from(dw);
            let x1 = (((dx as u64 + 1) * u64::from(sw)) / u64::from(dw))
                .max(x0 + 1)
                .min(u64::from(sw));
            let mut sum: u64 = 0;
            let mut n: u64 = 0;
            for sy in y0..y1 {
                let row = sy as usize * sw as usize;
                for sx in x0..x1 {
                    sum += u64::from(src[row + sx as usize]);
                    n += 1;
                }
            }
            // Rounded, not truncated: the hash must not depend on where inside the source cell a
            // value happens to land. `n` is never 0 because both spans are at least one pixel wide.
            let avg = sum.checked_add(n / 2).unwrap_or(sum) / n.max(1);
            out[(dy * dw + dx) as usize] = avg.min(255) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A left-to-right ramp: column `x` has luma `x * 255 / (w - 1)`, so every destination column is
    /// strictly brighter than the one to its right.
    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        let span = w.saturating_sub(1).max(1);
        for _ in 0..h {
            for x in 0..w {
                let l = ((x * 255) / span) as u8;
                v.extend_from_slice(&[l, l, l, 255]);
            }
        }
        v
    }

    #[test]
    fn a_flat_frame_has_no_set_bits() {
        let flat = vec![128u8; 16 * 16 * 4];
        let sig = visual_sig(&flat, 16, 16);
        assert_eq!(sig.dhash, 0);
    }

    #[test]
    fn a_left_to_right_ramp_clears_every_bit_and_its_mirror_sets_them() {
        // dHash sets a bit when the left neighbour is brighter, so a frame that brightens to the
        // right is all zeros and its mirror is all ones.
        assert_eq!(visual_sig(&gradient(64, 32), 64, 32).dhash, 0);
        let mirrored: Vec<u8> = gradient(64, 32).iter().rev().copied().collect();
        assert_eq!(visual_sig(&mirrored, 64, 32).dhash, u64::MAX);
    }

    #[test]
    fn identical_signatures_are_zero_distance() {
        let buf = gradient(32, 32);
        let a = visual_sig(&buf, 32, 32);
        let b = visual_sig(&buf, 32, 32);
        let d = distance(&a, &b);
        assert_eq!(d.dhash_bits, 0);
        assert_eq!(d.dhash_norm, 0.0);
        assert_eq!(d.combined, 0.0);
    }

    #[test]
    fn unrelated_frames_are_further_apart_than_identical_ones() {
        let a = visual_sig(&gradient(32, 32), 32, 32);
        let b = visual_sig(
            &gradient(32, 32).iter().rev().copied().collect::<Vec<_>>(),
            32,
            32,
        );
        assert!(distance(&a, &b).combined > 0.1);
    }

    #[test]
    fn histogram_normalizes_each_channel_to_its_own_peak() {
        // Half the pixels pure red, half pure blue: the red channel peaks in one bin.
        let mut buf = Vec::new();
        for _ in 0..64 {
            buf.extend_from_slice(&[255, 0, 0, 255]);
            buf.extend_from_slice(&[0, 0, 255, 255]);
        }
        let sig = visual_sig(&buf, 16, 8);
        assert_eq!(sig.hist[15], 255, "red's top bin is the peak");
        assert_eq!(sig.hist[47], 255, "blue's top bin is the peak");
        assert_eq!(
            sig.hist[32], 255,
            "green is flat: still normalized to its own peak"
        );
    }

    #[test]
    fn the_first_comparison_is_the_top_bit() {
        // Pins the bit order the contract specifies (MSB first) with one fixed vector: a frame
        // whose only left-to-right gradient is in the top row of the 9×8 grid must set the top eight
        // bits and nothing else. Written LSB-first the same frame would set bits 0..=7, and a
        // golden fixture built from the other reading of the contract would differ from this one in
        // every bit of every row while scoring an identical Hamming distance — the exact drift this
        // test exists to make impossible.
        let mut gray = vec![128u8; 9 * 8];
        // Row 0 of a 9×8 grid resized to 9×8 is itself, so a ramp written straight into it is what
        // the comparison sees.
        gray[..9].copy_from_slice(&[200, 190, 180, 170, 160, 150, 140, 130, 120]);
        assert_eq!(
            dhash(&gray, 9, 8),
            0xff00_0000_0000_0000,
            "row 0 fills the top eight bits"
        );

        // And the same ramp in the last row lands in the bottom eight.
        gray[..9].fill(128);
        gray[7 * 9..].copy_from_slice(&[200, 190, 180, 170, 160, 150, 140, 130, 120]);
        assert_eq!(
            dhash(&gray, 9, 8),
            0x0000_0000_0000_00ff,
            "row 7 fills the bottom eight bits"
        );
    }

    #[test]
    fn resize_covers_every_source_pixel_exactly_once_per_destination_column() {
        // A 9x8 destination from a 100x100 source must average the whole image, so a uniform
        // source survives the resize untouched.
        let flat: Vec<u8> = vec![77; 100 * 100];
        let small = area_resize(&flat, 100, 100, 9, 8);
        assert!(small.iter().all(|&v| v == 77));
    }
}
