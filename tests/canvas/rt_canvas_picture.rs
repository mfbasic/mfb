//! `canvas::Picture` draws its image (bug-484).
//!
//! Until bug-484 no renderer drew a `Picture`: `__canvas_headerFor`'s `Picture` arm
//! returned the empty `NONE` header, `__canvas_drawGeometry` returns immediately for
//! `NONE`, and a program that loaded an image and presented it got background pixels
//! with no diagnostic. `tests/cli/cli_canvas_image_resource.rs` presented one and
//! checked only exit markers, which is how the blank stayed invisible.
//!
//! These read pixels, on the software path — the oracle. Every expected value is
//! derived by hand from the documented conventions (Y-down, pixel centres at `+0.5`,
//! nearest sampling, coverage `clamp(0.5 - d, 0, 1)`), with every rectangle on whole
//! pixels so no assertion is a judgement call about an antialiased edge.
//!
//! The semantics pinned here, which bug-484 defines because plan-116-B/C scoped
//! `Picture` out citing it:
//!
//! * the image is scaled into `x, y, w, h` and sampled **nearest** — the plan-116-C
//!   §4.5 rule glyphs follow, for the same oracle-reproducibility reason;
//! * `Paint.fill` **tints** it: each channel is multiplied by the fill's, so the white
//!   fill every man example uses draws the image unchanged, and the fill's alpha is the
//!   picture's opacity;
//! * `Paint.clip`, `Paint.blend`, `Paint.transform` and a group's translation apply
//!   exactly as they do to a `Rectangle` of the same bounds.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const WIDTH: usize = 900;
const HEIGHT: usize = 640;
const BLACK: (u8, u8, u8, u8) = (0, 0, 0, 255);
const GREEN: (u8, u8, u8, u8) = (0, 255, 0, 255);

/// Build a `--debug` app, run it headless and synchronous, and return the last dumped
/// frame and the `MFB_CANVAS_STATS` lines.
fn render(name: &str, source: &str) -> (Vec<u8>, Vec<String>) {
    let project = common::temp_project(name, source);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        stdout,
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(
        stdout.contains("rendered"),
        "the program did not reach its end marker:\n{stdout}"
    );
    let pixels = std::fs::read(&frame)
        .unwrap_or_else(|e| panic!("canvas dump {} not written: {e}\n{stdout}", frame.display()));
    assert_eq!(
        pixels.len(),
        WIDTH * HEIGHT * 4,
        "dump is not a {WIDTH}x{HEIGHT} RGBA frame"
    );
    let lines = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    (pixels, lines)
}

fn pixel(frame: &[u8], x: usize, y: usize) -> (u8, u8, u8, u8) {
    let i = (y * WIDTH + x) * 4;
    (frame[i], frame[i + 1], frame[i + 2], frame[i + 3])
}

fn scene(body: &str) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT collections\nIMPORT color\nIMPORT io\n\nFUNC main() AS Nothing\n  \
         app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND FUNC\n"
    )
}

/// A byte-list literal for RGBA pixels, one `(r, g, b, a)` per pixel.
fn pixels(px: &[(u8, u8, u8, u8)]) -> String {
    let bytes: Vec<String> = px
        .iter()
        .flat_map(|&(r, g, b, a)| [r, g, b, a])
        .map(|v| format!("toByte({v})"))
        .collect();
    format!("[{}]", bytes.join(", "))
}

/// The bug document's reproduction, verbatim in substance: a 1×1 opaque green image
/// scaled across a 32×32 rectangle at (100, 100).
///
/// Before the fix every pixel of the frame was background — the rectangle was
/// untouched, not drawn wrong.
#[test]
fn a_picture_draws_its_image_into_its_rectangle() {
    let (frame, _) = render(
        "canvas_picture_basic",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             canvas::present([tile])\n",
            pixels(&[(0, 255, 0, 255)])
        )),
    );
    assert_eq!(pixel(&frame, 116, 116), GREEN, "the middle of the picture");
    assert_eq!(pixel(&frame, 100, 100), GREEN, "its top-left pixel");
    assert_eq!(pixel(&frame, 131, 131), GREEN, "its bottom-right pixel");
    assert_eq!(pixel(&frame, 99, 116), BLACK, "one pixel left of it");
    assert_eq!(pixel(&frame, 132, 116), BLACK, "one pixel right of it");
    assert_eq!(pixel(&frame, 116, 132), BLACK, "one pixel below it");
    let drawn = frame.chunks(4).filter(|p| *p != [0, 0, 0, 255]).count();
    assert_eq!(drawn, 32 * 32, "exactly the 32x32 destination is painted");
}

