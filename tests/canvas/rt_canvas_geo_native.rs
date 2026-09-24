//! The native geometry builder (`canvas::geoBuild`) and the native structural item hash
//! (`canvas::itemHash` / `canvas::sceneHashes`) — bug-686.
//!
//! `canvas::geoBuild` replaces the MFBASIC header builders for the common kinds, and its
//! record must be **bit-identical** to theirs: the software rasteriser reads it, and its
//! goldens are exact. A `--debug` build run with `MFB_CANVAS_GEO_VERIFY=1` rebuilds every
//! natively built record with the MFBASIC builders and counts the ones that differ in any
//! bit. The stats line reports `geoNative=` (records the native builder produced),
//! `geoVerified=` (records compared) and `geoVerifyMismatches=`.
//!
//! The hash half is checked by what it must do for the cache: an identical scene rebuilt
//! from scratch — whose lists carry different headroom, so its bytes differ — must hit.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build a `--debug --app` program, run it headless, synchronously and with the geometry
/// check on, and return one `MFB_CANVAS_STATS` line per frame.
fn stats(name: &str, source: &str) -> Vec<String> {
    stats_with(name, source, true)
}

/// [`stats`], optionally WITHOUT `MFB_CANVAS_SYNC`: the worker then presents as fast as
/// it can while the graphics thread renders whatever is installed, which is the only way
/// a frame can overlap a publish.
fn stats_with(name: &str, source: &str, sync: bool) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_GEO_VERIFY", "1");
    if sync {
        command.env("MFB_CANVAS_SYNC", "1");
    }
    let run = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let lines: Vec<String> = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    lines
}

/// One `name=value` field of a stats line, as a number.
fn field(line: &str, name: &str) -> i64 {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no `{name}=` field in {line:?}"))
        .parse()
        .unwrap_or_else(|e| panic!("`{name}` is not a number in {line:?}: {e}"))
}

/// Every paint the matrix crosses with every shape: fill and stroke each opaque-ish or
/// fully transparent, four stroke widths (none, thin, fractional, negative), the four
/// blend modes, and no clip / a fractional clip / a negative-width clip. A second group
/// keeps a one-stop gradient (not a gradient, so still native, but its kind and points
/// are still written) and an all-negative-zero transform (the identity by float `=`).
const PAINTS: &str = r#"
FUNC paints() AS List OF canvas::Paint
  MUT out AS List OF canvas::Paint = []
  LET fills AS List OF color::Color = [color::rgba(10, 20, 30, 200), color::rgba(0, 0, 0, 0)]
  LET strokes AS List OF color::Color = [color::rgba(200, 100, 50, 255), color::rgba(1, 2, 3, 0)]
  LET widths AS List OF Float = [0.0, 1.0, 2.75, 0.0 - 1.5]
  LET modes AS List OF canvas::BlendMode = [canvas::BlendMode.Normal, canvas::BlendMode.Multiply, canvas::BlendMode.Screen, canvas::BlendMode.Add]
  LET clips AS List OF canvas::Bounds = [canvas::Bounds[x := 0.0, y := 0.0, w := 0.0, h := 0.0], canvas::Bounds[x := 5.5, y := 0.0 - 3.25, w := 100.125, h := 50.0], canvas::Bounds[x := 10.0, y := 10.0, w := 0.0 - 5.0, h := 3.0]]
  FOR EACH f IN fills
    FOR EACH s IN strokes
      FOR EACH w IN widths
        FOR EACH m IN modes
          FOR EACH c IN clips
            out = collections::append(out, WITH canvas::fill(f) { stroke := s, strokeWidth := w, blend := m, clip := c })
          NEXT
        NEXT
      NEXT
    NEXT
  NEXT
  LET nz AS Float = 0.0 * (0.0 - 1.0)
  LET one AS canvas::GradientStop = canvas::GradientStop[offset := 0.25, color := color::rgb(9, 8, 7)]
  LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, startPoint := canvas::Point[x := 1.5, y := 2.5], endPoint := canvas::Point[x := 0.0 - 3.5, y := 4.25], stops := [one]]
  LET flat AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := 7.0, y := 8.0], endPoint := canvas::Point[x := 9.0, y := 10.0], stops := []]
  LET negZero AS canvas::Transform = canvas::Transform[a := nz, b := nz, c := nz, d := nz, tx := nz, ty := nz]
  out = collections::append(out, WITH canvas::fill(color::rgb(1, 2, 3)) { stroke := color::rgb(4, 5, 6), strokeWidth := 2.0, fillGradient := ramp })
  out = collections::append(out, WITH canvas::fill(color::rgb(1, 2, 3)) { fillGradient := flat, blend := canvas::BlendMode.Add })
  out = collections::append(out, WITH canvas::stroke(color::rgb(4, 5, 6), 3.0) { transform := negZero })
  RETURN out
