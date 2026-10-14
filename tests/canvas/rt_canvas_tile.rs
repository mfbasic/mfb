//! `canvas::Tile` software-renderer oracle (plan-173).

#[path = "../common/mod.rs"]
mod common;

use common::canvas_image::{compare_exact, Frame};
use std::path::Path;
use std::process::Command;

const WIDTH: usize = 900;
const HEIGHT: usize = 640;
const BLACK: (u8, u8, u8, u8) = (0, 0, 0, 255);

fn pixels(px: &[(u8, u8, u8, u8)]) -> String {
    let bytes: Vec<String> = px
        .iter()
        .flat_map(|&(r, g, b, a)| [r, g, b, a])
        .map(|v| format!("toByte({v})"))
        .collect();
    format!("[{}]", bytes.join(", "))
}

fn source(body: &str) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT io\n\nFUNC main() AS Nothing\n  \
         app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND FUNC\n"
    )
}

fn tile(
    name: &str,
    image: &str,
    sx: f64,
    sy: f64,
    sw: f64,
    sh: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> String {
    format!(
        "  LET {name} AS canvas::DrawItem = canvas::Tile[image := {image}, srcPosition := vector::Float2[x := {sx:?}, y := {sy:?}], srcSize := vector::Float2[x := {sw:?}, y := {sh:?}], position := vector::Float2[x := {x:?}, y := {y:?}], size := vector::Float2[x := {w:?}, y := {h:?}]]\n"
    )
}

fn render(name: &str, source: &str) -> (Frame, String) {
    render_with(name, source, false)
}

/// `verify` runs the `--debug` build with `MFB_CANVAS_GEO_VERIFY=1`, which re-lays every
/// flat frame's draw list out with the MFBASIC walk and counts any difference from the
/// native `canvas::sceneDrawsFlat` in `geoVerifyMismatches=`.
fn render_with(name: &str, source: &str, verify: bool) -> (Frame, String) {
    let project = common::temp_project(name, source);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let mut command = Command::new(&binary);
    if verify {
        command.env("MFB_CANVAS_GEO_VERIFY", "1");
    }
    let run = command
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let bytes = std::fs::read(&frame).expect("canvas dump written");
    let stats = std::fs::read_to_string(&stats).expect("canvas stats written");
    let _ = std::fs::remove_dir_all(&project);
    (Frame::from_rgba(WIDTH as u32, HEIGHT as u32, bytes), stats)
}

fn pixel(frame: &Frame, x: usize, y: usize) -> (u8, u8, u8, u8) {
    let i = (y * WIDTH + x) * 4;
    let bytes = &frame.pixels;
    (bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3])
}

#[test]
fn atlas_regions_scale_and_out_of_bounds_samples_are_exact() {
    let atlas = pixels(&[
        (255, 0, 0, 255),
        (0, 255, 0, 255),
        (0, 0, 255, 255),
        (255, 255, 0, 255),
    ]);
    let mut body = format!("  RES img AS canvas::Image = canvas::createImage(4, 1, {atlas})\n");
    body.push_str(&tile(
        "inside", "img", 1.0, 0.0, 2.0, 1.0, 0.0, 0.0, 8.0, 4.0,
    ));
    body.push_str(&tile(
        "partial", "img", -1.0, 0.0, 2.0, 1.0, 20.0, 0.0, 8.0, 4.0,
    ));
    body.push_str(&tile(
        "outside", "img", 5.0, 0.0, 1.0, 1.0, 40.0, 0.0, 4.0, 4.0,
    ));
    body.push_str(&tile(
        "fractional",
        "img",
        0.5,
        0.0,
        2.0,
        1.0,
        0.0,
        10.0,
        8.0,
        4.0,
    ));
    body.push_str(&tile(
        "zero", "img", 0.0, 0.0, 1.0, 1.0, 60.0, 0.0, 0.0, 4.0,
    ));
    body.push_str("  canvas::present([inside, partial, outside, fractional, zero])\n");
    let (frame, _) = render("canvas_tile_atlas", &source(&body));

    assert_eq!(pixel(&frame, 0, 1), (0, 255, 0, 255));
    assert_eq!(pixel(&frame, 3, 1), (0, 255, 0, 255));
    assert_eq!(pixel(&frame, 4, 1), (0, 0, 255, 255));
    assert_eq!(pixel(&frame, 7, 1), (0, 0, 255, 255));
    assert_eq!(
        pixel(&frame, 21, 1),
        BLACK,
        "out-of-image half stays transparent"
    );
    assert_eq!(pixel(&frame, 25, 1), (255, 0, 0, 255));
    assert_eq!(pixel(&frame, 41, 1), BLACK, "wholly outside draws nothing");
    assert_eq!(pixel(&frame, 1, 11), (255, 0, 0, 255));
    assert_eq!(pixel(&frame, 3, 11), (0, 255, 0, 255));
    assert_eq!(pixel(&frame, 7, 11), (0, 0, 255, 255));
    assert_eq!(
        pixel(&frame, 60, 1),
        BLACK,
        "zero destination draws nothing"
    );

    let reference = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/canvas/tile.png");
    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        frame.save_png(&reference);
        panic!(
            "generated new Tile reference {}; rerun without MFB_UPDATE_CANVAS_GOLDEN",
            reference.display()
        );
    }
    assert!(
        reference.exists(),
        "missing new Tile reference {}",
        reference.display()
    );
    if let Err(diff) = compare_exact(&frame, &Frame::load_png(&reference)) {
        panic!("Tile software output differs from its proved oracle: {diff}");
    }
}