/// Scaling samples the nearest texel: a 2×1 red|blue image across 64×32 puts the
/// seam exactly at x = 32, and a transparent texel lets the background through.
///
/// Pixel 31's centre is 31.5, which maps to `31.5 * 2 / 64 = 0.98` → texel 0; pixel
/// 32's is 32.5 → 1.02 → texel 1. A bilinear sampler would blend both into purple.
#[test]
fn scaling_samples_the_nearest_texel() {
    let (frame, _) = render(
        "canvas_picture_nearest",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(2, 1, {})\n  \
             RES clear AS canvas::Image = canvas::createImage(2, 1, {})\n  \
             LET a AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 64.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             LET b AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 200.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 64.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := clear]\n  \
             canvas::present([a, b])\n",
            pixels(&[(255, 0, 0, 255), (0, 0, 255, 255)]),
            pixels(&[(0, 255, 0, 255), (255, 255, 255, 0)])
        )),
    );
    assert_eq!(
        pixel(&frame, 0, 0),
        (255, 0, 0, 255),
        "left texel, first column"
    );
    assert_eq!(
        pixel(&frame, 31, 16),
        (255, 0, 0, 255),
        "left texel, last column"
    );
    assert_eq!(
        pixel(&frame, 32, 16),
        (0, 0, 255, 255),
        "right texel, first column"
    );
    assert_eq!(
        pixel(&frame, 63, 31),
        (0, 0, 255, 255),
        "right texel, last column"
    );
    assert_eq!(
        pixel(&frame, 210, 16),
        GREEN,
        "the opaque half of the second image"
    );
    assert_eq!(
        pixel(&frame, 240, 16),
        BLACK,
        "a texel with alpha 0 draws nothing over the background"
    );
}

/// `Paint.fill` tints the image channel by channel; a white image under a blue fill
/// is blue, and the fill's alpha is the picture's opacity.
///
/// Half alpha over black is not a whole-number check through the sRGB-linear blend,
/// so the opacity half asserts the ordering (strictly between background and full)
/// rather than a derived byte.
#[test]
fn the_fill_tints_the_image() {
    let (frame, _) = render(
        "canvas_picture_tint",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
             LET a AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 16.0, height := 16.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             canvas::present([a])\n",
            pixels(&[(255, 255, 255, 255)])
        )),
    );
    assert_eq!(pixel(&frame, 8, 8), (0, 0, 255, 255), "white x blue = blue");

    let (frame, _) = render(
        "canvas_picture_opacity",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
             LET a AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 16.0, height := 16.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) {{ transparency := 0.5 }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             canvas::present([a])\n",
            pixels(&[(0, 255, 0, 255)])
        )),
    );
    let (r, g, b, a) = pixel(&frame, 8, 8);
    assert_eq!((r, b, a), (0, 0, 255), "only green is contributed");
    assert!(
        g > 0 && g < 255,
        "half opacity lands strictly between: got {g}"
    );
}

