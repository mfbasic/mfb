//! macOS app-icon `.icns` generation (plan-22-B icns pipeline + plan-22-C
//! squircle mask).
//!
//! [`build_icns`] turns a 1024×1024 source image — the project's resolved `icon`
//! (plan-22-A) or the compiler's embedded default — into a complete
//! multi-resolution `AppIcon.icns` for the app bundle. The source is normalized
//! to a 1024 RGBA canvas, shaped once (scaled into the Big Sur content area and
//! clipped to a squircle so an arbitrary square image reads as a native macOS
//! icon, plan-22-C), then downsampled to every required `.icns` entry size.
//!
//! `image` (PNG decode) and `icns` (`.icns` container + PNG entry encode) are
//! compiler build-time dependencies only; nothing here reaches emitted programs.

use std::path::Path;

use icns::{IconFamily, IconType, Image as IcnsImage, PixelFormat};
use image::imageops::FilterType;
use image::{GenericImageView, RgbaImage};

/// The compiler's embedded default app icon (1024×1024 PNG), used when a project
/// sets no `icon`. Shares the single embedded asset committed with the macOS
/// app-mode codegen.
use crate::target::macos_aarch64::app::icon::APP_ICON_PNG;

/// Side of the working canvas and the required source size (plan-22-B §4.3).
const CANVAS: u32 = 1024;
/// Big Sur icon body on a 1024 grid (plan-22-C §4.2): 100px margin each side.
const CONTENT: u32 = 824;
/// Transparent margin around the content box: `(CANVAS - CONTENT) / 2`.
const MARGIN: u32 = (CANVAS - CONTENT) / 2;
/// Squircle corner radius ≈ 0.2237 × body (rounded-rect approximation).
const CORNER_RADIUS: f32 = 184.0;

/// Every `.icns` entry written into the family: (pixel size, icns icon type).
/// All ten are RGBA/PNG entries (16, 32, 128, 256, 512 at @1x and @2x). Their
/// OSTypes are `icp4 ic11 icp5 ic12 ic07 ic13 ic08 ic14 ic09 ic10`.
const ICON_ENTRIES: &[(u32, IconType)] = &[
    (16, IconType::RGBA32_16x16),       // icp4
    (32, IconType::RGBA32_16x16_2x),    // ic11
    (32, IconType::RGBA32_32x32),       // icp5
    (64, IconType::RGBA32_32x32_2x),    // ic12
    (128, IconType::RGBA32_128x128),    // ic07
    (256, IconType::RGBA32_128x128_2x), // ic13
    (256, IconType::RGBA32_256x256),    // ic08
    (512, IconType::RGBA32_256x256_2x), // ic14
    (512, IconType::RGBA32_512x512),    // ic09
    (1024, IconType::RGBA32_512x512_2x), // ic10
];

/// Build the `AppIcon.icns` bytes from `source` (a provided project `icon`) or
/// the embedded default when `source` is `None` (plan-22-B §4.3).
///
/// A provided `icon` must decode and be exactly 1024×1024; either failure is a
/// build error. The embedded default is valid by construction.
pub(crate) fn build_icns(source: Option<&Path>) -> Result<Vec<u8>, String> {
    let canvas = normalize_source(source)?;
    let shaped = apply_squircle_mask(canvas);

    let mut family = IconFamily::new();
    for &(size, icon_type) in ICON_ENTRIES {
        let scaled = image::imageops::resize(&shaped, size, size, FilterType::Lanczos3);
        let entry = IcnsImage::from_data(PixelFormat::RGBA, size, size, scaled.into_raw())
            .map_err(|err| format!("failed to build {size}×{size} icon entry: {err}"))?;
        family
            .add_icon_with_type(&entry, icon_type)
            .map_err(|err| format!("failed to encode {size}×{size} icon entry: {err}"))?;
    }

    let mut buf = Vec::new();
    family
        .write(&mut buf)
        .map_err(|err| format!("failed to encode .icns: {err}"))?;
    Ok(buf)
}