END FUNC
"#;

/// Every native kind at fractional, negative, large and tiny coordinates, and every
/// degenerate size the builders special-case: zero and negative width, height and
/// radius, a corner radius below zero and past the limit, zero-length and
/// negative-zero lines, and polygons of 0, 1 and 2 points, collinear and with a
/// duplicate vertex.
const SHAPES: &str = r#"
FUNC pt(x AS Float, y AS Float) AS canvas::Point
  RETURN canvas::Point[x := x, y := y]
END FUNC

FUNC shapes(p AS canvas::Paint) AS List OF canvas::DrawItem
  MUT out AS List OF canvas::DrawItem = []
  LET nz AS Float = 0.0 * (0.0 - 1.0)
  LET r1 AS canvas::DrawItem = canvas::Rectangle[x := 10.5, y := 20.25, w := 30.0, h := 40.0, paint := p]
  LET r2 AS canvas::DrawItem = canvas::Rectangle[x := 0.0 - 15.75, y := 0.0 - 3.5, w := 7.25, h := 9.125, paint := p]
  LET r3 AS canvas::DrawItem = canvas::Rectangle[x := 1000000.3, y := 0.0 - 2000000.0, w := 1234.5, h := 0.001, paint := p]
  LET r4 AS canvas::DrawItem = canvas::Rectangle[x := 5.0, y := 5.0, w := 0.0, h := 10.0, paint := p]
  LET r5 AS canvas::DrawItem = canvas::Rectangle[x := 5.0, y := 5.0, w := 0.0 - 3.0, h := 10.0, paint := p]
  LET r6 AS canvas::DrawItem = canvas::Rectangle[x := 5.0, y := 5.0, w := 10.0, h := 0.0, paint := p]
  LET r7 AS canvas::DrawItem = canvas::Rectangle[x := 5.0, y := 5.0, w := 10.0, h := 0.0 - 0.5, paint := p]
  LET r8 AS canvas::DrawItem = canvas::Rectangle[x := nz, y := nz, w := 0.000000001, h := 0.3, paint := p]
  LET q1 AS canvas::DrawItem = canvas::RoundedRect[x := 10.0, y := 10.0, w := 40.0, h := 20.0, cornerRadius := 5.0, paint := p]
  LET q2 AS canvas::DrawItem = canvas::RoundedRect[x := 10.0, y := 10.0, w := 40.0, h := 20.0, cornerRadius := 100.0, paint := p]
  LET q3 AS canvas::DrawItem = canvas::RoundedRect[x := 10.0, y := 10.0, w := 40.0, h := 20.0, cornerRadius := 0.0 - 3.0, paint := p]
  LET q4 AS canvas::DrawItem = canvas::RoundedRect[x := 0.0 - 3.3, y := 7.7, w := 0.5, h := 8.25, cornerRadius := 0.2, paint := p]
  LET q5 AS canvas::DrawItem = canvas::RoundedRect[x := 1.0, y := 1.0, w := 0.0, h := 8.0, cornerRadius := 2.0, paint := p]
  LET c1 AS canvas::DrawItem = canvas::Circle[x := 100.5, y := 50.25, radius := 20.0, paint := p]
  LET c2 AS canvas::DrawItem = canvas::Circle[x := 0.0 - 7.0, y := 0.0 - 8.0, radius := 0.5, paint := p]
  LET c3 AS canvas::DrawItem = canvas::Circle[x := 300000.0, y := 1.5, radius := 12345.678, paint := p]
  LET c4 AS canvas::DrawItem = canvas::Circle[x := 3.0, y := 3.0, radius := 0.0, paint := p]
  LET c5 AS canvas::DrawItem = canvas::Circle[x := 3.0, y := 3.0, radius := 0.0 - 1.0, paint := p]
  LET l1 AS canvas::DrawItem = canvas::Line[x1 := 1.0, y1 := 2.0, x2 := 300.0, y2 := 200.0, cap := canvas::CapStyle.Round, paint := p]
  LET l2 AS canvas::DrawItem = canvas::Line[x1 := 1.0, y1 := 2.0, x2 := 300.0, y2 := 200.0, cap := canvas::CapStyle.Butt, paint := p]
  LET l3 AS canvas::DrawItem = canvas::Line[x1 := 5.0, y1 := 5.0, x2 := 5.0, y2 := 5.0, cap := canvas::CapStyle.Round, paint := p]
  LET l4 AS canvas::DrawItem = canvas::Line[x1 := 0.0 - 10.5, y1 := 3.25, x2 := 7.75, y2 := 0.0 - 2.0, cap := canvas::CapStyle.Butt, paint := p]
  LET l5 AS canvas::DrawItem = canvas::Line[x1 := 1000000.0, y1 := 1000000.0, x2 := 1000001.0, y2 := 999999.0, cap := canvas::CapStyle.Round, paint := p]
  LET l6 AS canvas::DrawItem = canvas::Line[x1 := nz, y1 := 0.0, x2 := 0.0, y2 := nz, cap := canvas::CapStyle.Butt, paint := p]
  LET none AS List OF canvas::Point = []
  LET g1 AS canvas::DrawItem = canvas::Polygon[points := [pt(10.0, 10.0), pt(60.0, 15.5), pt(30.25, 70.0)], paint := p]
  LET g2 AS canvas::DrawItem = canvas::Polygon[points := [pt(1.0, 2.0), pt(3.0, 4.0)], paint := p]
  LET g3 AS canvas::DrawItem = canvas::Polygon[points := [pt(1.0, 2.0)], paint := p]
  LET g4 AS canvas::DrawItem = canvas::Polygon[points := none, paint := p]
  LET g5 AS canvas::DrawItem = canvas::Polygon[points := [pt(0.0, 0.0), pt(5.0, 5.0), pt(10.0, 10.0)], paint := p]
  LET g6 AS canvas::DrawItem = canvas::Polygon[points := [pt(0.0, 0.0), pt(8.0, 0.0), pt(8.0, 0.0), pt(8.0, 8.0), pt(0.0, 8.0)], paint := p]
  LET g7 AS canvas::DrawItem = canvas::Polygon[points := [pt(0.0 - 4.125, 0.0 - 9.5), pt(2000000.5, 3.0), pt(nz, 0.1), pt(0.3, 0.7), pt(0.0 - 0.001, 5.0)], paint := p]
  out = [r1, r2, r3, r4, r5, r6, r7, r8, q1, q2, q3, q4, q5, c1, c2, c3, c4, c5, l1, l2, l3, l4, l5, l6, g1, g2, g3, g4, g5, g6, g7]
  RETURN out
