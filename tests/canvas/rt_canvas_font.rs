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
//! cases exact: `OTTO` and `wOFF` are each a real thing a program might hand us and each
//! must be refused by the same rule, which is hard to arrange with borrowed system fonts
//! and trivial here. A `ttcf` collection is read (plan-148-A) — it loads its first face —
//! so a twelve-byte `ttcf` header, which holds no face at all, is refused for *that*.

#[path = "../common/mod.rs"]
mod common;

#[path = "rt_canvas_font/drawing.rs"]
mod drawing;
#[path = "rt_canvas_font/face_index_and_kerning.rs"]
mod face_index_and_kerning;
#[path = "rt_canvas_font/loading_and_measure.rs"]
mod loading_and_measure;
#[path = "rt_canvas_font/shaping.rs"]
mod shaping;

use common::canvas_image::{compare_exact, compare_within_tolerance, Frame, Tolerance};
use std::process::Command;

/// Build a `--app` program, run it headless, and return its stdout lines.
fn run(name: &str, source: &str) -> Vec<String> {
    run_files(name, source, &[])
}

/// As `run`, with `files` written into the project directory before the program runs.
fn run_files(name: &str, source: &str, files: &[(&str, Vec<u8>)]) -> Vec<String> {
    let project = common::temp_project(name, source);
    for (file, bytes) in files {
        std::fs::write(project.join(file), bytes).expect("write a fixture file");
    }
    let binary = common::build_app(&project, name);
    let out = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
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

/// `ErrBadFontFile`, from `errorCode`. Spelled here so a renumbering of the table
/// fails this test rather than silently changing what a program sees.
const ERR_BAD_FONT_FILE: &str = "77050022";

/// `ErrResourceClosed` — `mfb spec diagnostics error-codes` row `7-703-0004`.
const RESOURCE_CLOSED: &str = "77030004";

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
    truetype_fixture_with(upem, groups, square, &[])
}

/// As [`truetype_fixture`], with `glyph2` as glyph 2's `glyf` record instead of the
/// empty range (bug-757's composite). An empty `glyph2` builds the same bytes.
fn truetype_fixture_with(upem: u16, groups: u32, square: [i16; 4], glyph2: &[u8]) -> Vec<u8> {
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
    let glyph1_len = glyf.len() as u16;
    glyf.extend(glyph2);
    let glyf_len = glyf.len() as u16;
    let mut loca = Vec::new();
    for halved in [0u16, 0, glyph1_len / 2, glyf_len / 2] {
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
        .env("MFB_HEADLESS", "1")
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

/// As `run_with_font`, plus the dumped RGBA frame.
///
/// A separate entry point rather than widening `run_with_font`: every other test here
/// asserts on what the program printed, and a frame dump is a file those runs would
/// write and never read.
fn run_with_font_frame(name: &str, source: &str) -> Vec<u8> {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), minimal_truetype()).expect("write the font");
    let frame = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, name);
    let out = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        out.status.success(),
        "program exited {:?}:\n{}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let pixels = std::fs::read(&frame).expect("the canvas dump");
    let _ = std::fs::remove_dir_all(&project);
    pixels
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
    render_env_files(name, source, extra, &[])
}

/// As `render_env`, with `files` written beside `fixture.ttf`.
fn render_env_files(
    name: &str,
    source: &str,
    extra: &[(&str, &str)],
    files: &[(&str, Vec<u8>)],
) -> (Vec<u8>, String) {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), minimal_truetype()).expect("write the font");
    for (file, bytes) in files {
        std::fs::write(project.join(file), bytes).expect("write a fixture file");
    }
    let frame = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, name);
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
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