#[test]
fn closed_image_and_nonpositive_source_draw_nothing() {
    let mut body = format!(
        "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n",
        pixels(&[(255, 0, 0, 255)])
    );
    body.push_str(&tile(
        "closed", "img", 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 4.0, 4.0,
    ));
    body.push_str(&tile(
        "sourceZero",
        "img",
        0.0,
        0.0,
        0.0,
        1.0,
        10.0,
        0.0,
        4.0,
        4.0,
    ));
    body.push_str("  canvas::destroyImage(img)\n  canvas::present([closed, sourceZero])\n");
    let (frame, _) = render("canvas_tile_empty", &source(&body));
    assert_eq!(pixel(&frame, 1, 1), BLACK);
    assert_eq!(pixel(&frame, 11, 1), BLACK);
}

/// The `key=value` field `key` of one stats line.
fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no `{key}=` in stats line: {line}"))
}

/// The `base:count` of each entry of a stats line's `draws=` list.
fn draw_runs(line: &str) -> Vec<String> {
    match field(line, "draws") {
        "none" => Vec::new(),
        list => list
            .split('|')
            .map(|entry| entry.split(':').take(2).collect::<Vec<_>>().join(":"))
            .collect(),
    }
}

fn red_image() -> String {
    format!(
        "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n",
        pixels(&[(255, 0, 0, 255)])
    )
}

fn group(name: &str, x: f64, y: f64, t: [f64; 4]) -> String {
    format!(
        "  LET {name} AS canvas::DrawItem = canvas::Group[id := \"{name}\", position := vector::Float2[x := {x:?}, y := {y:?}], transform := canvas::Transform[a := {:?}, b := {:?}, c := {:?}, d := {:?}, tx := 0.0, ty := 0.0], name := \"map\"]\n",
        t[0], t[1], t[2], t[3]
    )
}

/// A frame with no group is laid out by the native `canvas::sceneDrawsFlat`. Items wholly
/// past each of the four viewport edges -- Tiles and a Rectangle, so culling is not a Tile
/// rule -- publish no draw entry, while the block list keeps every item, so the visible
/// runs keep the instance bases of a list that was never culled. An item that only
/// straddles an edge is drawn. The `--debug` verify mode re-lays the same frame out with
/// the MFBASIC walk (`__canvas_pushDraw`) and must find no difference.
#[test]
fn flat_items_past_each_viewport_edge_publish_no_draw() {
    let mut body = red_image();
    body.push_str(&tile(
        "left", "img", 0.0, 0.0, 1.0, 1.0, -20.0, 100.0, 8.0, 4.0,
    ));
    body.push_str(&tile(
        "shown", "img", 0.0, 0.0, 1.0, 1.0, 100.0, 100.0, 8.0, 4.0,
    ));
    body.push_str(&tile(
        "right", "img", 0.0, 0.0, 1.0, 1.0, 910.0, 100.0, 8.0, 4.0,
    ));
    body.push_str(&tile(
        "top", "img", 0.0, 0.0, 1.0, 1.0, 100.0, -20.0, 8.0, 4.0,
    ));
    body.push_str(&tile(
        "straddle", "img", 0.0, 0.0, 1.0, 1.0, -4.0, 200.0, 8.0, 4.0,
    ));
    body.push_str(
        "  LET bottom AS canvas::DrawItem = canvas::Rectangle[id := \"bottom\", position := vector::Float2[x := 100.0, y := 700.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 8.0, height := 4.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n",
    );
    body.push_str("  canvas::present([left, shown, right, top, straddle, bottom])\n");
    let source = source(&body).replace("IMPORT io\n", "IMPORT io\nIMPORT color\n");
    let (frame, stats) = render_with("canvas_tile_cull_flat", &source, true);
    let line = stats.lines().last().expect("a stats line");
    assert_eq!(
        field(line, "blocks"),
        "6",
        "culling keeps every block: {line}"
    );
    assert_eq!(
        draw_runs(line),
        ["1:1", "4:1"],
        "only `shown` (instance 1) and `straddle` (instance 4) are drawn: {line}"
    );
    assert_eq!(field(line, "geoVerifyMismatches"), "0", "{line}");
    assert_ne!(field(line, "drawsVerified"), "0", "{line}");
    assert_eq!(pixel(&frame, 102, 101), (255, 0, 0, 255));
    assert_eq!(
        pixel(&frame, 2, 201),
        (255, 0, 0, 255),
        "straddling tile drawn"
    );
}