END FUNC
"#;

/// Frame 1 is the whole native matrix. Frame 2 is items the native builder must
/// decline — a real transform, a two-stop gradient, an `Ellipse`, an `Arc` — so the
/// MFBASIC builders still run for them.
fn matrix_program() -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\n\
         {PAINTS}{SHAPES}\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         MUT items AS List OF canvas::DrawItem = []\n  \
         FOR EACH p IN paints()\n    \
         items = collections::append(items, shapes(p))\n  \
         NEXT\n  \
         canvas::present(items)\n  \
         LET t AS canvas::Transform = canvas::Transform[a := 1.0, b := 0.0, c := 0.0, d := 1.0, tx := 3.5, ty := 0.0]\n  \
         LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]]\n  \
         LET moved AS canvas::DrawItem = canvas::Rectangle[x := 1.0, y := 2.0, w := 3.0, h := 4.0, paint := WITH canvas::fill(color::rgb(1, 2, 3)) {{ transform := t }}]\n  \
         LET grad AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := 0.0, y := 0.0], endPoint := canvas::Point[x := 10.0, y := 0.0], stops := stops]\n  \
         LET gradPaint AS canvas::Paint = WITH canvas::fill(color::rgb(1, 2, 3)) {{ fillGradient := grad }}\n  \
         LET ramp AS canvas::DrawItem = canvas::Circle[x := 50.0, y := 50.0, radius := 9.0, paint := gradPaint]\n  \
         LET oval AS canvas::DrawItem = canvas::Ellipse[x := 40.0, y := 40.0, radiusX := 10.0, radiusY := 5.0, angle := 0.5, paint := canvas::fill(color::rgb(9, 9, 9))]\n  \
         LET bow AS canvas::DrawItem = canvas::Arc[x := 40.0, y := 40.0, radius := 10.0, startAngle := 0.0, endAngle := 2.0, cap := canvas::CapStyle.Round, paint := canvas::stroke(color::rgb(9, 9, 9), 2.0)]\n  \
         canvas::present([moved, ramp, oval, bow])\n  \
         io::print(\"rendered\")\n\
         END SUB\n"
    )
}

