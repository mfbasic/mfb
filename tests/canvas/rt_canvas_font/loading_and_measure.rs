//! plan-176-D: `rt_canvas_font`'s loading, measuring and bounds tests, and font
//! collections. The fixture builder, the render family and the sfnt table builders live
//! in the root, `rt_canvas_font.rs`.

use super::*;

/// Writes a 12-byte sfnt header with the given first four bytes, then loads it.
///
/// Twelve bytes because that is the whole table-directory header — version, table
/// count, and the three binary-search fields — and `loadFont`'s length guard rejects
/// anything shorter, so a four-byte file would be refused for the wrong reason and the
/// test would pass without testing the version check at all.
const PROGRAM: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
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
  ' The old form minted a `FontRef` and asserted a non-zero id. That checked one thing:
  ' the mint step succeeded. plan-116-I removed the mint, so there is no second
  ' assertion left to make here — whether the loader ACCEPTS a container, which is what
  ' this function measures, is decided entirely by the TRAP above.
  '
  ' Deliberately not replaced with a `measureText` check: this fixture's faces are
  ' synthetic containers built to exercise the loader's accept/refuse decision, and
  ' several measure zero for reasons that have nothing to do with acceptance. Asserting
  ' on their metrics would fail for the wrong reason, which is worse than not asserting.
  canvas::destroyFont(f)
  RETURN label & ": accepted"
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  io::print(attempt("truetype", 0, 1, 0, 0))
  io::print(attempt("appletrue", 116, 114, 117, 101))
  io::print(attempt("otto", 79, 84, 84, 79))
  io::print(attempt("ttcf", 116, 116, 99, 102))
  io::print(attempt("woff", 119, 79, 70, 70))
  MUT short AS List OF Byte = [toByte(0), toByte(1), toByte(0), toByte(0)]
  fs::writeBytes("short.bin", short) TRAP(e)
    RETURN
  END TRAP
  RES g AS canvas::Font = canvas::loadFont("short.bin") TRAP(e)
    io::print("short: refused " & toString(e.code))
    RETURN
  END TRAP
  io::print("short: accepted a four-byte file")
END FUNC
"#;

/// `ErrNotFound` (`7-705-0004`) — a face name the file does not carry (plan-148-A).
const ERR_FACE_NOT_FOUND: &str = "77050004";

/// `ErrPathNotFound` — the *other* answer `loadFont` can give, and the reason
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
    // `wOFF` is compressed, and this `ttcf` is a collection header that names no face
    // (a collection that does is read — see `a_collection_loads_its_first_face`).
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
IMPORT vector
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("no-such-font.ttf") TRAP(e)
    io::print("missing: " & toString(e.code))
    RETURN
  END TRAP
  io::print("missing: loaded a font that does not exist")
END FUNC
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

/// bug-610: a second explicit `destroyFont` raises `ErrResourceClosed`, exactly as
/// `fs::close` does (`mfb spec language resource-management` §15). It used to return
/// silently.
///
/// The second close goes through a resource parameter because a literal
/// `destroyFont(f)` twice is refused at compile time (`TYPE_USE_AFTER_MOVE`). The
/// scope-drop of `f` after both closes must stay silent, which the `done` line
/// confirms.
#[test]
fn a_second_destroy_font_raises_resource_closed() {
    let lines = run_with_font(
        "canvas_font_double_close",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT io

FUNC closeIt(f AS canvas::Font) AS Nothing
  canvas::destroyFont(f)
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("fixture.ttf")
  closeIt(f)
  closeIt(f) TRAP(e)
    io::print("second: " & toString(e.code))
    RECOVER
  END TRAP
  io::print("done")
END FUNC
"#,
    );
    assert_eq!(
        lines,
        vec![format!("second: {RESOURCE_CLOSED}"), "done".to_string()],
        "a second destroyFont must raise ErrResourceClosed, and the drop after it \
         must stay silent",
    );
}

const MEASURE: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load failed " & toString(e.code))
    RETURN
  END TRAP
  FOR EACH s IN ["A", "B", "AB", "X", "AXB", ""]
    LET m AS canvas::TextMetrics = canvas::measureText(face, 100.0, s)
    io::print("[" & s & "] w=" & toString(m.width) & " h=" & toString(m.height) & " a=" & toString(m.ascent) & " d=" & toString(m.descent) & " g=" & toString(m.lineGap))
  NEXT
  LET half AS canvas::TextMetrics = canvas::measureText(face, 50.0, "AB")
  io::print("[half] w=" & toString(half.width))
