//! Platform-neutral app-icon rendering (plan-51-A §4.2).
//!
//! Every app-mode build starts from the same source: the project's resolved
//! `icon` (plan-22-A) or the compiler's embedded 1024×1024 default. What differs
//! is the container — macOS wants a multi-resolution `.icns`
//! (`crate::os::macos::icon`), Linux wants loose PNGs at the hicolor sizes — so
//! the *decode + validate + default-fallback* front half lives here and is shared
//! by both. A project `icon` accepted on one platform is therefore accepted on
//! both, and one rejected on one is rejected on both, by construction rather than
//! by two copies of the rule agreeing.
//!
//! `image` (PNG decode/encode) is a compiler build-time dependency only; nothing
//! here reaches emitted programs.

mod default_png;

use std::path::Path;

use image::imageops::FilterType;
use image::{GenericImageView, RgbaImage};

pub(crate) use default_png::APP_ICON_PNG;

/// Side of the working canvas and the required source size (plan-22-B §4.3).
pub(crate) const CANVAS: u32 = 1024;

/// The freedesktop hicolor icon sizes an AppDir installs into
/// `usr/share/icons/hicolor/<N>x<N>/apps/` (plan-51-A §4.2).
///
/// All seven are downsamples of the same 1024 source, so no entry is ever an
/// upscale.
pub(crate) const HICOLOR_SIZES: [u32; 7] = [16, 32, 48, 64, 128, 256, 512];

/// The size of the AppDir's root `<name>.png` and `.DirIcon` (plan-51-A §4.1).
/// appimagetool copies the largest icon it finds from the AppDir root, and 256 is
/// the desktop convention; 512 would double the root PNG for no consumer.
pub(crate) const ROOT_ICON_SIZE: u32 = 256;

/// Decode `source` (or the embedded default) to a 1024×1024 RGBA canvas. A
/// provided icon that does not decode, or is not exactly 1024×1024, is a hard
/// error (plan-22-B §4.3 step 2, resolved Open Decision 4).
pub(crate) fn normalize_source(source: Option<&Path>) -> Result<RgbaImage, String> {
    match source {
        Some(path) => {
            let decoded = image::open(path).map_err(|err| {
                format!("icon '{}' is not a decodable image: {err}", path.display())
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

/// Render the app icon at `size`×`size` as PNG bytes (plan-51-A §4.2).
///
/// Shares [`normalize_source`] with the macOS `.icns` path, so a project `icon`
/// that is accepted on one platform is accepted on both and an icon rejected on
/// one is rejected on both. The macOS squircle mask is deliberately **not**
/// applied: it encodes a Big Sur shaping convention, and Linux icon themes shape
/// icons themselves.
pub(crate) fn render_png(source: Option<&Path>, size: u32) -> Result<Vec<u8>, String> {
    let canvas = normalize_source(source)?;
    let scaled = image::imageops::resize(&canvas, size, size, FilterType::Lanczos3);
    let mut bytes = Vec::new();
    scaled
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|err| format!("failed to encode {size}×{size} icon PNG: {err}"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_png_emits_each_hicolor_size_exactly() {
        for size in HICOLOR_SIZES {
            let bytes = render_png(None, size).expect("render default icon");
            assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n", "{size}: PNG magic");
            let decoded = image::load_from_memory(&bytes).expect("re-decode PNG");
            assert_eq!(
                decoded.dimensions(),
                (size, size),
                "{size}×{size} decodes to its own size"
            );
        }
    }

    #[test]
    fn render_png_root_size_is_256() {
        let decoded =
            image::load_from_memory(&render_png(None, ROOT_ICON_SIZE).expect("render")).unwrap();
        assert_eq!(decoded.dimensions(), (256, 256));
    }

    #[test]
    fn render_png_rejects_non_1024_source() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("small.png");
        RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 255, 255]))
            .save(&path)
            .unwrap();
        let err = render_png(Some(&path), 256).expect_err("non-1024 icon must fail");
        assert!(err.contains("must be 1024×1024"), "unexpected error: {err}");
        assert!(err.contains("64×64"), "reports actual size: {err}");
    }

    #[test]
    fn render_png_rejects_non_image_source() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bogus.png");
        std::fs::write(&path, b"this is not a PNG").unwrap();
        let err = render_png(Some(&path), 256).expect_err("non-image icon must fail");
        assert!(
            err.contains("is not a decodable image"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn render_png_accepts_a_1024_source_and_keeps_its_pixels() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("big.png");
        RgbaImage::from_pixel(CANVAS, CANVAS, image::Rgba([10, 200, 30, 255]))
            .save(&path)
            .unwrap();
        let decoded =
            image::load_from_memory(&render_png(Some(&path), 128).expect("render")).unwrap();
        assert_eq!(decoded.dimensions(), (128, 128));
        // A uniform source downsamples to the same uniform color — no mask applied.
        assert_eq!(
            decoded.to_rgba8().get_pixel(4, 4),
            &image::Rgba([10, 200, 30, 255])
        );
    }

    #[test]
    fn normalize_source_default_is_the_embedded_canvas() {
        let canvas = normalize_source(None).expect("embedded default");
        assert_eq!(canvas.dimensions(), (CANVAS, CANVAS));
    }
}
