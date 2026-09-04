//! `canvas::loadFont` accepts TrueType and refuses everything else (plan-98-G Phase 1).
//!
//! These run a real headless `--app` program rather than inspecting the emitted code,
//! for the same reason the rasteriser tests do: what has to be true is a statement
//! about *behaviour at the boundary* — which files are accepted, and which error each
//! rejected file produces — and a codegen-shape assertion would pass just as happily
//! while the version check compared the wrong bytes.
//!
//! **The fixtures are written by the program under test, not committed.** A twelve-byte
//! sfnt header is all `loadFont` reads, so the test can build one for each case it
//! wants and does not need a real font in the repository. That also makes the *negative*
//! cases exact: `OTTO`, `ttcf` and `wOFF` are each a real thing a program might hand us
//! and each must be refused by the same rule, which is hard to arrange with borrowed
//! system fonts and trivial here.

mod common;

use common::canvas_image::{compare_exact, compare_within_tolerance, Frame, Tolerance};
use std::process::Command;

/// Build a `--app` program, run it headless, and return its stdout lines.
fn run(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_app(&project, name);
    let out = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        out.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&out.status),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    lines
}

/// Writes a 12-byte sfnt header with the given first four bytes, then loads it.
///
/// Twelve bytes because that is the whole table-directory header — version, table
/// count, and the three binary-search fields — and `loadFont`'s length guard rejects
/// anything shorter, so a four-byte file would be refused for the wrong reason and the
/// test would pass without testing the version check at all.
const PROGRAM: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT fs
IMPORT io

FUNC attempt(label AS String, b0 AS Integer, b1 AS Integer, b2 AS Integer, b3 AS Integer) AS String
  MUT bytes AS List OF Byte = [toByte(b0), toByte(b1), toByte(b2), toByte(b3)]
  MUT i AS Integer = 0
  WHILE i < 8
    bytes = collections::append(bytes, toByte(0))
    i = i + 1
  END WHILE
  LET path AS String = label & ".bin"
  fs::writeBytes(path, bytes) TRAP(e)
    RETURN label & ": could not write the fixture"
  END TRAP
  RES f AS canvas::Font = canvas::loadFont(path) TRAP(e)
    RETURN label & ": refused " & toString(e.code)
  END TRAP
  LET r AS canvas::FontRef = canvas::fontRef(f)
  IF r.id = 0 THEN
    RETURN label & ": accepted with a zero handle"
  END IF
  canvas::destroyFont(f)
  RETURN label & ": accepted"
END FUNC