/// A map group is culled by its bounds after the group's transform, never before it.
/// Four frames hold the map wholly past each edge and publish nothing; the last frame
/// brings it back across three edges -- translated, scaled by a negative factor, and
/// turned a quarter. The last two have UNTRANSFORMED bounds wholly offscreen, which is
/// the false cull the plan rejects. All three are drawn; a fourth, still offscreen, is not.
#[test]
fn viewport_culling_uses_composed_group_bounds_and_keeps_the_scene() {
    const NONE: [f64; 4] = [0.0, 0.0, 0.0, 0.0];
    let mut body = red_image();
    body.push_str(&tile("cell", "img", 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 8.0, 4.0));
    body.push_str("  canvas::setGroup(\"map\", [cell])\n");
    for (name, x, y) in [
        ("offLeft", -1000.0, 0.0),
        ("offRight", 1000.0, 0.0),
        ("offTop", 0.0, -1000.0),
        ("offBottom", 0.0, 1000.0),
    ] {
        body.push_str(&group(name, x, y, NONE));
        body.push_str(&format!("  canvas::present([{name}])\n"));
    }
    // x in [896, 904), y in [100, 104): across the right edge.
    body.push_str(&group("moved", 896.0, 100.0, NONE));
    // x' = -4x: x in [888, 920), y in [200, 216). Untransformed, x is [920, 928).
    body.push_str(&group("scaled", 920.0, 200.0, [-4.0, 0.0, 0.0, 4.0]));
    // x' = -y, y' = x: x in [196, 200), y in [-4, 4). Untransformed, y is [-4, 0).
    body.push_str(&group("turned", 200.0, -4.0, [0.0, 1.0, -1.0, 0.0]));
    body.push_str("  canvas::present([moved, scaled, turned, offLeft])\n");
    let (frame, stats) = render("canvas_tile_culling", &source(&body));
    let lines: Vec<&str> = stats.lines().collect();
    assert_eq!(lines.len(), 5, "one completed frame per present: {stats}");
    for (edge, line) in ["left", "right", "top", "bottom"].into_iter().zip(&lines) {
        assert!(
            draw_runs(line).is_empty(),
            "fully offscreen {edge} group was submitted: {line}"
        );
    }
    assert_eq!(
        draw_runs(lines[4]).len(),
        3,
        "the moved, scaled and turned groups draw and the offscreen one does not: {}",
        lines[4]
    );
    assert_eq!(pixel(&frame, 898, 101), (255, 0, 0, 255), "moved");
    assert_eq!(pixel(&frame, 892, 208), (255, 0, 0, 255), "scaled");
    assert_eq!(pixel(&frame, 198, 1), (255, 0, 0, 255), "turned");
}

/// `setBytes` on an image a presented Tile samples repaints with the new pixels, as
/// `Picture`'s `set_bytes_on_a_presented_image_repaints` does. The Tile value does not
/// change, so its geometry is a cache hit; the hit must still confirm the image's pixel
/// block (`__canvas_pictureCached`), never resolve the cached record unconfirmed.
#[test]
fn set_bytes_on_a_presented_tile_image_repaints() {
    let mut body = red_image();
    body.push_str(&tile(
        "cell", "img", 0.0, 0.0, 1.0, 1.0, 100.0, 100.0, 32.0, 32.0,
    ));
    body.push_str(&format!(
        "  canvas::present([cell])\n  canvas::setBytes(img, {})\n",
        pixels(&[(0, 255, 0, 255)])
    ));
    let (frame, stats) = render("canvas_tile_set_bytes", &source(&body));
    assert_eq!(
        pixel(&frame, 116, 116),
        (0, 255, 0, 255),
        "the last frame shows the new pixels"
    );
    let last = stats.lines().last().unwrap_or_default();
    assert_eq!(
        field(last, "frames"),
        "2",
        "the present and the setBytes each rendered a frame: {stats}"
    );
}