/// The lit columns of a frame, as `(first, last)` runs left to right.
fn lit_column_runs(frame: &[u8]) -> Vec<(usize, usize)> {
    let mut lit = vec![false; WIDTH];
    for i in 0..frame.len() / 4 {
        if frame[i * 4] != 0 || frame[i * 4 + 1] != 0 || frame[i * 4 + 2] != 0 {
            lit[i % WIDTH] = true;
        }
    }
    let mut runs = Vec::new();
    let mut start = None;
    for (x, on) in lit.iter().enumerate() {
        match (start, on) {
            (None, true) => start = Some(x),
            (Some(s), false) => {
                runs.push((s, x - 1));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push((s, WIDTH - 1));
    }
    runs
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

/// `font` with one more table, `tag`, added to its directory (plan-148-A).
///
/// Rebuilds the file rather than patching it: every existing table moves 16 bytes to
/// make room for the new record, and the directory stays in sorted tag order as a real
/// font's does.
fn with_table(font: &[u8], tag: &[u8; 4], data: Vec<u8>) -> Vec<u8> {
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..count)
        .map(|i| {
            let rec = 12 + i * 16;
            let at = u32::from_be_bytes(font[rec + 8..rec + 12].try_into().unwrap()) as usize;
            let len = u32::from_be_bytes(font[rec + 12..rec + 16].try_into().unwrap()) as usize;
            (
                font[rec..rec + 4].try_into().unwrap(),
                font[at..at + len].to_vec(),
            )
        })
        .collect();
    tables.push((*tag, data));
    tables.sort_by_key(|table| table.0);
    let mut out = font[..4].to_vec();
    out.extend((tables.len() as u16).to_be_bytes());
    out.extend([0u8; 6]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend(tag);
        out.extend(0u32.to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    out
}

// ---------------------------------------------------------------------------------------
// plan-162-A: the parse-once face index.
//
// `canvas::loadFont` parses the metrics, the `cmap` and the advances once, into a flat
// `List OF Integer` the `Font` resource owns. Nothing hands that list back to a program,
// so a `--debug` build run with `MFB_CANVAS_FACE_DUMP=<file>` appends one `face …` line per
// load: the window these tests read it through (the `MFB_CANVAS_STATS` precedent).

/// Build `source` as a `--debug` app with `files` beside it, run it headless with `env`
/// (`{project}` in a value is the project directory), and return its stdout, its stderr
/// and the project directory, which the caller removes. Panics on a non-zero exit.
fn run_debug_files(
    name: &str,
    source: &str,
    files: &[(&str, Vec<u8>)],
    env: &[(&str, String)],
) -> (String, String, std::path::PathBuf) {
    let project = common::temp_project(name, source);
    for (file, bytes) in files {
        std::fs::write(project.join(file), bytes).expect("write a fixture file");
    }
    let binary = common::build_app_debug(&project, name);
    let mut command = Command::new(&binary);
    command.current_dir(&project).env("MFB_HEADLESS", "1");
    for (key, value) in env {
        command.env(
            key,
            value.replace("{project}", &project.display().to_string()),
        );
    }
    let out = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "program {}:\n{stdout}\n{stderr}",
        common::exit_description(&out.status),
    );
    (stdout, stderr, project)
}

/// `font` with table `tag`'s contents replaced by `data`, the directory and every table
/// offset rebuilt so the file stays well formed.
fn replace_table(font: &[u8], tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    let be32 = |at: usize| u32::from_be_bytes([font[at], font[at + 1], font[at + 2], font[at + 3]]);
    let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..count)
        .map(|i| {
            let rec = 12 + i * 16;
            let mut t = [0u8; 4];
            t.copy_from_slice(&font[rec..rec + 4]);
            let (at, len) = (be32(rec + 8) as usize, be32(rec + 12) as usize);
            (t, font[at..at + len].to_vec())
        })
        .collect();
    let slot = tables
        .iter_mut()
        .find(|(t, _)| t == tag)
        .expect("the fixture carries the table");
    slot.1 = data.to_vec();
    let mut out = font[..12].to_vec();
    let mut offset = 12 + 16 * count;
    let mut body = Vec::new();
    for (t, bytes) in &tables {
        out.extend(t);
        out.extend(0u32.to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((bytes.len() as u32).to_be_bytes());
        let padded = bytes.len().div_ceil(4) * 4;
        body.extend(bytes);
        body.resize(body.len() + padded - bytes.len(), 0);
        offset += padded;
    }
    out.extend(body);
    out
}

/// A `cmap` holding one subtable, built from its body.
fn cmap_with(platform: u16, encoding: u16, subtable: Vec<u8>) -> Vec<u8> {
    let mut cmap = Vec::new();
    cmap.extend(0u16.to_be_bytes()); // version
    cmap.extend(1u16.to_be_bytes()); // numTables
    cmap.extend(platform.to_be_bytes());
    cmap.extend(encoding.to_be_bytes());
    cmap.extend(12u32.to_be_bytes()); // the subtable follows this one record
    cmap.extend(subtable);
    cmap
}

/// A format-12 subtable whose groups are `(first, last, startGlyph)` in the order given.
fn cmap12(groups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(12u16.to_be_bytes());
    body.extend(0u16.to_be_bytes());
    body.extend(((16 + 12 * groups.len()) as u32).to_be_bytes());
    body.extend(0u32.to_be_bytes()); // language
    body.extend((groups.len() as u32).to_be_bytes());
    for (first, last, glyph) in groups {
        body.extend(first.to_be_bytes());
        body.extend(last.to_be_bytes());
        body.extend(glyph.to_be_bytes());
    }
    body
}

// ---------------------------------------------------------------------------------------
// plan-162-B: kerning — `GPOS` pair adjustment and the legacy `kern` table.
//
// The fixture is the square font with glyph 2 given the same square outline, and a
// `cmap` mapping `A` → 1 (advance 250) and `V` → 2 (advance 300). Each table builder
// below writes the OpenType structure it names, field by field, so a fixture states the
// font a kerning case needs rather than borrowing a system font's.

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

/// Glyph 1's record — a square — so another glyph id can carry the same outline.
fn square_record(square: [i16; 4]) -> Vec<u8> {
    let [x0, y0, x1, y1] = square;
    let mut glyf = Vec::new();
    glyf.extend(be16(1));
    for v in square {
        glyf.extend(be16(v as u16));
    }
    glyf.extend(be16(3));
    glyf.extend(be16(0));
    glyf.extend([0x01u8; 4]);
    for dx in [x0, x1 - x0, 0, x0 - x1] {
        glyf.extend(be16(dx as u16));
    }
    for dy in [y0, 0, y1 - y0, 0] {
        glyf.extend(be16(dy as u16));
    }
    glyf
}

/// The glyph id `A` maps to in [`kerning_font`]: advance 250.
const GID_A: u16 = 1;
/// The glyph id `V` maps to in [`kerning_font`]: advance 300.
const GID_V: u16 = 2;

/// `font` with each `(tag, data)` table replaced, or added when it is absent; the
/// directory is kept in tag order, as a real font's is.
fn with_tables(font: &[u8], tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = font.to_vec();
    for (tag, data) in tables {
        let count = u16::from_be_bytes([out[4], out[5]]) as usize;
        let present = (0..count).any(|i| &out[12 + i * 16..12 + i * 16 + 4] == *tag);
        if !present {
            // Add an empty table record in sorted position, then fill it.
            let mut records: Vec<([u8; 4], Vec<u8>)> = (0..count)
                .map(|i| {
                    let rec = 12 + i * 16;
                    let be32 = |at: usize| {
                        u32::from_be_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]])
                            as usize
                    };
                    let mut t = [0u8; 4];
                    t.copy_from_slice(&out[rec..rec + 4]);
                    (
                        t,
                        out[be32(rec + 8)..be32(rec + 8) + be32(rec + 12)].to_vec(),
                    )
                })
                .collect();
            records.push((**tag, Vec::new()));
            records.sort_by(|a, b| a.0.cmp(&b.0));
            let mut rebuilt = out[..4].to_vec();
            rebuilt.extend(be16(records.len() as u16));
            rebuilt.extend([0u8; 6]);
            let mut offset = 12 + 16 * records.len();
            let mut body = Vec::new();
            for (t, bytes) in &records {
                rebuilt.extend(t);
                rebuilt.extend(0u32.to_be_bytes());
                rebuilt.extend((offset as u32).to_be_bytes());
                rebuilt.extend((bytes.len() as u32).to_be_bytes());
                let padded = bytes.len().div_ceil(4) * 4;
                body.extend(bytes);
                body.resize(body.len() + padded - bytes.len(), 0);
                offset += padded;
            }
            rebuilt.extend(body);
            out = rebuilt;
        }
        out = replace_table(&out, tag, data);
    }
    out
}