/// Every record the native builder produces for the matrix is bit-identical to the one
/// the MFBASIC builders produce, and the native builder is what built every item of it.
#[test]
fn the_native_geometry_matches_the_mfbasic_builders_bit_for_bit() {
    let lines = stats("canvas_geo_native_matrix", &matrix_program());
    assert_eq!(lines.len(), 2, "one frame per present: {lines:?}");
    let (matrix, declined) = (&lines[0], &lines[1]);

    let native = field(matrix, "geoNative");
    let built = field(matrix, "generations");
    assert!(
        native > 1000,
        "the matrix is ~6,000 items of the five native kinds (distinct ones: over a \
         thousand), but the native builder produced {native} records.\n{matrix}"
    );
    assert_eq!(
        native, built,
        "every item of the matrix is a native kind with an identity transform and no \
         gradient, so every geometry build must have been the native one.\n{matrix}"
    );
    assert_eq!(
        field(matrix, "geoVerified"),
        native,
        "with MFB_CANVAS_GEO_VERIFY=1 every native record is checked.\n{matrix}"
    );
    assert_eq!(
        field(matrix, "geoVerifyMismatches"),
        0,
        "a native geometry record differs from the MFBASIC builders' in at least one \
         bit. The software rasteriser reads this record and its goldens are exact, so \
         the two must agree slot for slot (`func_geo_build.rs`).\n{matrix}"
    );

    assert_eq!(
        field(declined, "geoNative"),
        native,
        "a transform, a two-stop gradient, an Ellipse and an Arc are the MFBASIC \
         builders' — the native builder must decline all four.\n{declined}"
    );
    assert_eq!(
        field(declined, "generations") - built,
        4,
        "the four declined items must still be built, by the MFBASIC path.\n{declined}"
    );
    assert_eq!(field(declined, "geoVerifyMismatches"), 0, "{declined}");
}

