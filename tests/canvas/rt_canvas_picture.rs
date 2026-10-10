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
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", x := 100.0, y := 100.0, w := 32.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
             LET a AS canvas::DrawItem = canvas::Picture[id := \"\", x := 0.0, y := 0.0, w := 64.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Picture[id := \"\", x := 200.0, y := 0.0, w := 64.0, h := 32.0, image := clear, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
             LET a AS canvas::DrawItem = canvas::Picture[id := \"\", x := 0.0, y := 0.0, w := 16.0, h := 16.0, image := img, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n",
            pixels(&[(255, 255, 255, 255)])
        )),
    );
    assert_eq!(pixel(&frame, 8, 8), (0, 0, 255, 255), "white x blue = blue");

    let (frame, _) = render(
        "canvas_picture_opacity",
        &scene(&format!(
            "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n  \
             LET a AS canvas::DrawItem = canvas::Picture[id := \"\", x := 0.0, y := 0.0, w := 16.0, h := 16.0, image := img, paint := WITH canvas::fill(color::rgb(255, 255, 255)) {{ transparency := 0.5 }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
                 LET a AS canvas::DrawItem = canvas::Picture[id := \"\", x := 0.0, y := 0.0, w := 16.0, h := 16.0, image := img, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
        &scene("  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", x := 0.0, y := 0.0, w := 16.0, h := 16.0, paint := canvas::fill(color::rgba(200, 100, 50, 127)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([a])\n"),
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
             LET a AS canvas::DrawItem = canvas::Picture[id := \"\", x := 0.0, y := 0.0, w := 32.0, h := 32.0, image := g, paint := clipped, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET under AS canvas::DrawItem = canvas::Rectangle[id := \"\", x := 100.0, y := 0.0, w := 32.0, h := 32.0, paint := canvas::fill(color::rgb(0, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET mul AS canvas::Paint = WITH canvas::fill(color::rgb(255, 255, 255)) {{ blend := canvas::BlendMode.Multiply }}\n  \
             LET b AS canvas::DrawItem = canvas::Picture[id := \"\", x := 100.0, y := 0.0, w := 32.0, h := 32.0, image := y, paint := mul, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET t AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 400.0, ty := 100.0]\n  \
             LET rot AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
             LET c AS canvas::DrawItem = canvas::Picture[id := \"\", x := 0.0, y := 0.0, w := 200.0, h := 40.0, image := rb, paint := rot, transform := t]\n  \
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
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", x := 0.0, y := 0.0, w := 64.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"g\", [tile])\n  \
             LET node AS canvas::DrawItem = canvas::Group[name := \"g\", dx := 300.0, dy := 200.0]\n  \
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
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", x := 100.0, y := 100.0, w := 32.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET mark AS canvas::DrawItem = canvas::Rectangle[id := \"\", x := 300.0, y := 100.0, w := 32.0, h := 32.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", x := 100.0, y := 100.0, w := 32.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
         LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", x := 100.0, y := 100.0, w := 32.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", x := 100.0, y := 100.0, w := 32.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
         LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", x := 100.0, y := 100.0, w := 32.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