SUB main()
  app::setMode(app::Mode.Canvas)
  io::print(attempt("truetype", 0, 1, 0, 0))
  io::print(attempt("appletrue", 116, 114, 117, 101))
  io::print(attempt("otto", 79, 84, 84, 79))
  io::print(attempt("ttcf", 116, 116, 99, 102))
  io::print(attempt("woff", 119, 79, 70, 70))
  MUT short AS List OF Byte = [toByte(0), toByte(1), toByte(0), toByte(0)]
  fs::writeBytes("short.bin", short) TRAP(e)
    EXIT SUB
  END TRAP
  RES g AS canvas::Font = canvas::loadFont("short.bin") TRAP(e)
    io::print("short: refused " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("short: accepted a four-byte file")
END SUB
"#;

/// `ErrBadFontFile`, from `errorCode`. Spelled here so a renumbering of the table
/// fails this test rather than silently changing what a program sees.
const ERR_BAD_FONT_FILE: &str = "77050022";
/// `ErrNotFound` — the *other* answer `loadFont` can give, and the reason
/// `ErrBadFontFile` exists as a separate code at all.
const ERR_NOT_FOUND: &str = "77030001";

#[test]
fn load_font_accepts_truetype_outlines_and_refuses_every_other_container() {
    let lines = run("canvas_font_versions", PROGRAM);
    let find = |label: &str| -> String {
        lines
            .iter()
            .find(|l| l.starts_with(&format!("{label}:")))
            .unwrap_or_else(|| panic!("no `{label}` line in {lines:?}"))
            .clone()
    };

    // The two spellings of TrueType outlines. `0x00010000` is the version every
    // Windows-era font uses; `true` is Apple's, and dropping it would refuse a large
    // part of the fonts shipped on the development host.
    assert_eq!(find("truetype"), "truetype: accepted");
    assert_eq!(find("appletrue"), "appletrue: accepted");

    // Each rejected container is a real file a program might hand us, and each is
    // refused for its own reason: `OTTO` is CFF outlines (a different curve type),
    // `ttcf` holds several fonts so "the font" is ambiguous, and `wOFF` is compressed.
    for label in ["otto", "ttcf", "woff"] {
        assert_eq!(
            find(label),
            format!("{label}: refused {ERR_BAD_FONT_FILE}"),
            "{label} must be refused as a bad font file, not accepted or mis-coded",
        );
    }

    // A file too short to hold a table directory is refused by the same rule rather
    // than read past the end.
    assert_eq!(find("short"), format!("short: refused {ERR_BAD_FONT_FILE}"));
}

#[test]
fn a_missing_path_is_not_reported_as_a_bad_font() {
    let lines = run(
        "canvas_font_missing",
        r#"IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("no-such-font.ttf") TRAP(e)
    io::print("missing: " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("missing: loaded a font that does not exist")
END SUB
"#,
    );
    // The distinction is the whole reason `ErrBadFontFile` was added: a path typo and
    // a wrong format need different fixes, and collapsing them into one code would
    // send every reader to the wrong one.
    assert_eq!(
        lines.first().map(String::as_str),
        Some(format!("missing: {ERR_NOT_FOUND}").as_str()),
        "a missing path must stay ErrNotFound, not become ErrBadFontFile: {lines:?}",
    );
}

/// A minimal but *valid* TrueType file, built here so every expected metric below is
/// derivable rather than copied from a run.
///
/// Borrowing a system font would make the assertions numbers-from-a-run: correct only
/// as long as nobody looks, and untestable on a machine with a different font. Four
/// tables is all `measureText` reads — `cmap` for codepoint→glyph, `hmtx` for the
/// advance, `hhea` for `numberOfHMetrics` and the vertical metrics, `head` for
/// `unitsPerEm` — so the fixture is small enough to state completely:
///
/// * `unitsPerEm` 1000, so a `size` of 100 scales by exactly `0.1`;
/// * ascender 800, descender **-200** (the file stores it negative), lineGap 100;
/// * `numberOfHMetrics` 3, advances `[500, 250, 300]`;
/// * `cmap` maps `A`→glyph 1 and `B`→glyph 2, and nothing else.
///
/// So `A` is 25.0 px wide, `B` is 30.0, and any unmapped character falls to glyph 0 at
/// 50.0. `descent` comes back **positive** (20.0) because `TextMetrics` documents it
/// that way, and `height` is `80 + 20 + 10 = 110.0`.
fn minimal_truetype() -> Vec<u8> {
    truetype_fixture(1000, 2, [100, 0, 400, 300])
}

/// `minimal_truetype`, parameterised for the bug-509 bombs: `head.unitsPerEm`, the
/// `numGroups` the cmap subtable *claims* (it always holds exactly two), and the square
/// glyph's `[xMin, yMin, xMax, yMax]` in font units.
fn truetype_fixture(upem: u16, groups: u32, square: [i16; 4]) -> Vec<u8> {
    fn be16(v: u16) -> [u8; 2] {
        v.to_be_bytes()
    }
    fn be32(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    // `cmap`: one format-12 subtable, two single-codepoint groups.
    let mut cmap = Vec::new();
    cmap.extend(be16(0)); // version
    cmap.extend(be16(1)); // numTables
    cmap.extend(be16(3)); // platformID: Windows
    cmap.extend(be16(10)); // encodingID: UCS-4
    cmap.extend(be32(12)); // subtable offset, from the start of `cmap`
    cmap.extend(be16(12)); // format 12
    cmap.extend(be16(0)); // reserved
    cmap.extend(be32(40)); // subtable length
    cmap.extend(be32(0)); // language
    cmap.extend(be32(groups)); // numGroups — the subtable itself always holds two
    for (ch, gid) in [(b'A' as u32, 1u32), (b'B' as u32, 2)] {
        cmap.extend(be32(ch)); // startCharCode
        cmap.extend(be32(ch)); // endCharCode
        cmap.extend(be32(gid)); // startGlyphID
    }

    // `head`: 54 bytes, and only `unitsPerEm` at +18 is read.
    let mut head = vec![0u8; 54];
    head[18..20].copy_from_slice(&be16(upem));

    // `hhea`: 36 bytes — ascender/descender/lineGap at +4/+6/+8, numberOfHMetrics at +34.
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&be16(800));
    hhea[6..8].copy_from_slice(&be16((-200i16) as u16));
    hhea[8..10].copy_from_slice(&be16(100));
    hhea[34..36].copy_from_slice(&be16(3));

    // `hmtx`: three (advanceWidth, leftSideBearing) pairs.
    let mut hmtx = Vec::new();
    for advance in [500u16, 250, 300] {
        hmtx.extend(be16(advance));
        hmtx.extend(be16(0));
    }

    // `glyf`: glyph 1 is a square, `(100,0)` to `(400,300)` in font units. Four
    // on-curve points and no instructions, so the reader's every branch is the simple
    // one and the expected pixels are a rectangle anyone can compute.
    let [x0, y0, x1, y1] = square;
    let mut glyf = Vec::new();
    glyf.extend(be16(1)); // numberOfContours
    for v in square {
        glyf.extend(be16(v as u16)); // xMin, yMin, xMax, yMax
    }
    glyf.extend(be16(3)); // endPtsOfContours[0] — four points, so the last index is 3
    glyf.extend(be16(0)); // instructionLength
    for _ in 0..4 {
        glyf.push(0x01); // ON_CURVE, and neither axis short or repeated
    }
    // (x0,y0) → (x1,y0) → (x1,y1) → (x0,y1), as deltas from the previous point.
    for dx in [x0, x1 - x0, 0, x0 - x1] {
        glyf.extend(be16(dx as u16));
    }
    for dy in [y0, 0, y1 - y0, 0] {
        glyf.extend(be16(dy as u16));
    }

    // `loca`, 16-bit format (`head.indexToLocFormat` is 0, which the zeroed `head`
    // already says): offsets are stored halved. Glyphs 0 and 2 are empty ranges,
    // which is how the format spells "no outline" — a space, and `.notdef` here.
    let glyf_len = glyf.len() as u16;
    let mut loca = Vec::new();
    for halved in [0u16, 0, glyf_len / 2, glyf_len / 2] {
        loca.extend(be16(halved));
    }

    // Tag order is the sorted order a real directory uses.
    let tables: [(&[u8; 4], Vec<u8>); 6] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
    ];

    let mut font = Vec::new();
    font.extend(be32(0x0001_0000)); // sfnt version: TrueType outlines
    font.extend(be16(tables.len() as u16));
    font.extend(be16(0)); // searchRange — unread, this reader scans linearly
    font.extend(be16(0)); // entrySelector
    font.extend(be16(0)); // rangeShift

    let mut offset = 12 + 16 * tables.len() as u32;
    debug_assert!(
        glyf_len % 2 == 0,
        "a 16-bit `loca` cannot address an odd offset"
    );
    let mut body: Vec<u8> = Vec::new();
    for (tag, data) in &tables {
        font.extend(*tag);
        font.extend(be32(0)); // checksum — unread
        font.extend(be32(offset));
        font.extend(be32(data.len() as u32));
        offset += data.len() as u32;
        body.extend(data);
    }
    font.extend(body);
    font
}

/// Build a project, drop `fixture.ttf` beside it, run, and return stdout lines.
fn run_with_font(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), minimal_truetype()).expect("write the font");
    let binary = common::build_app(&project, name);
    let out = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        out.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&out.status),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    lines
}