/// The item hash is STRUCTURAL: the same scene rebuilt from scratch — its point lists
/// built by append (with headroom) instead of by literal, its paints rebuilt — hashes
/// the same, so the geometry cache hits and nothing is rebuilt but the one item that
/// moved. A hash over the item's bytes would miss on every item: the blocks differ in
/// capacity and padding although the values are equal.
#[test]
fn a_rebuilt_identical_scene_hits_the_geometry_cache() {
    let source = "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\n\n\
         FUNC scene(frame AS Integer, grown AS Boolean) AS List OF canvas::DrawItem\n  \
         MUT items AS List OF canvas::DrawItem = []\n  \
         MUT i AS Integer = 0\n  \
         WHILE i < 50\n    \
         LET x AS Float = toFloat(i) * 7.5\n    \
         MUT pts AS List OF canvas::Point = [canvas::Point[x := x, y := 1.0], canvas::Point[x := x + 5.0, y := 9.0], canvas::Point[x := x, y := 12.0]]\n    \
         IF grown THEN\n      \
         pts = []\n      \
         pts = collections::append(pts, canvas::Point[x := x, y := 1.0])\n      \
         pts = collections::append(pts, canvas::Point[x := x + 5.0, y := 9.0])\n      \
         pts = collections::append(pts, canvas::Point[x := x, y := 12.0])\n    \
         END IF\n    \
         LET poly AS canvas::DrawItem = canvas::Polygon[points := pts, paint := canvas::fill(color::rgb(i, 100, 200))]\n    \
         LET box AS canvas::DrawItem = canvas::Rectangle[x := x, y := 40.0, w := 5.0, h := 5.0, paint := canvas::fillStroke(color::rgb(1, 2, 3), color::rgb(4, 5, 6), 1.5)]\n    \
         items = collections::append(items, poly)\n    \
         items = collections::append(items, box)\n    \
         i = i + 1\n  \
         END WHILE\n  \
         LET dot AS canvas::DrawItem = canvas::Circle[x := toFloat(frame) * 4.0 + 10.0, y := 300.0, radius := 5.0, paint := canvas::fill(color::rgb(255, 0, 0))]\n  \
         items = collections::append(items, dot)\n  \
         RETURN items\n\
         END FUNC\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         canvas::present(scene(0, FALSE))\n  \
         canvas::present(scene(1, TRUE))\n  \
         canvas::present(scene(2, FALSE))\n  \
         io::print(\"rendered\")\n\
         END SUB\n";
    let lines = stats("canvas_geo_native_rehash", source);
    assert_eq!(lines.len(), 3, "one frame per present: {lines:?}");
    assert_eq!(
        field(&lines[0], "generations"),
        101,
        "the first frame builds its 101 items.\n{}",
        lines[0]
    );
    for pair in lines.windows(2) {
        let built = field(&pair[1], "generations") - field(&pair[0], "generations");
        assert_eq!(
            built, 1,
            "a rebuilt scene equal in value to the last one must rebuild only the one \
             item that moved; {built} were rebuilt, so equal items hashed differently.\n\
             before: {}\nafter:  {}",
            pair[0], pair[1],
        );
    }
    assert_eq!(field(&lines[2], "geoVerifyMismatches"), 0, "{}", lines[2]);
}