/// Decode `source` (or the embedded default) to a 1024×1024 RGBA canvas. A
/// provided icon that does not decode, or is not exactly 1024×1024, is a hard
/// error (plan-22-B §4.3 step 2, resolved Open Decision 4).
fn normalize_source(source: Option<&Path>) -> Result<RgbaImage, String> {
    match source {
        Some(path) => {
            let decoded = image::open(path).map_err(|err| {
                format!(
                    "icon '{}' is not a decodable image: {err}",
                    path.display()
                )
            })?;
            let (width, height) = decoded.dimensions();
            if (width, height) != (CANVAS, CANVAS) {
                return Err(format!(
                    "icon '{}' must be {CANVAS}×{CANVAS}, got {width}×{height}",
                    path.display()
                ));
            }
            Ok(decoded.to_rgba8())
        }
        None => {
            let decoded = image::load_from_memory(APP_ICON_PNG)
                .expect("embedded default app icon must be a valid PNG");
            Ok(decoded.to_rgba8())
        }
    }
}

/// Shape the 1024 RGBA `canvas` like a native macOS icon (plan-22-C §4.3):
/// scale the artwork into the centered `CONTENT`×`CONTENT` box and clip it to a
/// squircle (rounded-rect) of radius `CORNER_RADIUS`, leaving the surrounding
/// margin fully transparent. Applied once at 1024; every `.icns` entry inherits
/// the shape via downsampling.
fn apply_squircle_mask(canvas: RgbaImage) -> RgbaImage {
    // 1. Fit the artwork to the content box, centered on a transparent canvas.
    let artwork = image::imageops::resize(&canvas, CONTENT, CONTENT, FilterType::Lanczos3);
    let mut out = RgbaImage::new(CANVAS, CANVAS);
    image::imageops::replace(&mut out, &artwork, MARGIN as i64, MARGIN as i64);

    // 2. Multiply alpha by rounded-rect coverage. Coverage uses the analytic
    //    rounded-rect signed distance with a 1px antialiased edge; the later
    //    1024→size downsample smooths it further.
    let center = CANVAS as f32 / 2.0;
    let inner = CONTENT as f32 / 2.0 - CORNER_RADIUS; // straight-edge half-extent
    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let dx = (x as f32 + 0.5 - center).abs();
        let dy = (y as f32 + 0.5 - center).abs();
        let qx = (dx - inner).max(0.0);
        let qy = (dy - inner).max(0.0);
        let distance = (qx * qx + qy * qy).sqrt() - CORNER_RADIUS;
        let coverage = (0.5 - distance).clamp(0.0, 1.0);
        pixel[3] = (pixel[3] as f32 * coverage).round() as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn build_icns_default_has_all_entries_at_exact_sizes() {
        let bytes = build_icns(None).expect("embedded default icns");
        assert_eq!(&bytes[0..4], b"icns", "icns magic");

        let family = IconFamily::read(Cursor::new(&bytes)).expect("re-read icns");
        for &(size, icon_type) in ICON_ENTRIES {
            let image = family
                .get_icon_with_type(icon_type)
                .unwrap_or_else(|err| panic!("missing {icon_type:?}: {err}"));
            assert_eq!(
                (image.width(), image.height()),
                (size, size),
                "{icon_type:?} decodes to {size}×{size}"
            );
        }
    }

    #[test]
    fn build_icns_rejects_non_1024_source() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("small.png");
        RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 255, 255]))
            .save(&path)
            .unwrap();
        let err = build_icns(Some(&path)).expect_err("non-1024 icon must fail");
        assert!(err.contains("must be 1024×1024"), "unexpected error: {err}");
        assert!(err.contains("64×64"), "reports actual size: {err}");
    }

    #[test]
    fn build_icns_rejects_non_image_source() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bogus.png");
        std::fs::write(&path, b"this is not a PNG").unwrap();
        let err = build_icns(Some(&path)).expect_err("non-image icon must fail");
        assert!(
            err.contains("is not a decodable image"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn squircle_mask_clips_corners_and_margin_keeps_center() {
        // A fully-opaque canvas isolates the mask geometry from source pixels.
        let solid = RgbaImage::from_pixel(CANVAS, CANVAS, image::Rgba([200, 50, 50, 255]));
        let masked = apply_squircle_mask(solid);

        // Center stays opaque.
        assert_eq!(masked.get_pixel(512, 512)[3], 255, "center opaque");
        // Straight top edge just inside the content box stays opaque.
        assert_eq!(
            masked.get_pixel(512, MARGIN + 6)[3],
            255,
            "top-center edge opaque"
        );
        // The content-box corner is clipped away by the squircle.
        assert_eq!(
            masked.get_pixel(MARGIN + 1, MARGIN + 1)[3],
            0,
            "content corner clipped"
        );
        // Everything outside the margin is transparent.
        assert_eq!(masked.get_pixel(8, 8)[3], 0, "margin transparent");
    }
}