/// plan-163-C: a picture's fill is a TINT whose alpha is its strength, and its opacity
/// comes from `transparency` alone.
///
/// One texel (200, 100, 50, 255), drawn opaque over black, so each drawn pixel is the
/// computed texel itself. `tint = texel * fill / 255` (truncated), then the texel moves
/// towards it by `fill.a / 255`, the difference rounded half away from zero:
///
/// * the zero `Paint` (fill alpha 0): the texel unchanged, opaque — (200, 100, 50);
/// * white at alpha 255: `tint = texel`, unchanged;
/// * red at alpha 255: the multiply, (200, 0, 0);
/// * red at alpha 128: green `100 + round(-100 * 128 / 255) = 100 - 50 = 50`, blue
///   `50 + round(-50 * 128 / 255) = 50 - 25 = 25`, so (200, 50, 25);
/// * white with `transparency := 0.5`: the texel at alpha `(255 * 127 + 127) / 255 =
///   127`, which draws exactly as a rectangle filled `rgba(200, 100, 50, 127)`.
#[test]
fn the_fill_is_a_tint_and_transparency_is_the_opacity() {
    let texel = pixels(&[(200, 100, 50, 255)]);
    let draw = |name: &str, paint: &str| {
        render(
            name,
            &scene(&format!(
                "  RES img AS canvas::Image = canvas::createImage(1, 1, {texel})\n  \
                 LET zero AS canvas::Paint = canvas::Paint[fill := color::rgba(0, 0, 0, 0), stroke := color::rgba(0, 0, 0, 0), strokeWidth := 0.0, blend := canvas::BlendMode.Normal, clip := [], clipRule := canvas::FillRule.EvenOdd, gradient := canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 0.0, y := 0.0], stops := [], spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly], cap := canvas::CapStyle.Butt, join := canvas::JoinStyle.Round, miterLimit := 0.0, dashes := [], dashOffset := 0.0, transparency := 0.0]\n  \
                 LET a AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 16.0, height := 16.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
                 canvas::present([a])\n"
            )),
        )
        .0
    };
    assert_eq!(
        pixel(&draw("canvas_tint_zero", "zero"), 8, 8),
        (200, 100, 50, 255),
        "the zero Paint draws the image unchanged and opaque"
    );
    assert_eq!(
        pixel(
            &draw(
                "canvas_tint_white",
                "canvas::fill(color::rgb(255, 255, 255))"
            ),
            8,
            8
        ),
        (200, 100, 50, 255),
        "a white tint changes nothing"
    );
    assert_eq!(
        pixel(
            &draw("canvas_tint_red", "canvas::fill(color::rgb(255, 0, 0))"),
            8,
            8
        ),
        (200, 0, 0, 255),
        "an opaque red tint is the multiply"
    );
    assert_eq!(
        pixel(
            &draw(
                "canvas_tint_half",
                "canvas::fill(color::rgba(255, 0, 0, 128))"
            ),
            8,
            8
        ),
        (200, 50, 25, 255),
        "a half-strength red tint goes halfway"
    );
    let faded = draw(
        "canvas_tint_faded",
        "WITH canvas::fill(color::rgb(255, 255, 255)) { transparency := 0.5 }",
    );
    let spelled = render(
        "canvas_tint_spelled",
        &scene("  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 16.0, height := 16.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(200, 100, 50, 127)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([a])\n"),
    )
    .0;
    assert_eq!(
        pixel(&faded, 8, 8),
        pixel(&spelled, 8, 8),
        "transparency 0.5 fades the texel to alpha 127"
    );
}

/// A picture honours `Paint.clip`, `Paint.blend` and `Paint.transform` exactly as a
/// rectangle does — plan-116-B/C scoped `Picture` out, and this is where it is scoped
/// back in.
#[test]
fn clip_blend_and_transform_apply_to_a_picture() {
    let green = pixels(&[(0, 255, 0, 255)]);
    let yellow = pixels(&[(255, 255, 0, 255)]);
    let red_blue = pixels(&[(255, 0, 0, 255), (0, 0, 255, 255)]);
    let (frame, _) = render(
        "canvas_picture_paint",
        &scene(&format!(
            "  RES g AS canvas::Image = canvas::createImage(1, 1, {green})\n  \
             RES y AS canvas::Image = canvas::createImage(1, 1, {yellow})\n  \
             RES rb AS canvas::Image = canvas::createImage(2, 1, {red_blue})\n  \
             LET clipped AS canvas::Paint = WITH canvas::fill(color::rgb(255, 255, 255)) {{ clip := [[vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 16.0, y := 0.0], vector::Float2[x := 16.0, y := 640.0], vector::Float2[x := 0.0, y := 640.0]]] }}\n  \
             LET a AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := clipped, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := g]\n  \
             LET under AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET mul AS canvas::Paint = WITH canvas::fill(color::rgb(255, 255, 255)) {{ blend := canvas::BlendMode.Multiply }}\n  \
             LET b AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := mul, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := y]\n  \
             LET t AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 400.0, ty := 100.0]\n  \
             LET rot AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
             LET c AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := rot, transform := t], clip := canvas::PictureClip.ClipAndTransform, fit := canvas::PictureFit.Stretch, image := rb]\n  \
             canvas::present([a, under, b, c])\n"
        )),
    );
    // Clip: x 0..16 of a 0..32 picture.
    assert_eq!(pixel(&frame, 8, 16), GREEN, "inside the clip");
    assert_eq!(
        pixel(&frame, 24, 16),
        BLACK,
        "outside the clip, inside the picture"
    );
    // Multiply: yellow x cyan = green, and 0 or 255 in every channel, so exact.
    assert_eq!(pixel(&frame, 116, 16), GREEN, "yellow multiplied over cyan");
    // Transform: `x' = -y + 400`, `y' = x + 100`, so shape x 0..100 (red texel) lands
    // at surface y 100..200 and x 100..200 (blue texel) at y 200..300, all at x 360..400.
    assert_eq!(
        pixel(&frame, 380, 150),
        (255, 0, 0, 255),
        "the rotated left texel"
    );
    assert_eq!(
        pixel(&frame, 380, 250),
        (0, 0, 255, 255),
        "the rotated right texel"
    );
    assert_eq!(
        pixel(&frame, 100, 20),
        (0, 255, 0, 255),
        "the untransformed spot holds only the multiplied tile"
    );
    assert_eq!(
        pixel(&frame, 420, 200),
        BLACK,
        "just outside the rotated picture"
    );
}