/// The frame pass itself (`canvas::sceneResolve`, `canvas::sceneLayout`,
/// `canvas::sceneDrawsFlat`), over a moving scene that mixes the kinds it builds with
/// every kind it hands to MFBASIC: a transformed rectangle, an `Ellipse`, a `Picture`,
/// then a `Group` node, then a layered scene. With `MFB_CANVAS_GEO_VERIFY=1` every record
/// it built is rebuilt by the MFBASIC builders, every draw hash it folded is refolded,
/// and every group-free draw list it laid out is laid out again by the MFBASIC walk —
/// all bit for bit. Every item moves every frame, so the arena compacts and the index is
/// rebuilt on most frames.
#[test]
fn the_native_frame_pass_matches_the_mfbasic_walk_on_a_mixed_moving_scene() {
    let source = "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\n\n\
         FUNC shapes(frame AS Integer) AS List OF canvas::DrawItem\n  \
         MUT items AS List OF canvas::DrawItem = []\n  \
         LET t AS canvas::Transform = canvas::Transform[a := 1.0, b := 0.0, c := 0.0, d := 1.0, tx := 2.5, ty := 0.0]\n  \
         MUT k AS Integer = 0\n  \
         WHILE k < 120\n    \
         LET x AS Float = toFloat((k * 37) MOD 700) + toFloat(frame) * 1.25\n    \
         LET y AS Float = toFloat((k * 53) MOD 500)\n    \
         LET box AS canvas::DrawItem = canvas::Rectangle[x := x, y := y, w := 6.0, h := 4.5, paint := canvas::fillStroke(color::rgba(k, 100, 200, 180), color::rgb(1, 2, 3), 1.5)]\n    \
         LET blended AS canvas::DrawItem = canvas::Circle[x := x, y := y + 9.0, radius := 3.0, paint := WITH canvas::fillStroke(color::rgb(9, 9, 9), color::rgb(200, 1, 1), 2.0) { blend := canvas::BlendMode.Multiply }]\n    \
         LET seg AS canvas::DrawItem = canvas::Line[x1 := x, y1 := y, x2 := x + 10.0, y2 := y + 3.0, cap := canvas::CapStyle.Round, paint := canvas::stroke(color::rgb(255, 255, 0), 1.0)]\n    \
         LET tri AS canvas::DrawItem = canvas::Polygon[points := [canvas::Point[x := x, y := y], canvas::Point[x := x + 8.0, y := y + 1.0], canvas::Point[x := x + 3.0, y := y + 7.0]], paint := canvas::fill(color::rgb(0, 255, 0))]\n    \
         items = collections::append(items, box)\n    \
         items = collections::append(items, blended)\n    \
         items = collections::append(items, seg)\n    \
         items = collections::append(items, tri)\n    \
         IF k MOD 30 = 0 THEN\n      \
         LET moved AS canvas::DrawItem = canvas::Rectangle[x := x, y := y, w := 5.0, h := 5.0, paint := WITH canvas::fill(color::rgb(1, 2, 3)) { transform := t }]\n      \
         LET oval AS canvas::DrawItem = canvas::Ellipse[x := x, y := y, radiusX := 6.0, radiusY := 3.0, angle := 0.25, paint := canvas::fill(color::rgb(9, 9, 200))]\n      \
         items = collections::append(items, moved)\n      \
         items = collections::append(items, oval)\n    \
         END IF\n    \
         k = k + 1\n  \
         END WHILE\n  \
         RETURN items\n\
         END FUNC\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         RES img AS canvas::Image = canvas::createImage(2, 1, [toByte(255), toByte(0), toByte(0), toByte(255), toByte(0), toByte(0), toByte(255), toByte(255)])\n  \
         LET badge AS canvas::DrawItem = canvas::Rectangle[x := 1.0, y := 1.0, w := 3.0, h := 3.0, paint := canvas::fill(color::rgb(4, 4, 4))]\n  \
         canvas::setGroup(\"badge\", [badge])\n  \
         MUT frame AS Integer = 0\n  \
         WHILE frame < 8\n    \
         MUT items AS List OF canvas::DrawItem = shapes(frame)\n    \
         LET pic AS canvas::DrawItem = canvas::Picture[x := toFloat(frame), y := 600.0, w := 8.0, h := 4.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255))]\n    \
         items = collections::append(items, pic)\n    \
         IF frame >= 6 THEN\n      \
         LET node AS canvas::DrawItem = canvas::Group[name := \"badge\", dx := toFloat(frame), dy := 3.0]\n      \
         items = collections::append(items, node)\n    \
         END IF\n    \
         canvas::present(items)\n    \
         frame = frame + 1\n  \
         END WHILE\n  \
         LET back AS canvas::DrawLayer = canvas::DrawLayer[items := shapes(20)]\n  \
         LET front AS canvas::DrawLayer = canvas::DrawLayer[items := shapes(21)]\n  \
         canvas::presentLayers([back, front])\n  \
         io::print(\"rendered\")\n\
         END SUB\n";
    let lines = stats("canvas_geo_native_frame_pass", source);
    assert_eq!(lines.len(), 9, "one frame per present: {lines:?}");
    let last = &lines[8];
    assert_eq!(
        field(last, "geoVerifyMismatches"),
        0,
        "the native frame pass disagreed with the MFBASIC builders, fold or walk.\n{last}"
    );
    assert!(
        field(last, "geoVerified") >= 8 * 480,
        "every moving shape of the eight frames is built natively and re-checked.\n{last}"
    );
    assert!(
        field(last, "drawsVerified") >= 7,
        "six group-free frames and the layered one are laid out natively and \
         re-checked.\n{last}"
    );
    assert!(
        field(last, "geoCompactions") >= 3,
        "a scene whose items all move compacts the arena at frame boundaries.\n{last}"
    );
    for pair in lines[..8].windows(2) {
        let built = field(&pair[1], "generations") - field(&pair[0], "generations");
        assert!(
            (480..=490).contains(&built),
            "every one of the 480 moving shapes changed, plus the transformed rectangles \
             and ellipses (8) and the picture (1): {built} builds.\nbefore: {}\nafter:  {}",
            pair[0],
            pair[1],
        );
    }
}

