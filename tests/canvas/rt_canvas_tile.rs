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
        "  LET {name} AS canvas::DrawItem = canvas::Tile[id := \"{name}\", image := {image}, srcPosition := vector::Float2[x := {sx:?}, y := {sy:?}], srcSize := vector::Float2[x := {sw:?}, y := {sh:?}], position := vector::Float2[x := {x:?}, y := {y:?}], size := vector::Float2[x := {w:?}, y := {h:?}]]\n"
    )
}

fn render(name: &str, source: &str) -> (Frame, String) {
    let project = common::temp_project(name, source);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let run = Command::new(&binary)
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

#[test]
fn viewport_culling_uses_composed_group_bounds_and_keeps_the_scene() {
    let mut body = format!(
        "  RES img AS canvas::Image = canvas::createImage(1, 1, {})\n",
        pixels(&[(255, 0, 0, 255)])
    );
    body.push_str(&tile("cell", "img", 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 8.0, 4.0));
    body.push_str("  canvas::setGroup(\"map\", [cell])\n");
    for (name, x, y) in [
        ("left", -1000.0, 0.0),
        ("right", 1000.0, 0.0),
        ("top", 0.0, -1000.0),
        ("bottom", 0.0, 1000.0),
    ] {
        body.push_str(&format!(
            "  LET {name} AS canvas::DrawItem = canvas::Group[id := \"{name}\", position := vector::Float2[x := {x:?}, y := {y:?}], transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], name := \"map\"]\n  canvas::present([{name}])\n"
        ));
    }
    body.push_str("  LET visible AS canvas::DrawItem = canvas::Group[id := \"visible\", position := vector::Float2[x := 12.0, y := 2.0], transform := canvas::Transform[a := 0.0, b := 1.0, c := -2.0, d := 0.0, tx := 0.0, ty := 0.0], name := \"map\"]\n  canvas::present([visible])\n");
    let (frame, stats) = render("canvas_tile_culling", &source(&body));
    let lines: Vec<&str> = stats.lines().collect();
    assert_eq!(lines.len(), 5, "one completed frame per present: {stats}");
    for (edge, line) in ["left", "right", "top", "bottom"].into_iter().zip(&lines) {
        assert!(
            line.contains("draws=none"),
            "fully offscreen {edge} group was submitted: {line}"
        );
    }
    assert!(
        !lines[4].contains("draws=none"),
        "the transformed group that re-entered was culled: {}",
        lines[4]
    );
    assert_eq!(pixel(&frame, 8, 3), (255, 0, 0, 255));
    assert_eq!(pixel(&frame, 11, 9), (255, 0, 0, 255));
    assert_eq!(pixel(&frame, 3, 3), BLACK);
}