const MEASURE: &str = r#"IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load failed " & toString(e.code))
    EXIT SUB
  END TRAP
  FOR EACH s IN ["A", "B", "AB", "X", "AXB", ""]
    LET m AS canvas::TextMetrics = canvas::measureText(face, 100.0, s)
    io::print("[" & s & "] w=" & toString(m.width) & " h=" & toString(m.height) & " a=" & toString(m.ascent) & " d=" & toString(m.descent) & " g=" & toString(m.lineGap))
  NEXT
  LET half AS canvas::TextMetrics = canvas::measureText(face, 50.0, "AB")
  io::print("[half] w=" & toString(half.width))
END SUB
"#;

#[test]
fn measure_text_scales_the_fonts_own_metrics() {
    let lines = run_with_font("canvas_measure_text", MEASURE);
    let at = |i: usize| lines.get(i).cloned().unwrap_or_default();

    // Per-glyph advances straight out of `hmtx`, scaled by `size / unitsPerEm` = 0.1.
    // The vertical numbers are the same for every string, including the empty one: a
    // line has a height whether or not anything is on it.
    assert_eq!(at(0), "[A] w=25.00 h=110.00 a=80.00 d=20.00 g=10.00");
    assert_eq!(at(1), "[B] w=30.00 h=110.00 a=80.00 d=20.00 g=10.00");
    assert_eq!(at(2), "[AB] w=55.00 h=110.00 a=80.00 d=20.00 g=10.00");

    // An unmapped codepoint is glyph 0 — `.notdef` — which has its own advance. The
    // font must not be asked to fail here: a missing glyph draws the empty box the
    // font provides and takes up its width.
    assert_eq!(at(3), "[X] w=50.00 h=110.00 a=80.00 d=20.00 g=10.00");
    assert_eq!(at(4), "[AXB] w=105.00 h=110.00 a=80.00 d=20.00 g=10.00");

    // An empty string is zero wide and still a full line tall.
    assert_eq!(at(5), "[] w=0.00 h=110.00 a=80.00 d=20.00 g=10.00");

    // Halving the size halves every measurement — the scale is the only thing `size`
    // touches, which is what makes the metrics usable for layout at any size.
    assert_eq!(at(6), "[half] w=27.50");
}

#[test]
fn descent_is_reported_positive_though_the_file_stores_it_negative() {
    // `hhea.descender` in the fixture is -200, and `TextMetrics` documents `descent`
    // as a positive distance below the baseline. Getting this backwards produces a
    // `height` of 60 instead of 110 — a plausible-looking number that lays text out
    // wrong, which is exactly the kind of thing a shipped constant should pin.
    let lines = run_with_font(
        "canvas_measure_descent",
        r#"IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load failed")
    EXIT SUB
  END TRAP
  LET m AS canvas::TextMetrics = canvas::measureText(face, 100.0, "A")
  IF m.descent > 0.0 THEN
    io::print("descent is positive: " & toString(m.descent))
  ELSE
    io::print("descent is not positive: " & toString(m.descent))
  END IF
  io::print("height " & toString(m.height))
END SUB
"#,
    );
    assert_eq!(
        lines.first().map(String::as_str),
        Some("descent is positive: 20.00")
    );
    assert_eq!(lines.get(1).map(String::as_str), Some("height 110.00"));
}

/// Surface dimensions, fixed by `__canvas_surfaceSize`.
const WIDTH: usize = 900;

/// Build a project with the fixture font, render one frame headless, return the pixels.
fn render(name: &str, source: &str) -> Vec<u8> {
    render_with(name, source, false)
}

/// The same, with `MFB_CANVAS_GPU` optionally on.
fn render_with(name: &str, source: &str, gpu: bool) -> Vec<u8> {
    let extra: &[(&str, &str)] = if gpu { &[("MFB_CANVAS_GPU", "1")] } else { &[] };
    let (pixels, stats) = render_env(name, source, extra);
    if gpu && !stats.contains("metalReady=TRUE") {
        // A host with no Metal device is a real configuration, not a failure. The skip
        // gates on the flag the *renderer* gates on, so the test and the runtime can
        // never disagree about whether the GPU path was taken.
        return Vec::new();
    }
    pixels
}

/// Render one frame headless under extra environment, returning the pixels and the
/// `MFB_CANVAS_STATS` text. The stats are the only window onto the caches: they are
/// written by the graphics thread, which is the thread that owns them — a program
/// asking from `main` would be asking the worker, whose copies of those globals are
/// its own and always empty (`.ai/canvas-threading.md` §1).
fn render_env(name: &str, source: &str, extra: &[(&str, &str)]) -> (Vec<u8>, String) {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), minimal_truetype()).expect("write the font");
    let frame = project.join("frame.rgba");
    let binary = common::build_app(&project, name);
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_SYNC", "1");
    for (key, value) in extra {
        command.env(key, value);
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
    let pixels = std::fs::read(&frame).expect("canvas dump written");
    let stats = std::fs::read_to_string(&stats).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    (pixels, stats)
}

/// The last frame's value for a `key=value` field of the stats line.
fn stat(stats: &str, key: &str) -> i64 {
    let last = stats
        .lines()
        .filter(|line| line.contains(&format!("{key}=")))
        .next_back()
        .unwrap_or_else(|| panic!("no stats line carries `{key}=`:\n{stats}"));
    let after = last.split(&format!("{key}=")).nth(1).unwrap();
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse()
        .unwrap_or_else(|e| panic!("`{key}=` is not a number in `{last}`: {e}"))
}