END FUNC
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
IMPORT vector
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load failed")
    RETURN
  END TRAP
  LET m AS canvas::TextMetrics = canvas::measureText(face, 100.0, "A")
  IF m.descent > 0.0 THEN
    io::print("descent is positive: " & toString(m.descent))
  ELSE
    io::print("descent is not positive: " & toString(m.descent))
  END IF
  io::print("height " & toString(m.height))
END FUNC
"#,
    );
    assert_eq!(
        lines.first().map(String::as_str),
        Some("descent is positive: 20.00")
    );
    assert_eq!(lines.get(1).map(String::as_str), Some("height 110.00"));
}

/// `measureText` sums advances as **Integers** in font units and scales once. At size
/// 35 that differs in the last bit from adding the scaled advances glyph by glyph,
/// whether or not the target fuses the multiply-add (35 was chosen because both
/// orders of rounding separate there), so the comparison is exact and only the
/// Integer-sum form passes. The second printed line proves the size still separates.
#[test]
fn measure_text_sums_a_mixed_run_in_font_units_then_scales_once() {
    let lines = run_with_font(
        "canvas_measure_walk",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load failed")
    RETURN
  END TRAP
  LET m AS canvas::TextMetrics = canvas::measureText(face, 35.0, "ABXA")
  LET scale AS Float = 35.0 / toFloat(1000)
  LET summed AS Float = toFloat(250 + 300 + 500 + 250) * scale
  MUT stepped AS Float = 0.0
  FOR EACH adv IN [250, 300, 500, 250]
    stepped = stepped + toFloat(adv) * scale
  NEXT
  io::print("integer-sum " & toString(m.width = summed))
  io::print("forms differ " & toString(summed <> stepped))
END FUNC
"#,
    );
    assert_eq!(
        lines,
        vec![
            "integer-sum TRUE".to_string(),
            "forms differ TRUE".to_string()
        ],
        "measureText must sum the run's advances in font units and scale once; the \
         second line proves this size can tell that apart from a per-glyph sum",
    );
}

/// A closed font still raises `ErrResourceClosed` from `measureText` — the man page
/// says so — rather than measuring as zeroes the way a font with no `head` table does.
#[test]
fn measuring_a_closed_font_raises_resource_closed() {
    let lines = run_with_font(
        "canvas_measure_closed",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT io

FUNC closeIt(f AS canvas::Font) AS Nothing
  canvas::destroyFont(f)
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("fixture.ttf")
  closeIt(f)
  LET m AS canvas::TextMetrics = canvas::measureText(f, 100.0, "A") TRAP(e)
    io::print("closed: " & toString(e.code))
    RETURN
  END TRAP
  io::print("measured " & toString(m.width))
END FUNC
"#,
    );
    assert_eq!(lines, vec![format!("closed: {RESOURCE_CLOSED}")]);
}

/// The renderer's font table holds sixteen fonts, and a full table is documented as
/// not an error: a seventeenth font still **measures**, and only text drawn in it comes
/// out empty (`gen_font_table.rs:emit_register_font`). So measuring must not depend on
/// the font having a table slot.
#[test]
fn a_font_past_the_renderers_table_still_measures() {
    let mut source = String::from(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT io\n\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n",
    );
    for i in 1..=17 {
        source.push_str(&format!(
            "  RES f{i} AS canvas::Font = canvas::loadFont(\"fixture.ttf\")\n"
        ));
    }
    source.push_str(
        "  io::print(\"first \" & toString(canvas::measureText(f1, 100.0, \"AB\").width))\n  io::print(\"last \" & toString(canvas::measureText(f17, 100.0, \"AB\").width))\nEND FUNC\n",
    );
    let lines = run_with_font("canvas_measure_seventeenth", &source);
    assert_eq!(
        lines,
        vec!["first 55.00".to_string(), "last 55.00".to_string()],
        "the seventeenth font has no renderer table slot, and must measure anyway",
    );
}

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
    let binary = common::build_app_debug(&project, name);
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
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
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load failed " & toString(e.code))
    RETURN
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 600.0], text := "A", fonts := [face], fontSize := {size}, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([label])
  io::print("presented")