/// A picture inside a translated group lands where the group puts it, with the right
/// texels — the group offset moves the sample point as well as the bounds.
#[test]
fn a_picture_in_a_group_moves_with_it() {
    let (frame, _) = render(
        "canvas_picture_group",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(2, 1, {})\n  \
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 64.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             canvas::setGroup(\"g\", [tile])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 300.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]\n  \
             canvas::present([node])\n",
            pixels(&[(255, 0, 0, 255), (0, 0, 255, 255)])
        )),
    );
    assert_eq!(
        pixel(&frame, 310, 216),
        (255, 0, 0, 255),
        "left texel, moved"
    );
    assert_eq!(
        pixel(&frame, 350, 216),
        (0, 0, 255, 255),
        "right texel, moved"
    );
    assert_eq!(
        pixel(&frame, 10, 16),
        BLACK,
        "nothing at the untranslated position"
    );
}

/// A scene built before its image is destroyed still presents, and that item draws
/// nothing while the rest of the frame renders (`.ai/canvas-threading.md` §7).
#[test]
fn a_destroyed_image_draws_nothing() {
    let (frame, _) = render(
        "canvas_picture_destroyed",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             LET mark AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 300.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::destroyImage(img)\n  \
             canvas::present([tile, mark])\n",
            pixels(&[(0, 255, 0, 255)])
        )),
    );
    assert_eq!(
        pixel(&frame, 116, 116),
        BLACK,
        "the destroyed image's item draws nothing"
    );
    assert_eq!(
        pixel(&frame, 316, 116),
        (255, 0, 0, 255),
        "the frame around it renders"
    );
}

/// `setBytes` on an image the live scene draws repaints with the new pixels, with no
/// `present` — `mfb spec app canvas` "Image content is orthogonal to the scene", and
/// redraw trigger 5 of `.ai/canvas-threading.md` §4.
///
/// Before bug-484 there was nothing to repaint, and `setBytes` signalled no redraw at
/// all; re-presenting the unchanged scene would not have helped either, because the
/// frame skip refuses an identical scene.
#[test]
fn set_bytes_on_a_presented_image_repaints() {
    let (frame, stats) = render(
        "canvas_picture_set_bytes",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             canvas::present([tile])\n  \
             canvas::setBytes(img, {})\n",
            pixels(&[(255, 0, 0, 255)]),
            pixels(&[(0, 255, 0, 255)])
        )),
    );
    assert_eq!(
        pixel(&frame, 116, 116),
        GREEN,
        "the last frame shows the new pixels"
    );
    let last = stats.last().cloned().unwrap_or_default();
    assert!(
        last.contains("frames=2"),
        "the present and the setBytes each rendered a frame: {stats:?}"
    );
}

