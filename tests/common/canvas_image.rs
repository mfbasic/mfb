//! Comparing rendered canvas frames against stored reference images.
//!
//! Two comparators, and the difference between them is plan-98-A's invariant 5:
//!
//! * [`compare_exact`] is the gate for the **software** rasteriser. That path is
//!   deterministic — no driver, no transcendental, exact-coverage AA — so "the same
//!   scene renders to the same pixels" is a fact about it, and anything weaker would
//!   let a real regression through.
//! * [`compare_within_tolerance`] is the gate plan-98-E/F will use for **GPU**
//!   backends, where exact match is the wrong test: rasterisation rules, filtering
//!   and blend precision differ legitimately between drivers, so a GPU frame is
//!   correct when it is *close* to the software oracle, not when it is identical.
//!
//! It lives here, written and tested now rather than when E lands, so invariant 5 is
//! a thing that exists rather than a thing that is promised.
//!
//! ## Why references are stored as PNG and compared as pixels
//!
//! The plan called for a raw `.bin` oracle plus a PNG for humans, on the grounds
//! that PNG codecs vary. They do — but only in the *file bytes* they produce for
//! given pixels, and nothing here ever compares file bytes. A PNG decodes to exactly
//! one pixel array, so decoding and comparing pixels is precisely as exact as a raw
//! blob, while being ~200x smaller and directly viewable. See plan-98-C Correction 9.

use std::path::Path;

/// A decoded RGBA8 image.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, four bytes per pixel.
    pub pixels: Vec<u8>,
}

impl Frame {
    /// Wrap a raw RGBA8 buffer of known dimensions.
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(
            pixels.len(),
            (width as usize) * (height as usize) * 4,
            "buffer is not {width}x{height} RGBA8",
        );
        Frame {
            width,
            height,
            pixels,
        }
    }

    /// Decode a stored reference image.
    pub fn load_png(path: &Path) -> Frame {
        let decoded = image::open(path)
            .unwrap_or_else(|e| panic!("decode {}: {e}", path.display()))
            .to_rgba8();
        Frame {
            width: decoded.width(),
            height: decoded.height(),
            pixels: decoded.into_raw(),
        }
    }

    /// Write this frame out as a PNG.
    pub fn save_png(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create golden directory");
        }
        image::save_buffer(
            path,
            &self.pixels,
            self.width,
            self.height,
            image::ColorType::Rgba8,
        )
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }

    fn pixel(&self, index: usize) -> [u8; 4] {
        let base = index * 4;
        [
            self.pixels[base],
            self.pixels[base + 1],
            self.pixels[base + 2],
            self.pixels[base + 3],
        ]
    }
}

/// What a comparison found, when it found something.
#[derive(Debug)]
pub struct Mismatch {
    /// How many pixels differed at all.
    pub differing_pixels: usize,
    /// The largest absolute per-channel difference anywhere in the frame.
    pub max_channel_delta: u8,
    /// `(x, y)` of the first differing pixel in row-major order, with both values.
    pub first: Option<(u32, u32, [u8; 4], [u8; 4])>,
    /// Total pixels compared, so a caller can report a proportion.
    pub total_pixels: usize,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} of {} pixels differ (max channel delta {})",
            self.differing_pixels, self.total_pixels, self.max_channel_delta,
        )?;
        if let Some((x, y, got, want)) = self.first {
            write!(f, "; first at ({x}, {y}): got {got:?}, want {want:?}")?;
        }
        Ok(())
    }
}

/// How far a GPU frame may drift from the software oracle and still be correct.
///
/// Two independent limits, because one alone is the wrong shape. A per-channel
/// epsilon alone would accept a frame where *every* pixel is slightly wrong, which is
/// a systematic error (a wrong gamma, a half-pixel offset) rather than sampling
/// noise. A differing-pixel budget alone would accept a handful of catastrophically
/// wrong pixels. Requiring both bounds the error in each direction.
#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    /// The largest absolute per-channel difference any single pixel may have.
    pub max_channel_delta: u8,
    /// The largest fraction of pixels that may differ at all, in `0.0..=1.0`.
    pub max_differing_fraction: f64,
}

impl Tolerance {
    /// The placeholder thresholds plan-98-E/F start from.
    ///
    /// Deliberately tight: a GPU backend rendering the *same* analytic SDFs at the
    /// same resolution should differ only in the last step or two of the blend and
    /// only on antialiased edges. These are placeholders in the sense that E will
    /// re-measure them against real driver output and record what it found — not in
    /// the sense that they are guesses to be loosened until something passes.
    pub const GPU_DEFAULT: Tolerance = Tolerance {
        max_channel_delta: 2,
        max_differing_fraction: 0.02,
    };
}

/// Compare two frames pixel for pixel.
///
/// This is the software rasteriser's gate. A failure is a bug hunt — localize it by
/// the reported coordinate and root-cause the primitive — never a signal to
/// regenerate the reference. See AGENTS.md's four-question rule.
pub fn compare_exact(got: &Frame, want: &Frame) -> Result<(), Mismatch> {
    let diff = measure(got, want);
    if diff.differing_pixels == 0 {
        Ok(())
    } else {
        Err(diff)
    }
}

/// Compare two frames, allowing bounded per-pixel drift.
///
/// The comparator plan-98-E/F use against the software oracle. Both limits must
/// hold: no pixel may be off by more than `max_channel_delta` in any channel, and no
/// more than `max_differing_fraction` of pixels may differ at all.
pub fn compare_within_tolerance(
    got: &Frame,
    want: &Frame,
    tolerance: Tolerance,
) -> Result<(), Mismatch> {
    let diff = measure(got, want);
    let fraction = diff.differing_pixels as f64 / diff.total_pixels.max(1) as f64;
    if diff.max_channel_delta <= tolerance.max_channel_delta
        && fraction <= tolerance.max_differing_fraction
    {
        Ok(())
    } else {
        Err(diff)
    }
}

/// The shared measurement both comparators judge.
///
/// Computed once and in full rather than short-circuiting on the first difference:
/// "which pixel differs first" is far less useful for a bug hunt than "how many, and
/// how badly" — a one-pixel seam and a wholly wrong frame need different
/// investigations, and only the totals tell them apart.
fn measure(got: &Frame, want: &Frame) -> Mismatch {
    assert_eq!(
        (got.width, got.height),
        (want.width, want.height),
        "frames differ in size; a size mismatch is a harness bug, not a rendering difference",
    );
    let total_pixels = (got.width as usize) * (got.height as usize);
    let mut differing_pixels = 0;
    let mut max_channel_delta = 0u8;
    let mut first = None;

    for index in 0..total_pixels {
        let a = got.pixel(index);
        let b = want.pixel(index);
        if a == b {
            continue;
        }
        differing_pixels += 1;
        for channel in 0..4 {
            max_channel_delta = max_channel_delta.max(a[channel].abs_diff(b[channel]));
        }
        if first.is_none() {
            let x = (index % got.width as usize) as u32;
            let y = (index / got.width as usize) as u32;
            first = Some((x, y, a, b));
        }
    }

    Mismatch {
        differing_pixels,
        max_channel_delta,
        first,
        total_pixels,
    }
}