END FUNC
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
    // DEC-54, the wrong-answer half. Format 12's `numGroups` is a u32 the file
    // controls; this one claims 4,294,967,295 groups in a 40-byte subtable that holds
    // two, and `cmap` is the first table in the file. Unbounded, the scan ran on past
    // the subtable into `glyf` and `head`, and one of those twelve-byte windows
    // happened to contain `X`: it mapped to glyph 2 and measured 30.00 instead of the
    // `.notdef` 50.00 (measured on the unfixed build). The subtable's own `length`
    // says to stop at two, and with the scan bounded by it the metrics are the ones
    // `measure_text_scales_the_fonts_own_metrics` pins — bounding the scan changes
    // nothing about a lookup the table can answer. The hang half is the test below.
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

/// `font`, with its format-12 subtable copied to the end of the file and both the
/// copy's `length` and `numGroups` set to `u32::MAX`. This is the audit's own DEC-54
/// shape — `spikes/audit-3/gen/cmap12.py` appends the subtable at EOF — with the
/// length made hostile too, so the subtable's own length bounds nothing and the end
/// of the file is the only bound left.
fn cmap_subtable_at_eof_claiming_everything(font: &[u8]) -> Vec<u8> {
    let be32 = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().unwrap()) as usize;
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    let cmap = (0..tables)
        .map(|i| 12 + i * 16)
        .find(|rec| &font[*rec..rec + 4] == b"cmap")
        .map(|rec| be32(rec + 8))
        .expect("the fixture has a cmap table");
    // Encoding record 0's subtable offset is at cmap+8, relative to the table start.
    let subtable = cmap + be32(cmap + 8);
    let mut moved = font[subtable..subtable + 40].to_vec();
    moved[4..8].copy_from_slice(&u32::MAX.to_be_bytes()); // length
    moved[12..16].copy_from_slice(&u32::MAX.to_be_bytes()); // numGroups
    let mut out = font.to_vec();
    let at = out.len();
    out[cmap + 8..cmap + 12].copy_from_slice(&((at - cmap) as u32).to_be_bytes());
    out.extend_from_slice(&moved);
    out
}

#[test]
fn a_cmap_group_count_past_the_end_of_the_file_is_bounded_by_the_file() {
    // DEC-54, the hang half. The subtable sits at the end of the file, so past its two
    // real groups every twelve-byte window reads as zeros through `getOr` and matches
    // nothing but U+0000. Unbounded, mapping one unmapped character was a scan of
    // 4,294,967,295 such groups — 583 s of CPU in the audit, and this run hits its
    // deadline. The subtable's `length` lies here as well, so the bound that has to
    // hold is the file's end: two groups fit before it, and the metrics are exactly
    // the fixture's — including `X`, which stays `.notdef` because nothing past the
    // file can match it.
    let run = run_fonts_bounded(
        "canvas_cmap_eof_bomb",
        MEASURE,
        &[(
            "fixture.ttf",
            cmap_subtable_at_eof_claiming_everything(&minimal_truetype()),
        )],
        false,
        Duration::from_secs(30),
    );
    let at = |i: usize| run.lines.get(i).cloned().unwrap_or_default();
    assert_eq!(at(0), "[A] w=25.00 h=110.00 a=80.00 d=20.00 g=10.00");
    assert_eq!(at(1), "[B] w=30.00 h=110.00 a=80.00 d=20.00 g=10.00");
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
IMPORT vector
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  FOR EACH name IN ["upem1", "upem0", "upem15", "upem16385", "upem16", "upem16384", "upem1000"]
    RES f AS canvas::Font = canvas::loadFont(name & ".ttf") TRAP(e)
      io::print(name & ": refused " & toString(e.code))
      CONTINUE FOR
    END TRAP
    canvas::destroyFont(f)
    io::print(name & ": accepted")
  NEXT
END FUNC
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

/// A TrueType Collection holding `faces`, each a standalone sfnt such as
/// `truetype_fixture` builds (plan-148-A).
///
/// A collection is a `ttcf` header, a face count and one offset per face, then the faces
/// themselves. Each face keeps its own table directory, but the offsets in that directory
/// count from the start of the *collection* — so every copied face has its records
/// rebased by where it landed. That rebasing is exactly what the loader has to undo, and
/// getting it wrong here would make the test agree with a loader that is wrong the same
/// way; the round-trip test's oracle is the plain file, which shares none of this code.
fn collection(faces: &[Vec<u8>]) -> Vec<u8> {
    let header_len = 12 + 4 * faces.len();
    let mut bases = Vec::new();
    let mut at = header_len.div_ceil(4) * 4;
    for face in faces {
        bases.push(at);
        at += face.len().div_ceil(4) * 4;
    }
    let mut out = Vec::new();
    out.extend(b"ttcf");
    out.extend(0x0001_0000u32.to_be_bytes()); // TTC version 1.0
    out.extend((faces.len() as u32).to_be_bytes());
    for base in &bases {
        out.extend((*base as u32).to_be_bytes());
    }
    for (face, base) in faces.iter().zip(&bases) {
        out.resize(*base, 0);
        let mut copy = face.clone();
        let tables = u16::from_be_bytes([copy[4], copy[5]]) as usize;
        for i in 0..tables {
            let rec = 12 + i * 16 + 8;
            let offset = u32::from_be_bytes(copy[rec..rec + 4].try_into().unwrap());
            copy[rec..rec + 4].copy_from_slice(&(offset + *base as u32).to_be_bytes());
        }
        out.extend(copy);
    }
    out
}

const DRAW_A: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("FONTFILE")
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 200.0], text := "AB", fonts := [face], fontSize := 100.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([label])
END FUNC
"#;