/// The same with `MFB_CANVAS_DAMAGE` on: a partial redraw must see the picture as
/// changed, although no item in the scene is different.
#[test]
fn set_bytes_repaints_under_damage_tracking() {
    let source = scene(&format!(
        "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
         LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
         canvas::present([tile])\n  \
         canvas::setBytes(img, {})\n",
        pixels(&[(255, 0, 0, 255)]),
        pixels(&[(0, 255, 0, 255)])
    ));
    let project = common::temp_project("canvas_picture_damage", &source);
    let frame = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, "canvas_picture_damage");
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_DAMAGE", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let pixels = std::fs::read(&frame).expect("dump");
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(
        pixel(&pixels, 116, 116),
        GREEN,
        "damage tracking saw the new pixels"
    );
}

/// `setBytes` on an image NO scene draws repaints nothing (race-matrix row R10):
/// mutating an off-screen buffer must not turn into a frame.
#[test]
fn set_bytes_on_an_undrawn_image_does_not_repaint() {
    let (frame, stats) = render(
        "canvas_picture_offscreen",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
             RES other AS canvas::Image = canvas::createImage(1, 1, {})\n  \
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             canvas::present([tile])\n  \
             canvas::setBytes(other, {})\n",
            pixels(&[(0, 255, 0, 255)]),
            pixels(&[(255, 0, 0, 255)]),
            pixels(&[(0, 0, 255, 255)])
        )),
    );
    assert_eq!(pixel(&frame, 116, 116), GREEN);
    assert_eq!(
        stats.len(),
        1,
        "one frame, from the present only: {stats:?}"
    );
}

/// Two `setBytes` with no frame between them: the frame that follows shows the LAST
/// (race-matrix row R9).
///
/// Not synchronous, so the two swaps can land before the graphics thread wakes; each
/// frame reads whichever pixel block is current when it is built, so an intermediate
/// value is at most drawn and then replaced, never the one left on screen.
#[test]
fn set_bytes_twice_between_frames_shows_the_last() {
    let source = scene(&format!(
        "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
         LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 32.0, height := 32.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
         canvas::present([tile])\n  \
         canvas::setBytes(img, {})\n  \
         canvas::setBytes(img, {})\n  \
         os::sleep(1500)\n",
        pixels(&[(0, 0, 255, 255)]),
        pixels(&[(255, 0, 0, 255)]),
        pixels(&[(0, 255, 0, 255)])
    ))
    .replace("IMPORT io", "IMPORT io\nIMPORT os");
    let project = common::temp_project("canvas_picture_r9", &source);
    let frame = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, "canvas_picture_r9");
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let pixels = std::fs::read(&frame).expect("dump");
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(pixel(&pixels, 116, 116), GREEN, "the last setBytes wins");
}

/// The text of a `canvas::Picture` on the shape bound to `shape`.
fn shaped(shape: &str, clip: &str, fit: &str, image: &str) -> String {
    format!(
        "canvas::Picture[id := \"\", shape := {shape}, clip := canvas::PictureClip.{clip}, \
         fit := canvas::PictureFit.{fit}, image := {image}]"
    )
}

const RED: (u8, u8, u8, u8) = (255, 0, 0, 255);
const BLUE: (u8, u8, u8, u8) = (0, 0, 255, 255);