/// The kerning base font, plus `tables`.
fn kerning_font(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let square = [100, 0, 400, 300];
    let base = truetype_fixture_with(1000, 2, square, &square_record(square));
    let cmap = cmap_with(
        3,
        10,
        cmap12(&[
            (b'A' as u32, b'A' as u32, GID_A as u32),
            (b'V' as u32, b'V' as u32, GID_V as u32),
        ]),
    );
    let mut all: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"cmap", cmap)];
    all.extend(tables.iter().map(|(t, d)| (*t, d.clone())));
    with_tables(&base, &all)
}

/// Bytes in a ValueRecord of `format`: two per set bit of the low byte.
fn value_size(format: u16) -> usize {
    2 * (format & 0x00ff).count_ones() as usize
}

/// A ValueRecord of `format` whose fields, in format-bit order, are `fields`.
fn value_record(format: u16, fields: &[i16]) -> Vec<u8> {
    assert_eq!(
        fields.len() * 2,
        value_size(format),
        "one field per format bit"
    );
    fields.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// Coverage format 1: the sorted glyph array.
fn coverage1(glyphs: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(be16(1));
    out.extend(be16(glyphs.len() as u16));
    for g in glyphs {
        out.extend(be16(*g));
    }
    out
}

/// One PairValueRecord: the second glyph, then its two ValueRecords.
type PairRecord = (u16, Vec<u8>, Vec<u8>);

/// PairPos format 1 over `firsts` (one pair set each, in coverage order).
fn pair_pos1(firsts: &[u16], vf1: u16, vf2: u16, sets: &[Vec<PairRecord>]) -> Vec<u8> {
    assert_eq!(firsts.len(), sets.len());
    let header = 10 + 2 * sets.len();
    let mut sets_bytes: Vec<Vec<u8>> = Vec::new();
    for set in sets {
        let mut b = Vec::new();
        b.extend(be16(set.len() as u16));
        for (second, v1, v2) in set {
            assert_eq!(v1.len(), value_size(vf1));
            assert_eq!(v2.len(), value_size(vf2));
            b.extend(be16(*second));
            b.extend(v1);
            b.extend(v2);
        }
        sets_bytes.push(b);
    }
    let mut offsets = Vec::new();
    let mut at = header;
    for b in &sets_bytes {
        offsets.push(at);
        at += b.len();
    }
    let mut out = Vec::new();
    out.extend(be16(1));
    out.extend(be16(at as u16)); // coverage after the sets
    out.extend(be16(vf1));
    out.extend(be16(vf2));
    out.extend(be16(sets.len() as u16));
    for o in offsets {
        out.extend(be16(o as u16));
    }
    for b in sets_bytes {
        out.extend(b);
    }
    out.extend(coverage1(firsts));
    out
}

/// A lookup of `kind` holding `subtables`.
fn lookup(kind: u16, subtables: &[Vec<u8>]) -> Vec<u8> {
    let header = 6 + 2 * subtables.len();
    let mut out = Vec::new();
    out.extend(be16(kind));
    out.extend(be16(0)); // lookupFlag
    out.extend(be16(subtables.len() as u16));
    let mut at = header;
    for s in subtables {
        out.extend(be16(at as u16));
        at += s.len();
    }
    for s in subtables {
        out.extend(s);
    }
    out
}

/// A `GPOS` (or `GSUB`) table: one script, `script`, whose default LangSys enables one
/// feature, `feature`, over every lookup in `lookups`, in order.
fn layout_table(script: &[u8; 4], feature: &[u8; 4], lookups: &[Vec<u8>]) -> Vec<u8> {
    // ScriptList: one record, its Script, the Script's default LangSys.
    let mut script_list = Vec::new();
    script_list.extend(be16(1));
    script_list.extend(script);
    script_list.extend(be16(8)); // Script at 8
    script_list.extend(be16(4)); // Script: defaultLangSys at 4
    script_list.extend(be16(0)); // langSysCount
    script_list.extend(be16(0)); // LangSys: lookupOrder
    script_list.extend(be16(0xffff)); // requiredFeatureIndex: none
    script_list.extend(be16(1));
    script_list.extend(be16(0)); // featureIndex 0
                                 // FeatureList: one record and its Feature.
    let mut feature_list = Vec::new();
    feature_list.extend(be16(1));
    feature_list.extend(feature);
    feature_list.extend(be16(8));
    feature_list.extend(be16(0)); // featureParams
    feature_list.extend(be16(lookups.len() as u16));
    for i in 0..lookups.len() {
        feature_list.extend(be16(i as u16));
    }
    // LookupList.
    let mut lookup_list = Vec::new();
    lookup_list.extend(be16(lookups.len() as u16));
    let mut at = 2 + 2 * lookups.len();
    for l in lookups {
        lookup_list.extend(be16(at as u16));
        at += l.len();
    }
    for l in lookups {
        lookup_list.extend(l);
    }
    let script_at = 10;
    let feature_at = script_at + script_list.len();
    let lookup_at = feature_at + feature_list.len();
    let mut out = Vec::new();
    out.extend(be16(1));
    out.extend(be16(0));
    out.extend(be16(script_at as u16));
    out.extend(be16(feature_at as u16));
    out.extend(be16(lookup_at as u16));
    out.extend(script_list);
    out.extend(feature_list);
    out.extend(lookup_list);
    out
}

const X_ADVANCE: u16 = 0x0004;

/// (a) `GPOS` format 1 under `kern`: `A,A` = −30 and `A,V` = −120 in one pair set, so a
/// misread record size moves `A,V` onto garbage.
fn gpos_format1() -> Vec<u8> {
    let set = vec![
        (GID_A, value_record(X_ADVANCE, &[-30]), vec![]),
        (GID_V, value_record(X_ADVANCE, &[-120]), vec![]),
    ];
    layout_table(
        b"DFLT",
        b"kern",
        &[lookup(2, &[pair_pos1(&[GID_A], X_ADVANCE, 0, &[set])])],
    )
}