/// A frame draws ONE scene: every index it resolves — cache hits included, of every kind
/// — draws the geometry of the item the frame holds at that index (bug-686).
///
/// A hit is trusted on the item's hash alone, and the hashes are published by a second
/// call (`canvas::publishHashes`) after the scene (`canvas::publishScene`). The worker
/// here alternates two scenes of the same length, A and B, 60 times, a few milliseconds
/// apart and without `MFB_CANVAS_SYNC`, so the graphics thread renders while publishes
/// land. A and B put
/// different kinds at the same indices and different positions on them, and every item of
/// both is cached after the first two frames — so a frame that pairs one scene's items
/// with the other's hashes draws a HIT of the wrong item, and one that files a miss under
/// the other scene's hash keeps drawing it. Every tenth index is a kind the MFBASIC path
/// builds (an ellipse in A, a transformed rectangle in B).
///
/// `MFB_CANVAS_GEO_VERIFY=1` checks every resolved index of every frame against the
/// MFBASIC builders' record for the item at that index (`geoResolvedChecked=`).
#[test]
fn a_frame_never_draws_one_scenes_geometry_for_anothers_items() {
    let source = "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\nIMPORT os\n\n\
         FUNC scene(flip AS Boolean) AS List OF canvas::DrawItem\n  \
         MUT items AS List OF canvas::DrawItem = []\n  \
         LET t AS canvas::Transform = canvas::Transform[a := 1.0, b := 0.0, c := 0.0, d := 1.0, tx := 1.5, ty := 0.0]\n  \
         MUT k AS Integer = 0\n  \
         WHILE k < 6000\n    \
         LET px AS Float = toFloat((k * 37) MOD 880)\n    \
         LET py AS Float = toFloat((k * 53) MOD 620)\n    \
         LET qx AS Float = toFloat((k * 41) MOD 880) + 0.5\n    \
         LET qy AS Float = toFloat((k * 59) MOD 620) + 0.25\n    \
         LET box AS canvas::DrawItem = canvas::Rectangle[x := px, y := py, w := 6.0, h := 4.0, paint := canvas::fill(color::rgb(0, 200, 255))]\n    \
         LET dot AS canvas::DrawItem = canvas::Circle[x := qx, y := qy, radius := 3.0, paint := canvas::fill(color::rgb(255, 80, 0))]\n    \
         LET oval AS canvas::DrawItem = canvas::Ellipse[x := qx, y := qy, radiusX := 5.0, radiusY := 2.0, angle := 0.5, paint := canvas::fill(color::rgb(9, 9, 200))]\n    \
         LET moved AS canvas::DrawItem = canvas::Rectangle[x := px, y := py, w := 5.0, h := 5.0, paint := WITH canvas::fill(color::rgb(1, 200, 3)) { transform := t }]\n    \
         IF k MOD 10 = 0 THEN\n      \
         IF flip THEN\n        \
         items = collections::append(items, moved)\n      \
         ELSE\n        \
         items = collections::append(items, oval)\n      \
         END IF\n    \
         ELSE\n      \
         IF (k MOD 2 = 0) = flip THEN\n        \
         items = collections::append(items, dot)\n      \
         ELSE\n        \
         items = collections::append(items, box)\n      \
         END IF\n    \
         END IF\n    \
         k = k + 1\n  \
         END WHILE\n  \
         RETURN items\n\
         END FUNC\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         LET a AS List OF canvas::DrawItem = scene(FALSE)\n  \
         LET b AS List OF canvas::DrawItem = scene(TRUE)\n  \
         MUT n AS Integer = 0\n  \
         WHILE n < 60\n    \
         IF n MOD 2 = 0 THEN\n      \
         canvas::present(a)\n    \
         ELSE\n      \
         canvas::present(b)\n    \
         END IF\n    \
         os::sleep(20)\n    \
         n = n + 1\n  \
         END WHILE\n  \
         io::print(\"rendered\")\n\
         END SUB\n";
    let lines = stats_with("canvas_geo_native_publish_race", source, false);
    let last = lines.last().expect("at least one frame rendered");
    assert!(
        lines.len() >= 3,
        "the graphics thread rendered only {} frames while the worker presented 60 \
         scenes; the race needs frames to overlap presents.\n{last}",
        lines.len()
    );
    assert!(
        field(last, "geoResolvedChecked") >= 6000,
        "every resolved index of every frame is checked.\n{last}"
    );
    assert_eq!(
        field(last, "geoVerifyMismatches"),
        0,
        "a frame drew geometry that is not its item's: the items, the layers and the \
         hashes a frame reads must come from ONE publish of the scene, and a hit must be \
         taken by the hash published for that scene.\n{last}"
    );
}