/// plan-172-L: the image is visible only inside the shape, and the shape's stroke
/// outlines it on top.
///
/// A 1×1 green image in a circle of radius 50 about (200, 200), no stroke: the centre
/// is the image; (240, 240), centre 57.3 px out, is inside the circle's box and
/// outside the circle; (252, 200), 52.5 px out, is just past it.
///
/// The same about (400, 200) with a 6 px red stroke, which covers 47 to 53 px from the
/// centre: (448, 200), centre 48.5 px out, is INSIDE the circle and is the stroke's —
/// the outline is drawn over the image — and so is (450, 200), half a pixel outside;
/// (445, 200), 45.5 px out, is still the image, and (455, 200), 55.5 px out, is past
/// the stroke.
///
/// A picture on a `Line` has no inside and draws nothing: (200, 400), on the line, is
/// background, as is the whole frame's count of the stroke colour it would have had.
#[test]
fn a_shaped_picture_is_its_image_inside_the_shape_and_its_stroke_on_top() {
    let (frame, _) = render(
        "canvas_picture_shaped",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {green})\n  \
             MUT bare AS canvas::Circle\n  \
             bare = WITH bare {{ position := vector::Float2[x := 200.0, y := 200.0], radius := 50.0, paint := canvas::fill(color::rgb(255, 255, 255)) }}\n  \
             LET a AS canvas::DrawItem = {a}\n  \
             MUT ringed AS canvas::Circle\n  \
             ringed = WITH ringed {{ position := vector::Float2[x := 400.0, y := 200.0], radius := 50.0, paint := canvas::fillStroke(color::rgb(255, 255, 255), color::rgb(255, 0, 0), 6.0) }}\n  \
             LET b AS canvas::DrawItem = {b}\n  \
             MUT rule AS canvas::Line\n  \
             rule = WITH rule {{ startAt := vector::Float2[x := 100.0, y := 400.5], endAt := vector::Float2[x := 300.0, y := 400.5], paint := canvas::stroke(color::rgb(0, 0, 255), 20.0) }}\n  \
             LET c AS canvas::DrawItem = {c}\n  \
             canvas::present([a, b, c])\n",
            green = pixels(&[GREEN]),
            a = shaped("bare", "ClipOnly", "Stretch", "img"),
            b = shaped("ringed", "ClipOnly", "Stretch", "img"),
            c = shaped("rule", "ClipOnly", "Stretch", "img"),
        )),
    );
    assert_eq!(pixel(&frame, 200, 200), GREEN, "the circle's centre");
    assert_eq!(
        pixel(&frame, 240, 240),
        BLACK,
        "in the box, outside the circle"
    );
    assert_eq!(pixel(&frame, 252, 200), BLACK, "just past the circle");

    assert_eq!(pixel(&frame, 400, 200), GREEN);
    assert_eq!(pixel(&frame, 445, 200), GREEN, "inside the stroke");
    assert_eq!(pixel(&frame, 448, 200), RED, "the stroke, over the image");
    assert_eq!(
        pixel(&frame, 450, 200),
        RED,
        "the stroke, outside the circle"
    );
    assert_eq!(pixel(&frame, 455, 200), BLACK, "past the stroke");

    assert_eq!(pixel(&frame, 200, 400), BLACK, "a picture on a line");
    assert_eq!(
        frame
            .chunks(4)
            .filter(|p| p[2] == 255 && p[0] == 0 && p[1] == 0)
            .count(),
        0,
        "the line's own stroke is no part of a picture that draws nothing"
    );
}

/// `Contain` and `Cover`, each with a 2×1 image — red on the left, blue on the right —
/// in a 100 by 100 rectangle.
///
/// `Contain` at (500, 100): the image is 100 by 50, centred, so it covers rows 125 to
/// 175. (510, 150) is its red half and (590, 150) its blue; (550, 110), inside the
/// shape and above the image, is background.
///
/// `Cover` at (650, 300): the image is 200 by 100, centred on the rectangle, so it
/// would run from x = 600 to 800 and its halves meet at x = 700 — the rectangle's
/// middle. (660, 350) is red and (740, 350) blue; (640, 350), where the image would
/// be were it not cut to the shape, is background.
#[test]
fn contain_fits_inside_the_shape_and_cover_is_cut_to_it() {
    let (frame, _) = render(
        "canvas_picture_fit",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(2, 1, {halves})\n  \
             MUT box AS canvas::Rectangle\n  \
             box = WITH box {{ position := vector::Float2[x := 500.0, y := 100.0], size := canvas::Size[width := 100.0, height := 100.0], paint := canvas::fill(color::rgb(255, 255, 255)) }}\n  \
             LET inside AS canvas::DrawItem = {contain}\n  \
             LET over AS canvas::DrawItem = {cover}\n  \
             canvas::present([inside, over])\n",
            halves = pixels(&[RED, BLUE]),
            contain = shaped("box", "ClipOnly", "Contain", "img"),
            cover = shaped(
                "WITH box { position := vector::Float2[x := 650.0, y := 300.0] }",
                "ClipOnly",
                "Cover",
                "img"
            ),
        )),
    );
    assert_eq!(pixel(&frame, 510, 150), RED);
    assert_eq!(pixel(&frame, 590, 150), BLUE);
    assert_eq!(
        pixel(&frame, 550, 110),
        BLACK,
        "inside the shape, above the image"
    );
    assert_eq!(
        pixel(&frame, 550, 190),
        BLACK,
        "inside the shape, below the image"
    );
    assert_eq!(pixel(&frame, 540, 126), RED, "the image's first whole row");

    assert_eq!(pixel(&frame, 660, 350), RED);
    assert_eq!(pixel(&frame, 740, 350), BLUE);
    assert_eq!(pixel(&frame, 640, 350), BLACK, "the overflow is cut off");
    assert_eq!(pixel(&frame, 760, 350), BLACK, "on the other side too");
    assert_eq!(
        pixel(&frame, 700, 310),
        BLUE,
        "the halves meet at the middle"
    );
    assert_eq!(pixel(&frame, 699, 310), RED);
}