fn pixel(frame: &[u8], x: usize, y: usize) -> (u8, u8, u8, u8) {
    let at = (y * WIDTH + x) * 4;
    (frame[at], frame[at + 1], frame[at + 2], frame[at + 3])
}

#[test]
fn a_glyph_outline_renders_where_its_own_coordinates_put_it() {
    // Glyph 1 is the square `(100,0)..(400,300)` in font units. At `unitsPerEm` 1000
    // and `size` 100 the scale is exactly 0.1, so with the pen at x=100 and the
    // **baseline** at y=200 the ink is `x` 110..140 and `y` 170..200 — the Y flip is
    // the whole reason the top edge is the *smaller* number, since a font's Y grows
    // upward and the surface's grows down.
    let frame = render(
        "canvas_glyph_render",
        r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[x := 100.0, y := 200.0, text := "A", font := canvas::fontRef(face), size := 100.0, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([label])
END SUB
"#,
    );

    let lit: Vec<(usize, usize)> = (0..frame.len() / 4)
        .filter(|i| frame[i * 4] != 0 || frame[i * 4 + 1] != 0 || frame[i * 4 + 2] != 0)
        .map(|i| (i % WIDTH, i / WIDTH))
        .collect();
    assert!(!lit.is_empty(), "the glyph drew nothing at all");
    let min_x = lit.iter().map(|p| p.0).min().unwrap();
    let max_x = lit.iter().map(|p| p.0).max().unwrap();
    let min_y = lit.iter().map(|p| p.1).min().unwrap();
    let max_y = lit.iter().map(|p| p.1).max().unwrap();

    // A pixel is lit when its *centre* is inside the shape, so the covered centres run
    // 110.5..139.5 and 170.5..199.5 — the last whole pixels inside the 30x30 square.
    assert_eq!(
        (min_x, max_x, min_y, max_y),
        (110, 139, 170, 199),
        "glyph ink is not where its own coordinates put it",
    );
    assert_eq!(lit.len(), 30 * 30, "the square is not solid");
    assert_eq!(pixel(&frame, 125, 185), (255, 255, 255, 255), "inside");
    assert_eq!(pixel(&frame, 105, 185), (0, 0, 0, 255), "left of it");
    assert_eq!(pixel(&frame, 125, 160), (0, 0, 0, 255), "above it");
    assert_eq!(
        pixel(&frame, 125, 210),
        (0, 0, 0, 255),
        "below the baseline"
    );
}

#[test]
fn a_string_advances_the_pen_between_glyphs() {
    // "AA" is the same outline twice, one advance apart. Glyph 1's advance is 250
    // font units = 25 px, so the second square starts 25 px right of the first and the
    // two overlap into one 55-px-wide band — which is what a *monospaced* run looks
    // like when the glyph is wider than its advance, and is exactly the arithmetic the
    // pen has to get right.
    let frame = render(
        "canvas_glyph_advance",
        r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[x := 100.0, y := 200.0, text := "AA", font := canvas::fontRef(face), size := 100.0, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([label])
END SUB
"#,
    );
    let lit: Vec<usize> = (0..frame.len() / 4)
        .filter(|i| frame[i * 4] != 0)
        .map(|i| i % WIDTH)
        .collect();
    assert!(!lit.is_empty(), "the string drew nothing at all");
    assert_eq!(
        (*lit.iter().min().unwrap(), *lit.iter().max().unwrap()),
        (110, 164),
        "the second glyph is not one advance right of the first",
    );
}

#[test]
fn text_in_a_font_that_was_never_loaded_draws_nothing() {
    // A `FontRef` a program fabricated, or one whose font it released — the runtime
    // draws empty rather than following a handle it cannot resolve. This is the
    // property that lets `canvas::destroyFont` be safe while a scene still names the
    // font, so it is worth pinning separately from the happy path.
    let frame = render(
        "canvas_glyph_no_font",
        r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET label AS canvas::DrawItem = canvas::Text[x := 100.0, y := 200.0, text := "A", font := canvas::FontRef[id := 12345], size := 100.0, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([label])
END SUB
"#,
    );
    assert!(
        frame.chunks(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0),
        "text in an unresolvable font drew something",
    );
}

/// A text scene the GPU draws — four glyphs, at a size whose bitmaps fit Metal's
/// per-glyph payload.
///
/// Since the glyph cache landed, a glyph reaches the GPU as a *coverage bitmap* rather
/// than as flattened edges, so the old `MAX_EDGES` cap no longer decides whether text is
/// GPU-drawable. What decides it now is bitmap size: Metal carries one glyph's coverage
/// in a `setFragmentBytes:` payload, capped at 4 KiB — about 64x64 — and Vulkan carries
/// the whole frame's in a buffer region. At size 120 the fixture's square is 36x36, well
/// inside both.
const GPU_TEXT: &str = r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[x := 100.0, y := 300.0, text := "AAAA", font := canvas::fontRef(face), size := 120.0, paint := canvas::fill(canvas::rgb(220, 40, 160))]
  canvas::present([label])
END SUB
"#;

#[test]
fn text_on_the_gpu_matches_the_software_oracle() {
    let software = render_with("canvas_text_gpu_sw", GPU_TEXT, false);
    let gpu = render_with("canvas_text_gpu_hw", GPU_TEXT, true);
    if gpu.is_empty() {
        eprintln!("skip: this host reports no Metal device");
        return;
    }
    assert!(
        software.iter().any(|&b| b != 0),
        "the software render drew nothing, so the comparison would be vacuous",
    );

    let height = (software.len() / 4 / WIDTH) as u32;
    let want = Frame {
        width: WIDTH as u32,
        height,
        pixels: software,
    };
    let got = Frame {
        width: WIDTH as u32,
        height,
        pixels: gpu,
    };
    // A tolerance rather than exact equality, for the same reason every other GPU
    // comparison uses one: the shader composites in linear space with hardware blending
    // and the oracle blends in sRGB, so antialiased edges land within a step or two of
    // each other rather than on the same value.
    //
    // What this catches is the failure that is otherwise silent. Before the backends
    // could draw a glyph, Metal accepted a scene whose kind its shader did not know and
    // returned a frame with the text simply missing — 4,536 pixels wrong, reported as
    // success. A GPU frame that merely *resembles* the oracle is the same lie in a
    // quieter form, which is why this compares pixels rather than checking that the
    // renderer claimed the GPU.
    if let Err(diff) = compare_within_tolerance(&got, &want, Tolerance::GPU_DEFAULT) {
        panic!("GPU text differs from the software oracle: {diff:?}");
    }
}

/// Shapes drawn **after** a glyph run land in the right place, and the glyphs are drawn
/// exactly once.
///
/// plan-116-A made consecutive non-text items one instanced draw. A glyph run ends that
/// run and takes item-buffer slots of its own, so the next run has to restart past them.
/// If it does not, the trailing shapes are drawn as one run *beginning at the first
/// glyph* — and every glyph quad is drawn a second time.
///
/// **The label is translucent, and that is the whole reason this test can see the bug.**
/// The fixture glyph is an axis-aligned opaque square, so its coverage is binary, and
/// compositing an opaque square over itself is idempotent: with an opaque label a
/// renderer that drew every glyph twice produced a byte-identical frame and no assertion
/// could tell. At alpha 150 a second composite is arithmetically different from one.
/// (Measured on the Vulkan harness, which is where this was first caught: bug present →
/// worst channel delta 27; bug absent → 1.)
///
/// The trailing circle is what forces a non-empty final run — with the text last, the
/// final flush is always empty and the ordering is never exercised.
const GPU_TEXT_THEN_SHAPE: &str = r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET under AS canvas::DrawItem = canvas::Rectangle[x := 80.0, y := 150.0, w := 300.0, h := 200.0, paint := canvas::fill(canvas::rgb(40, 60, 90))]
  LET label AS canvas::DrawItem = canvas::Text[x := 100.0, y := 300.0, text := "AAAA", font := canvas::fontRef(face), size := 120.0, paint := canvas::fill(canvas::rgba(220, 40, 160, 150))]
  LET tail AS canvas::DrawItem = canvas::Circle[x := 600.0, y := 200.0, radius := 60.0, paint := canvas::fill(canvas::rgb(120, 220, 60))]
  canvas::present([under, label, tail])
END SUB
"#;

#[test]
fn a_shape_after_a_glyph_run_matches_the_software_oracle() {
    let software = render_with("canvas_text_tail_sw", GPU_TEXT_THEN_SHAPE, false);
    let gpu = render_with("canvas_text_tail_hw", GPU_TEXT_THEN_SHAPE, true);
    if gpu.is_empty() {
        eprintln!("skip: this host reports no Metal device");
        return;
    }
    assert!(
        software.iter().any(|&b| b != 0),
        "the software render drew nothing, so the comparison would be vacuous",
    );

    let height = (software.len() / 4 / WIDTH) as u32;
    let want = Frame {
        width: WIDTH as u32,
        height,
        pixels: software,
    };
    let got = Frame {
        width: WIDTH as u32,
        height,
        pixels: gpu,
    };
    if let Err(diff) = compare_within_tolerance(&got, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "a scene of shape, translucent text, shape differs from the software \
             oracle: {diff:?}\n\
             A difference concentrated in the text band means the glyph quads were \
             drawn twice — the instanced run after the glyphs restarted at the wrong \
             base. A difference at the trailing circle means it was not drawn at all."
        );
    }
}

/// Two scenes of three hundred text items each, presented A, B, A.
///
/// Every size is a distinct glyph-cache key (`__canvas_sizeQ` quantises to 1/16 px and
/// the step here is 0.2) and the two scenes' size ranges are disjoint, so the program
/// asks for six hundred distinct glyphs while never needing more than three hundred at
/// once. That is what makes eviction *possible*: a glyph is pinned while any live
/// geometry entry references it, and while the frame being rendered holds its offset —
/// so the only glyphs that can ever be dropped are ones no scene on screen still names.
/// Scene B is what makes scene A's glyphs droppable, and the second A is what proves
/// they come back identical.
///
/// The fixture glyph is a square with four edges, so six hundred rasterisations are
/// cheap while the bitmaps — 6x6 up to 36x36 — add up to far more than the forced budget.
const EVICTION: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections

FUNC scene(face AS canvas::Font, base AS Float) AS List OF canvas::DrawItem
  LET white AS canvas::Paint = canvas::fill(canvas::rgb(255, 255, 255))
  MUT items AS List OF canvas::DrawItem = []
  MUT i AS Integer = 0
  WHILE i < 300
    LET size AS Float = base + toFloat(i) * 0.2
    LET x AS Float = 4.0 + toFloat(i MOD 20) * 45.0
    LET y AS Float = 36.0 + toFloat(i / 20) * 40.0
    LET glyph AS canvas::DrawItem = canvas::Text[x := x, y := y, text := "A", font := canvas::fontRef(face), size := size, paint := white]
    items = collections::append(items, glyph)
    i = i + 1
  END WHILE
  RETURN items
END FUNC

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET a AS List OF canvas::DrawItem = scene(face, 20.0)
  LET b AS List OF canvas::DrawItem = scene(face, 80.0)
  canvas::present(a)
  canvas::present(b)
  ' The frame that gets dumped, and the one that is compared: scene A again, after
  ' scene B has had the chance to push its glyphs out of the cache.
  canvas::present(a)
END SUB
"#;

#[test]
fn eviction_frees_unpinned_glyphs_and_changes_no_pixel() {
    // Same program, same scene, two budgets. The roomy run never evicts, so it is the
    // oracle; the forced run cannot hold the scene and has to drop and re-raster.
    let (thrashed, pressed) = render_env(
        "canvas_glyph_eviction",
        EVICTION,
        &[("MFB_CANVAS_GLYPH_BUDGET", "8192")],
    );
    let (roomy, relaxed) = render_env("canvas_glyph_roomy", EVICTION, &[]);

    // The pressure is real, and it is *only* pressure — the default budget holds the
    // whole scene, which is what makes eviction a cache behaviour rather than something
    // ordinary text runs into.
    assert!(
        stat(&pressed, "glyphEvictions") > 0,
        "the forced budget never evicted:\n{pressed}",
    );
    assert_eq!(
        stat(&relaxed, "glyphEvictions"),
        0,
        "a 300-item scene evicted at the default budget:\n{relaxed}",
    );

    // Entries really left the cache. Without this the test would still pass if
    // `__canvas_glyphEvict` compacted the cache while keeping every entry — which is
    // exactly what it does when everything is pinned, and is not eviction.
    //
    // The roomy run ends holding all six hundred glyphs; the pressured one cannot, and
    // the ones it dropped are scene B's, which nothing on screen still names.
    let kept = stat(&pressed, "glyphs");
    let all = stat(&relaxed, "glyphs");
    assert_eq!(
        all, 600,
        "the roomy run should still hold both scenes' glyphs",
    );
    assert!(
        kept < all,
        "eviction dropped nothing: {kept} entries under pressure, {all} without",
    );

    // And the pixels are identical. This is the whole claim: a glyph the live scene
    // still draws is never the one dropped, and one that *was* dropped re-rasters to
    // the same bitmap it had before. Exact, not tolerant — a single wrong pixel here
    // means a glyph went missing or came back different, and both are silent failures
    // in a cache that reports nothing.
    let height = (roomy.len() / 4 / WIDTH) as u32;
    let got = Frame::from_rgba(WIDTH as u32, height, thrashed);
    let want = Frame::from_rgba(WIDTH as u32, height, roomy);
    assert!(
        want.pixels.iter().any(|&b| b != 0),
        "the oracle frame is blank, so the comparison would be vacuous",
    );
    if let Err(diff) = compare_exact(&got, &want) {
        panic!("a frame drawn under cache pressure differs from the same frame drawn without it: {diff:?}");
    }
}

/// A rotated text run draws rotated, and where the matrix says.
///
/// plan-116-C §4.5. A glyph's pixels come from a cached coverage bitmap, and under a
/// transform the blit has to invert: walk the surface region the glyph now covers and
/// sample the bitmap, rather than walk the bitmap and write surface pixels. Walking the
/// bitmap under a rotation leaves holes, because the mapping is no longer one sample
/// per pixel.
///
/// The fixture glyph is a solid square, which makes this checkable without judging an
/// antialiased edge: a 90° rotation of a horizontal row of squares is a *vertical*
/// column of squares, so "is there ink here" is a whole-pixel question.
///
/// 90° exactly, so nearest sampling is exact too — the point here is the inverted loop
/// and the transformed per-glyph bounds, not the sampling filter.
const ROTATED_TEXT: &str = r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET t AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 500.0, ty := 100.0]
  LET p AS canvas::Paint = WITH canvas::fill(canvas::rgb(255, 255, 255)) { transform := t }
  LET label AS canvas::DrawItem = canvas::Text[x := 40.0, y := 60.0, text := "AAAA", font := canvas::fontRef(face), size := 60.0, paint := p]
  canvas::present([label])
END SUB
"#;

/// The same run with no transform, so the two can be compared as pictures.
const UPRIGHT_TEXT: &str = r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[x := 40.0, y := 60.0, text := "AAAA", font := canvas::fontRef(face), size := 60.0, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([label])
END SUB
"#;

#[test]
fn a_rotated_text_run_draws_rotated() {
    let upright = render_with("canvas_text_upright", UPRIGHT_TEXT, false);
    let rotated = render_with("canvas_text_rotated", ROTATED_TEXT, false);
    assert!(
        upright.iter().any(|&b| b != 0),
        "the upright run drew nothing, so the comparison would be vacuous"
    );

    let lit = |frame: &[u8], x: usize, y: usize| {
        frame[(y * WIDTH + x) * 4 + 3] != 0 && frame[(y * WIDTH + x) * 4] != 0
    };
    // Ink in the upright run's own band, which the rotated one must have vacated.
    let mut upright_ink = 0usize;
    let mut rotated_there = 0usize;
    for y in 20..60 {
        for x in 40..240 {
            if lit(&upright, x, y) {
                upright_ink += 1;
            }
            if lit(&rotated, x, y) {
                rotated_there += 1;
            }
        }
    }
    assert!(
        upright_ink > 500,
        "the upright run should fill its band; found {upright_ink} lit pixels"
    );
    assert_eq!(
        rotated_there, 0,
        "the rotated run must have left the upright band entirely — {rotated_there} \
         pixels of it are still lit, so the transform did not move the glyphs"
    );

    // And it must be SOMEWHERE: a transform that dropped the run would also vacate
    // the band, so count the rotated run's own ink and check it matches.
    let total = |frame: &[u8]| {
        (0..WIDTH * (frame.len() / 4 / WIDTH))
            .filter(|i| frame[i * 4 + 3] != 0 && frame[i * 4] != 0)
            .count()
    };
    let (up, rot) = (total(&upright), total(&rotated));
    assert!(
        rot > 0,
        "the rotated run drew nothing at all — the inverted blit found no samples"
    );
    // A 90° rotation is area-preserving, and the glyph is a solid square, so the ink
    // count should match closely. Allow a small margin for the boundary pixels that
    // nearest sampling rounds differently.
    let diff = (up as i64 - rot as i64).abs();
    assert!(
        diff * 20 < up as i64,
        "a 90-degree rotation preserves area, so the rotated run should have about as \
         much ink as the upright one: upright {up}, rotated {rot}"
    );
}

/// A transformed text run reaches the GPU, and draws there what the oracle draws.
///
/// Two claims in one render, because they fail together and are cheap to separate only
/// here:
///
/// 1. **Neither `*Renderable` predicate declines a transform.** `gpuSelected=TRUE` is
///    the observable for that — the renderer sets it only after the predicate let the
///    scene through, so a predicate that grew a `hasTransform` bail would flip the
///    scene to software and this assertion, not the pixel comparison, is what would
///    fail. That distinction is the point: falling back to software still produces a
///    *correct* picture, so the comparison below would pass while the GPU path went
///    entirely unexercised.
/// 2. **The glyph fragment path's inverse sample matches the oracle's.** A transformed
///    run is the one case where the per-glyph quad cannot be narrowed to the glyph's
///    own box (see the `_glyph_hull` block in `runtime/canvas/vulkan.rs`), so this is
///    also the test that the run's transformed hull is what the vertex stage expands.
#[test]
fn a_transformed_text_run_reaches_the_gpu_and_matches_the_oracle() {
    let software = render_with("canvas_text_xform_sw", ROTATED_TEXT, false);
    let (gpu, stats) = render_env(
        "canvas_text_xform_hw",
        ROTATED_TEXT,
        &[("MFB_CANVAS_GPU", "1")],
    );
    if !stats.contains("metalReady=TRUE") {
        eprintln!("skip: this host reports no Metal device");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "a scene whose only item carries a transform was refused by the renderable \
         predicate and fell back to software. The transform slots sit past the header \
         fields the predicates read, so nothing in them should reach either: {stats}"
    );
    assert!(
        software.iter().any(|&b| b != 0),
        "the software render of the rotated run drew nothing, so the comparison would \
         be vacuous"
    );

    let height = (software.len() / 4 / WIDTH) as u32;
    let want = Frame {
        width: WIDTH as u32,
        height,
        pixels: software,
    };
    let got = Frame {
        width: WIDTH as u32,
        height,
        pixels: gpu,
    };
    if let Err(diff) = compare_within_tolerance(&got, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's transformed glyph run disagrees with the oracle: {diff}\n\
             The inverse sample is the suspect: the shader maps the fragment back \
             through the run's inverse and reads the cached coverage there, so a \
             whole-run offset is a wrong translation, and a run that came out sheared \
             or mirrored is a transposed matrix."
        );
    }
}

// --- font-derived size bombs (bug-509, DEC-53/54) --------------------------------------
//
// A TrueType file names sizes the renderer used to trust: `cmap` format 12's
// `numGroups` drove an unbounded scan, and `head.unitsPerEm` is the divisor of every
// scale, so a two-byte edit made one letter a gigapixel bitmap. Each is now bounded
// by what the file can actually hold or the format actually allows. A bomb's failure
// mode is "still running", so these runs carry a deadline and the deadline is the
// failure — `Command::output` would wait as long as the bomb lasts.

use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant};