/// A collection's first face draws exactly what the same font drawn from its own file
/// draws (plan-148-A Phase 1). Exact, because extraction is a byte copy: any difference
/// at all means a table was rebased to the wrong place.
#[test]
fn a_collection_loads_its_first_face() {
    let ttc = collection(&[
        minimal_truetype(),
        truetype_fixture(1000, 2, [0, 0, 500, 500]),
    ]);
    let (want, _) = render_env(
        "canvas_ttc_oracle",
        &DRAW_A.replace("FONTFILE", "fixture.ttf"),
        &[],
    );
    let (got, _) = render_env_files(
        "canvas_ttc_face0",
        &DRAW_A.replace("FONTFILE", "fixture.ttc"),
        &[],
        &[("fixture.ttc", ttc)],
    );
    let height = (want.len() / 4 / WIDTH) as u32;
    let want = Frame::from_rgba(WIDTH as u32, height, want);
    let got = Frame::from_rgba(WIDTH as u32, height, got);
    assert!(
        want.pixels.chunks(4).any(|p| p[0] != 0),
        "the oracle frame drew no text, so the comparison would be vacuous",
    );
    if let Err(diff) = compare_exact(&got, &want) {
        panic!("face 0 of a collection draws differently from the same font's own file: {diff:?}");
    }
}

/// A collection whose face, or one of whose tables, lies past the end of the file is
/// refused with `ErrBadFontFile` rather than read past the end (plan-148-A Phase 1).
#[test]
fn a_collection_that_runs_past_its_end_is_refused() {
    // One face, offset far beyond the file.
    let mut past = Vec::new();
    past.extend(b"ttcf");
    past.extend(0x0001_0000u32.to_be_bytes());
    past.extend(1u32.to_be_bytes());
    past.extend(100_000u32.to_be_bytes());
    // A real face whose `glyf` record claims far more bytes than the file holds.
    let mut table = collection(&[minimal_truetype()]);
    let base = u32::from_be_bytes(table[12..16].try_into().unwrap()) as usize;
    let glyf_len = base + 12 + 16 + 12; // second record (`glyf`), its length field
    table[glyf_len..glyf_len + 4].copy_from_slice(&1_000_000u32.to_be_bytes());
    let lines = run_files(
        "canvas_ttc_past_end",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT io

FUNC attempt(label AS String, path AS String) AS Nothing
  RES f AS canvas::Font = canvas::loadFont(path) TRAP(e)
    io::print(label & ": refused " & toString(e.code))
    RETURN
  END TRAP
  io::print(label & ": accepted")
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  attempt("face", "past.ttc")
  attempt("table", "table.ttc")
END FUNC
"#,
        &[("past.ttc", past), ("table.ttc", table)],
    );
    assert_eq!(
        lines,
        vec![
            format!("face: refused {ERR_BAD_FONT_FILE}"),
            format!("table: refused {ERR_BAD_FONT_FILE}"),
        ],
    );
}