/// `ClipOnly` leaves the image upright under the shape's transform; `ClipAndTransform`
/// turns it with the shape.
///
/// A 2×1 image, red left and blue right, in a 100 by 40 rectangle bound at its centre
/// and turned a quarter turn, so it stands 40 wide and 100 tall.
///
/// Under `ClipOnly` about (200, 300) the image is fitted to the box around the turned
/// shape — x 180 to 220, y 250 to 350 — and is not turned: red is still on the LEFT.
/// (190, 300) is red and (210, 300) blue, top to bottom.
///
/// Under `ClipAndTransform` about (400, 300) the image turns with the shape: its left
/// half is now the top. (400, 270) is red and (400, 330) blue, across the whole width.
#[test]
fn clip_only_keeps_the_image_upright_and_clip_and_transform_turns_it() {
    let (frame, _) = render(
        "canvas_picture_clipmode",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(2, 1, {halves})\n  \
             MUT card AS canvas::Rectangle\n  \
             card = WITH card {{ position := vector::Float2[x := 200.0, y := 300.0], bind := canvas::BindTo.Center, size := canvas::Size[width := 100.0, height := 40.0], paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 1.0, 0.0 - 1.0, 0.0, 0.0, 0.0] }}\n  \
             LET upright AS canvas::DrawItem = {upright}\n  \
             LET turned AS canvas::DrawItem = {turned}\n  \
             canvas::present([upright, turned])\n",
            halves = pixels(&[RED, BLUE]),
            upright = shaped("card", "ClipOnly", "Stretch", "img"),
            turned = shaped(
                "WITH card { position := vector::Float2[x := 400.0, y := 300.0] }",
                "ClipAndTransform",
                "Stretch",
                "img"
            ),
        )),
    );
    for y in [260, 300, 340] {
        assert_eq!(pixel(&frame, 190, y), RED, "upright, left, row {y}");
        assert_eq!(pixel(&frame, 210, y), BLUE, "upright, right, row {y}");
    }
    assert_eq!(pixel(&frame, 170, 300), BLACK, "outside the turned shape");
    for x in [385, 400, 415] {
        assert_eq!(pixel(&frame, x, 270), RED, "turned, top, column {x}");
        assert_eq!(pixel(&frame, x, 330), BLUE, "turned, bottom, column {x}");
    }
    assert_eq!(pixel(&frame, 370, 300), BLACK, "outside the turned shape");
}

/// `canvas::setBytes` on the image of a SHAPED picture repaints it: the picture's
/// record is rebuilt from the new pixels although the item did not change, exactly as
/// a plain picture's is.
#[test]
fn set_bytes_repaints_a_shaped_picture() {
    let (frame, stats) = render(
        "canvas_picture_shaped_set_bytes",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {red})\n  \
             MUT disc AS canvas::Circle\n  \
             disc = WITH disc {{ position := vector::Float2[x := 200.0, y := 200.0], radius := 50.0, paint := canvas::fill(color::rgb(255, 255, 255)) }}\n  \
             LET a AS canvas::DrawItem = {a}\n  \
             canvas::present([a])\n  \
             canvas::setBytes(img, {green})\n",
            red = pixels(&[RED]),
            green = pixels(&[GREEN]),
            a = shaped("disc", "ClipOnly", "Stretch", "img"),
        )),
    );
    assert_eq!(
        pixel(&frame, 200, 200),
        GREEN,
        "the last frame shows the new pixels"
    );
    assert_eq!(
        pixel(&frame, 240, 240),
        BLACK,
        "and is still cut to the circle"
    );
    let last = stats.last().cloned().unwrap_or_default();
    assert!(
        last.contains("frames=2"),
        "the present and the setBytes each rendered a frame: {stats:?}"
    );
}