struct FontRun {
    lines: Vec<String>,
    /// The dumped frame, when `dump` was asked for; empty otherwise.
    frame: Vec<u8>,
}

/// Build `source` with the given font files beside it, run it headless under a
/// deadline, and return what it printed (and drew). Stdout is read after exit; these
/// programs print a handful of lines.
fn run_fonts_bounded(
    name: &str,
    source: &str,
    fonts: &[(&str, Vec<u8>)],
    dump: bool,
    timeout: Duration,
) -> FontRun {
    let project = common::temp_project(name, source);
    for (file, bytes) in fonts {
        std::fs::write(project.join(file), bytes).expect("write a font fixture");
    }
    let frame = project.join("frame.rgba");
    let binary = common::build_app(&project, name);
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if dump {
        command.env("MFB_CANVAS_DUMP", &frame);
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&project);
            panic!(
                "{name}: still running after {timeout:?} — a font-derived size the \
                 renderer did not bound"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout).ok();
    }
    assert!(
        status.success(),
        "{name}: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let frame = if dump {
        std::fs::read(&frame).expect("canvas dump written")
    } else {
        Vec::new()
    };
    let _ = std::fs::remove_dir_all(&project);
    FontRun {
        lines: stdout.lines().map(str::to_string).collect(),
        frame,
    }
}