/// A `name` table (format 0) holding `records`: `(platform, encoding, language, nameID,
/// the string's bytes as stored)`.
fn name_table(records: &[(u16, u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(0u16.to_be_bytes()); // format 0
    out.extend((records.len() as u16).to_be_bytes());
    out.extend(((6 + 12 * records.len()) as u16).to_be_bytes()); // stringOffset
    let mut strings: Vec<u8> = Vec::new();
    for (platform, encoding, language, id, bytes) in records {
        for v in [*platform, *encoding, *language, *id, bytes.len() as u16] {
            out.extend(v.to_be_bytes());
        }
        out.extend((strings.len() as u16).to_be_bytes());
        strings.extend(bytes);
    }
    out.extend(strings);
    out
}

fn utf16be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// The minimal fixture carrying Windows-Unicode full and PostScript names, with glyph
/// 1's (`A`'s) advance set to `advance` so two faces can be told apart by measuring.
fn named_face(full: &str, post_script: &str, advance: u16) -> Vec<u8> {
    let mut font = minimal_truetype();
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    let hmtx = (0..count)
        .map(|i| 12 + i * 16)
        .find(|rec| &font[*rec..*rec + 4] == b"hmtx")
        .map(|rec| u32::from_be_bytes(font[rec + 8..rec + 12].try_into().unwrap()) as usize)
        .expect("the fixture has hmtx");
    font[hmtx + 4..hmtx + 6].copy_from_slice(&advance.to_be_bytes());
    with_table(
        &font,
        b"name",
        name_table(&[
            (3, 1, 0x0409, 4, utf16be(full)),
            (3, 1, 0x0409, 6, utf16be(post_script)),
        ]),
    )
}

/// `canvas::loadFont(path, face)` picks a collection's face by PostScript name or by full
/// name, and refuses a name no face carries with `ErrNotFound` (plan-148-A Phase 2). The faces differ only in `A`'s advance — 250 and 400 font units, 25 and
/// 40 px at size 100 — so the measured width says which one loaded.
#[test]
fn load_font_chooses_a_collection_face_by_name() {
    let ttc = collection(&[
        named_face("Face A Full", "FaceA", 250),
        named_face("Face B Full", "FaceB", 400),
    ]);
    // A Mac Roman-only face: 0x8E is `é` in Mac OS Roman.
    let roman = with_table(
        &minimal_truetype(),
        b"name",
        name_table(&[(1, 0, 0, 4, b"Caf\x8e".to_vec())]),
    );
    let lines = run_files(
        "canvas_font_face_by_name",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT io

FUNC attempt(label AS String, path AS String, face AS String) AS Nothing
  RES f AS canvas::Font = canvas::loadFont(path, face) TRAP(e)
    io::print(label & ": refused " & toString(e.code))
    RETURN
  END TRAP
  io::print(label & ": " & toString(canvas::measureText(f, 100.0, "A").width))
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  attempt("ps-b", "faces.ttc", "FaceB")
  attempt("ps-a", "faces.ttc", "FaceA")
  attempt("full-b", "faces.ttc", "Face B Full")
  attempt("absent", "faces.ttc", "Nope")
  attempt("roman", "roman.ttf", "Café")
END FUNC
"#,
        &[("faces.ttc", ttc), ("roman.ttf", roman)],
    );
    assert_eq!(
        lines,
        vec![
            "ps-b: 40.00".to_string(),
            "ps-a: 25.00".to_string(),
            "full-b: 40.00".to_string(),
            format!("absent: refused {ERR_FACE_NOT_FOUND}"),
            "roman: 25.00".to_string(),
        ],
    );
}

/// A real system collection loads by PostScript name: `Helvetica.ttc` ships with every
/// macOS, and its bold face is not face 0 — so this is the host-font proof that the
/// face walk and the extraction work on a production file, not only the fixture.
#[cfg(target_os = "macos")]
#[test]
fn a_system_collection_face_loads_by_post_script_name() {
    let lines = run(
        "canvas_font_helvetica_bold",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES regular AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Helvetica.ttc")
  RES bold AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Helvetica.ttc", "Helvetica-Bold")
  LET r AS Float = canvas::measureText(regular, 100.0, "Hamburgefonts").width
  LET b AS Float = canvas::measureText(bold, 100.0, "Hamburgefonts").width
  IF r > 0.0 AND b > r THEN
    io::print("bold is wider")
  ELSE
    io::print("regular " & toString(r) & " bold " & toString(b))
  END IF
END FUNC
"#,
    );
    assert_eq!(lines, vec!["bold is wider".to_string()]);
}