/// A shaped picture whose paint has a `clip` shows the image where the shape AND the
/// clip both cover, and nowhere else.
///
/// The picture is a 4×4 image in a circle of radius 50 about (200, 200), clipped to the
/// half-plane right of its centre: the clip is a rectangle from (0, −100) to (100, 100)
/// in the circle's own coordinates. The same picture without the clip is drawn about
/// (500, 200), 300 px to the right.
///
/// * (230, 200), inside both, is the image — the pixel the unclipped picture has 300
///   px on.
/// * (170, 200), inside the circle alone, is background.
/// * (252, 200), inside the clip alone, is background.
/// * No seam: the two clips are intersected into one outline, band by band, and a
///   boundary left between two bands would dip coverage along a horizontal line inside
///   the region. Every pixel inside the intersection and more than 2 px from its
///   boundary — more than 2 px right of x = 200, less than 48 px from the centre — is
///   compared with the unclipped picture's, all of them, and none may differ.
#[test]
fn a_shaped_picture_with_a_clip_shows_only_where_both_cover() {
    let mut texels = Vec::new();
    for y in 0..4u8 {
        for x in 0..4u8 {
            texels.push((
                40 + x * 60,
                40 + y * 60,
                if (x + y) % 2 == 0 { 220 } else { 0 },
                255,
            ));
        }
    }
    let (frame, _) = render(
        "canvas_picture_intersect",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(4, 4, {texels})\n  \
             MUT whole AS canvas::Circle\n  \
             whole = WITH whole {{ position := vector::Float2[x := 500.0, y := 200.0], radius := 50.0, paint := canvas::fill(color::rgb(255, 255, 255)) }}\n  \
             LET half AS List OF List OF vector::Float2 = [[vector::Float2[x := 0.0, y := 0.0 - 100.0], vector::Float2[x := 100.0, y := 0.0 - 100.0], vector::Float2[x := 100.0, y := 100.0], vector::Float2[x := 0.0, y := 100.0]]]\n  \
             LET cut AS canvas::Circle = WITH whole {{ position := vector::Float2[x := 200.0, y := 200.0], paint := WITH whole.paint {{ clip := half }} }}\n  \
             LET a AS canvas::DrawItem = {a}\n  \
             LET b AS canvas::DrawItem = {b}\n  \
             canvas::present([a, b])\n",
            texels = pixels(&texels),
            a = shaped("cut", "ClipOnly", "Stretch", "img"),
            b = shaped("whole", "ClipOnly", "Stretch", "img"),
        )),
    );
    assert_eq!(pixel(&frame, 530, 200).3, 255);
    assert_ne!(
        pixel(&frame, 530, 200),
        BLACK,
        "the unclipped picture is drawn"
    );
    assert_eq!(
        pixel(&frame, 230, 200),
        pixel(&frame, 530, 200),
        "inside both"
    );
    assert_eq!(pixel(&frame, 170, 200), BLACK, "inside the circle alone");
    assert_ne!(
        pixel(&frame, 470, 200),
        BLACK,
        "which the unclipped picture does cover"
    );
    assert_eq!(pixel(&frame, 252, 200), BLACK, "inside the clip alone");

    let mut compared = 0;
    for y in 150..250usize {
        for x in 203..250usize {
            let (dx, dy) = (x as f64 + 0.5 - 200.0, y as f64 + 0.5 - 200.0);
            if (dx * dx + dy * dy).sqrt() < 48.0 {
                compared += 1;
                assert_eq!(
                    pixel(&frame, x, y),
                    pixel(&frame, x + 300, y),
                    "a seam at ({x}, {y})"
                );
            }
        }
    }
    assert!(compared > 3000, "only {compared} pixels compared");
}