/// Draw one `A` from `fixture.ttf` with the pen at (100, 600) and the given size.
fn draw_a(size: &str) -> String {
    format!(
        r#"IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load failed " & toString(e.code))
    EXIT SUB
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[x := 100.0, y := 600.0, text := "A", font := canvas::fontRef(face), size := {size}, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([label])
  io::print("presented")
END SUB
"#
    )
}

fn lit_pixels(frame: &[u8]) -> usize {
    (0..frame.len() / 4)
        .filter(|i| frame[i * 4] != 0 || frame[i * 4 + 1] != 0 || frame[i * 4 + 2] != 0)
        .count()
}

#[test]
fn a_cmap_group_count_is_bounded_by_the_table_that_holds_it() {
    // DEC-54. Format 12's `numGroups` is a u32 the file controls; this one claims
    // 4,294,967,295 groups in a 40-byte subtable that holds two. The lookup scanned
    // every claimed group — 583 s of CPU to map one unmapped character — where the
    // subtable's own `length` and the end of the file both said to stop at two. The
    // metrics are the ones `measure_text_scales_the_fonts_own_metrics` pins: bounding
    // the scan changes nothing about a lookup the table can answer.
    let run = run_fonts_bounded(
        "canvas_cmap_bomb",
        MEASURE,
        &[(
            "fixture.ttf",
            truetype_fixture(1000, u32::MAX, [100, 0, 400, 300]),
        )],
        false,
        Duration::from_secs(30),
    );
    let at = |i: usize| run.lines.get(i).cloned().unwrap_or_default();
    assert_eq!(at(0), "[A] w=25.00 h=110.00 a=80.00 d=20.00 g=10.00");
    assert_eq!(at(3), "[X] w=50.00 h=110.00 a=80.00 d=20.00 g=10.00");
    assert_eq!(at(4), "[AXB] w=105.00 h=110.00 a=80.00 d=20.00 g=10.00");
    assert_eq!(at(6), "[half] w=27.50");
}

#[test]
fn a_units_per_em_outside_the_formats_range_is_refused_at_load() {
    // DEC-53, the file half. `head.unitsPerEm` divides into every scale: at 1, the
    // 300-unit square at size 100 is 30,000 px a side and its bitmap is 900 megapixels
    // (measured 62 s and 7.6 GB for one letter). The format allows 16..16384 and
    // nothing else, FreeType refuses the file outside that range, and so does
    // `loadFont` — as `ErrBadFontFile`, the code for "a file this build cannot read".
    // A file with no `head` at all stays accepted: it has no scale to poison.
    const SOURCE: &str = r#"IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  FOR EACH name IN ["upem1", "upem0", "upem15", "upem16385", "upem16", "upem16384", "upem1000"]
    RES f AS canvas::Font = canvas::loadFont(name & ".ttf") TRAP(e)
      io::print(name & ": refused " & toString(e.code))
      CONTINUE FOR
    END TRAP
    canvas::destroyFont(f)
    io::print(name & ": accepted")
  NEXT
END SUB
"#;
    let square = [100, 0, 400, 300];
    let fonts: Vec<(&str, Vec<u8>)> = [
        ("upem1.ttf", 1u16),
        ("upem0.ttf", 0),
        ("upem15.ttf", 15),
        ("upem16385.ttf", 16385),
        ("upem16.ttf", 16),
        ("upem16384.ttf", 16384),
        ("upem1000.ttf", 1000),
    ]
    .into_iter()
    .map(|(file, upem)| (file, truetype_fixture(upem, 2, square)))
    .collect();
    let run = run_fonts_bounded(
        "canvas_upem_range",
        SOURCE,
        &fonts,
        false,
        Duration::from_secs(30),
    );
    let find = |label: &str| -> String {
        run.lines
            .iter()
            .find(|l| l.starts_with(&format!("{label}:")))
            .unwrap_or_else(|| panic!("no `{label}` line in {:?}", run.lines))
            .clone()
    };
    for label in ["upem1", "upem0", "upem15", "upem16385"] {
        assert_eq!(
            find(label),
            format!("{label}: refused {ERR_BAD_FONT_FILE}"),
            "a unitsPerEm outside 16..16384 must be refused as a bad font file",
        );
    }
    for label in ["upem16", "upem16384", "upem1000"] {
        assert_eq!(find(label), format!("{label}: accepted"));
    }
}

#[test]
fn a_glyph_whose_bitmap_would_exceed_the_raster_cap_is_skipped() {
    // DEC-53, the outline half. A legal `unitsPerEm` of 16 with a glyph spanning
    // 30,000 units — coordinates are int16, so a file may say so — is 375,000 px a
    // side at size 200: 1.4e11 coverage bytes for one glyph. Past the cap the glyph is
    // recorded empty and draws nothing. It cannot raise: the rasteriser runs on the
    // graphics thread, where a raise is a hang (`helper_glyph_cache.rs`, Correction 13).
    let run = run_fonts_bounded(
        "canvas_glyph_bomb",
        &draw_a("200.0"),
        &[("fixture.ttf", truetype_fixture(16, 2, [0, 0, 30000, 30000]))],
        true,
        Duration::from_secs(30),
    );
    assert_eq!(run.lines.last().map(String::as_str), Some("presented"));
    assert_eq!(
        lit_pixels(&run.frame),
        0,
        "a glyph past the raster cap must draw nothing, not a partial bitmap",
    );
}

#[test]
fn a_display_sized_glyph_is_well_inside_the_raster_cap() {
    // The cap must not touch real text. At `unitsPerEm` 1000 a 1000-unit square at
    // size 1500 is a 1500x1500 bitmap — larger than any glyph a 4K display can show
    // whole — and it still rasterises: the ink runs from the pen at x=100 to the right
    // edge of the 900-wide surface, and from the top edge down to the baseline at 600.
    let run = run_fonts_bounded(
        "canvas_glyph_display_size",
        &draw_a("1500.0"),
        &[("fixture.ttf", truetype_fixture(1000, 2, [0, 0, 1000, 1000]))],
        true,
        Duration::from_secs(120),
    );
    assert_eq!(run.lines.last().map(String::as_str), Some("presented"));
    assert_eq!(pixel(&run.frame, 500, 300), (255, 255, 255, 255), "inside");
    assert_eq!(
        pixel(&run.frame, 101, 599),
        (255, 255, 255, 255),
        "pen corner"
    );
    assert_eq!(
        pixel(&run.frame, 50, 300),
        (0, 0, 0, 255),
        "left of the pen"
    );
    assert_eq!(
        pixel(&run.frame, 500, 620),
        (0, 0, 0, 255),
        "below the baseline"
    );
    assert_eq!(
        lit_pixels(&run.frame),
        800 * 600,
        "the visible part of the square is solid"
    );
}
