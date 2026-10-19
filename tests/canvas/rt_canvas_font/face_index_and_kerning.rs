//! plan-176-D: `rt_canvas_font`'s face index tests (tables, cmaps, system fonts, the
//! load/destroy cycle) and kerning tests (`kern` and GPOS pair positioning). The shared
//! sfnt/OpenType table builders live in the root, `rt_canvas_font.rs`.

use super::*;

/// The face index of every font `source` loads from `files`, one `Vec` per load.
fn face_dumps(name: &str, source: &str, files: &[(&str, Vec<u8>)]) -> Vec<Vec<i64>> {
    let (_, _, project) = run_debug_files(
        name,
        source,
        files,
        &[("MFB_CANVAS_FACE_DUMP", "{project}/face.txt".to_string())],
    );
    let text = std::fs::read_to_string(project.join("face.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    text.lines()
        .map(|line| {
            let mut words = line.split(' ');
            assert_eq!(
                words.next(),
                Some("face"),
                "a dump line is `face …`: {line}"
            );
            words.map(|w| w.parse().expect("an index word")).collect()
        })
        .collect()
}

/// `font` with the table directory's `from` tag renamed `to`, so `__canvas_fontTable`
/// no longer finds it: the cheapest way to build "a font with no `head`".
fn rename_table(font: &[u8], from: &[u8; 4], to: &[u8; 4]) -> Vec<u8> {
    let mut out = font.to_vec();
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    let at = (0..tables)
        .map(|i| 12 + i * 16)
        .find(|&rec| &font[rec..rec + 4] == from)
        .expect("the fixture carries the table");
    out[at..at + 4].copy_from_slice(to);
    out
}

const LOAD_FIXTURE: &str = r#"IMPORT app
IMPORT canvas

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("fixture.ttf")
END FUNC
"#;

/// The fixture's index, word for word, from its own tables: `head.unitsPerEm` 1000,
/// `hhea` 800 / −200 / 100, a format-12 `cmap` mapping `A`→1 and `B`→2 (two runs of
/// delta −64), three `hmtx` advances, no kerning (word 9 is 0), and no `post` or `OS/2`
/// (words 10-13 unset, −32768), no ligatures or carets (14-15). Layout version 4, whose
/// header is sixteen words (plan-162-B added word 9, -D words 10-13, -F 14-15).
#[test]
fn face_index_of_the_fixture_is_its_own_tables() {
    let dumps = face_dumps(
        "canvas_face_index_fixture",
        LOAD_FIXTURE,
        &[("fixture.ttf", minimal_truetype())],
    );
    assert_eq!(
        dumps,
        vec![vec![
            4, 1000, 800, -200, 100, // version, unitsPerEm, ascent, descent, lineGap
            2, 16, 3, 22, // R at word 16, H at word 22
            0,  // no kerning section
            -32768, -32768, -32768, -32768, // no underline, no strikeout
            0, 0, // no ligatures, no carets
            65, 65, -64, 66, 66, -64, // the runs
            500, 250, 300, // the advances
        ]],
    );
}

/// A font with no `head` has no scale: word 1 is 0, which every reader takes as "draws
/// and measures as nothing" — the answer `__canvas_fontUnitsPerEm` gives it. The rest of
/// the index is still the font's.
#[test]
fn face_index_of_a_font_with_no_head_has_no_units_per_em() {
    let headless = rename_table(&minimal_truetype(), b"head", b"hxad");
    let dumps = face_dumps(
        "canvas_face_index_no_head",
        LOAD_FIXTURE,
        &[("fixture.ttf", headless)],
    );
    assert_eq!(
        dumps,
        vec![vec![
            4, 0, 800, -200, 100, 2, 16, 3, 22, 0, -32768, -32768, -32768, -32768, 0, 0, 65, 65,
            -64, 66, 66, -64, 500, 250, 300
        ]],
    );
}

/// A format-4 segment: `start..=end`, `delta`, and either no glyph array (the glyph is
/// `cp + delta`, mod 65536) or one entry per codepoint (an `idRangeOffset` segment).
struct Segment {
    start: u16,
    end: u16,
    delta: i16,
    glyphs: Option<Vec<u16>>,
}

/// A format-4 subtable with `segments` in exactly the order given — sorted or not.
fn cmap4(segments: &[Segment]) -> Vec<u8> {
    let segs = segments.len();
    let mut ends = Vec::new();
    let mut starts = Vec::new();
    let mut deltas = Vec::new();
    let mut ranges = Vec::new();
    let mut array: Vec<u8> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        ends.extend(seg.end.to_be_bytes());
        starts.extend(seg.start.to_be_bytes());
        deltas.extend(seg.delta.to_be_bytes());
        match &seg.glyphs {
            None => ranges.extend(0u16.to_be_bytes()),
            Some(glyphs) => {
                // Bytes from this segment's own `idRangeOffset` word to its glyphs.
                let offset = (segs - i) * 2 + array.len();
                ranges.extend((offset as u16).to_be_bytes());
                for g in glyphs {
                    array.extend(g.to_be_bytes());
                }
            }
        }
    }
    let mut body = Vec::new();
    body.extend(4u16.to_be_bytes()); // format
    let length = 16 + segs * 8 + array.len();
    body.extend((length as u16).to_be_bytes());
    body.extend(0u16.to_be_bytes()); // language
    body.extend(((segs * 2) as u16).to_be_bytes());
    body.extend([0u8; 6]); // searchRange, entrySelector, rangeShift — unread
    body.extend(ends);
    body.extend(0u16.to_be_bytes()); // reservedPad
    body.extend(starts);
    body.extend(deltas);
    body.extend(ranges);
    body.extend(array);
    body
}

/// Run the `--debug` face self-check over `font`: `measureText` with
/// `MFB_CANVAS_FACE_CHECK` set compares the index readers with the byte readers for every
/// scalar and glyph id. Returns the `faceCheck …` line and the font's index.
fn face_check(name: &str, font: Vec<u8>) -> (String, Vec<i64>) {
    let source = r#"IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("fixture.ttf")
  io::print(toString(canvas::measureText(f, 10.0, "A").width))
END FUNC
"#;
    let (_, _, project) = run_debug_files(
        name,
        source,
        &[("fixture.ttf", font)],
        &[
            ("MFB_CANVAS_FACE_CHECK", "{project}/check.txt".to_string()),
            ("MFB_CANVAS_FACE_DUMP", "{project}/face.txt".to_string()),
        ],
    );
    let check = std::fs::read_to_string(project.join("check.txt")).unwrap_or_default();
    let face = std::fs::read_to_string(project.join("face.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    let index = face
        .split_whitespace()
        .skip(1)
        .map(|w| w.parse().expect("an index word"))
        .collect();
    (check.trim().to_string(), index)
}

/// Assert a self-check line found no mismatch, and return its `mapped=` count.
fn assert_check_clean(check: &str) -> i64 {
    for field in [
        "glyphMismatches=0",
        "advanceMismatches=0",
        "metricMismatches=0",
    ] {
        assert!(
            check.split(' ').any(|f| f == field),
            "the index disagrees with the byte readers: `{check}`"
        );
    }
    check
        .split(' ')
        .find_map(|f| f.strip_prefix("mapped="))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no `mapped=` in `{check}`"))
}

/// The fixture's index answers what its bytes do, for every scalar and glyph id.
#[test]
fn face_index_answers_the_fixture_exactly() {
    let (check, _) = face_check("canvas_face_index_sweep_fixture", minimal_truetype());
    assert_eq!(
        assert_check_clean(&check),
        2,
        "`A` and `B` are mapped: `{check}`"
    );
}

/// Format 4, every way a segment can answer: delta segments, a delta that wraps past
/// 65,535, `idRangeOffset` segments whose array holds a `0` (unmapped) amid real ids, a
/// segment starting above the end of the one before (the gap reads 0), and a segment
/// listed after one whose endCode already covers it — so `__canvas_cmap4`'s "first
/// segment whose endCode reaches the codepoint" rule, not the file order, decides who
/// answers.
#[test]
fn face_index_answers_a_format_4_cmap_exactly() {
    let segments = [
        Segment {
            start: 0x41,
            end: 0x5a,
            delta: -0x40,
            glyphs: None,
        },
        // An array segment: `0` is unmapped, the rest get the delta added.
        Segment {
            start: 0x60,
            end: 0x66,
            delta: 3,
            glyphs: Some(vec![9, 0, 9, 10, 11, 0, 40]),
        },
        // Wraps: 0xfff0 + 0x20 = 0x10010 → glyph 0x10, and 0xfffe → 0x1e.
        Segment {
            start: 0xfff0,
            end: 0xfffe,
            delta: 0x20,
            glyphs: None,
        },
        // Shadowed: its endCode (0x50) is below 0x5a, so the first segment answers it.
        Segment {
            start: 0x48,
            end: 0x50,
            delta: 100,
            glyphs: None,
        },
        // Reaches back below the wrap segment's start; only 0xffff is its own.
        Segment {
            start: 0x70,
            end: 0xffff,
            delta: 1,
            glyphs: None,
        },
    ];
    let font = replace_table(
        &minimal_truetype(),
        b"cmap",
        &cmap_with(3, 1, cmap4(&segments)),
    );
    let (check, index) = face_check("canvas_face_index_sweep_cmap4", font);
    // 26 letters, 5 of the array's 7, 15 wrapped, and 0xffff + 1 = 0x10000, which wraps to
    // glyph 0: unmapped.
    assert_eq!(assert_check_clean(&check), 26 + 5 + 15, "`{check}`");
    let runs = index[5] as usize;
    let at = index[6] as usize;
    assert_eq!(
        &index[at..at + runs * 3],
        &[
            0x41,
            0x5a,
            -0x40, // the letters
            0x60,
            0x60,
            -0x54, // 0x60 → 12
            0x62,
            0x64,
            -0x56, // 0x62..0x64 → 12..14: one delta, one run
            0x66,
            0x66,
            -0x3b, // 0x66 → 43
            0xfff0,
            0xfffe,
            0x20 - 0x10000, // the wrapped run, delta mod 65536
        ],
        "format-4 runs",
    );
}

/// Format 12 whose groups overlap, go backwards and include an empty one: still "the
/// first group that contains the codepoint", settled by `__canvas_cmap12Painted`.
#[test]
fn face_index_answers_an_unsorted_overlapping_format_12_cmap_exactly() {
    let groups = [
        (100, 200, 1000),
        (150, 300, 2000), // 150..200 is the first group's
        (50, 120, 3000),  // only 50..99 is its own
        (400, 399, 5),    // empty: last before first
        (250, 260, 7000), // inside the second group: none of it is its own
        (500, 600, 10),
        (90, 95, 4000), // inside the third group: none of it is its own
    ];
    let font = replace_table(
        &minimal_truetype(),
        b"cmap",
        &cmap_with(3, 10, cmap12(&groups)),
    );
    let (check, index) = face_check("canvas_face_index_sweep_cmap12_painted", font);
    assert_eq!(
        assert_check_clean(&check),
        (200 - 50 + 1) + (300 - 200) + (600 - 500 + 1),
        "`{check}`"
    );
    let runs = index[5] as usize;
    let at = index[6] as usize;
    assert_eq!(
        &index[at..at + runs * 3],
        &[50, 99, 2950, 100, 200, 900, 201, 300, 1850, 500, 600, -490],
        "painted format-12 runs",
    );
}

/// A real font holds its index to its bytes for every scalar and glyph id: a format-4
/// face with hundreds of runs and `idRangeOffset` segments, and a format-12 face. The
/// fixture fonts above pin each rule; these pin the rules against what shipping fonts
/// actually contain. macOS only, where the files are known to exist.
#[cfg(target_os = "macos")]
#[test]
fn face_index_answers_real_system_fonts_exactly() {
    for (name, path) in [
        (
            "canvas_face_index_sweep_arial",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ),
        (
            "canvas_face_index_sweep_bamum",
            "/System/Library/Fonts/Supplemental/NotoSansBamum-Regular.ttf",
        ),
    ] {
        let Ok(font) = std::fs::read(path) else {
            eprintln!("skipping {path}: not installed on this host");
            continue;
        };
        let (check, _) = face_check(name, font);
        assert!(assert_check_clean(&check) > 0, "{path}: `{check}`");
    }
}

/// `arena.0.live_bytes` at exit for `cycles` load/destroy cycles of the fixture font.
fn live_after_cycles(cycles: usize) -> u64 {
    let source = r#"IMPORT app
IMPORT canvas

FUNC cycle() AS Nothing
  RES f AS canvas::Font = canvas::loadFont("fixture.ttf")
  canvas::destroyFont(f)
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT i AS Integer = 0
  WHILE i < {N}
    cycle()
    i = i + 1
  END WHILE
END FUNC
"#
    .replace("{N}", &cycles.to_string());
    let name = format!("canvas_face_index_cycles_{cycles}");
    let (_, stderr, project) =
        run_debug_files(&name, &source, &[("fixture.ttf", minimal_truetype())], &[]);
    let _ = std::fs::remove_dir_all(&project);
    let lines = common::debug_report::arena_lines(&name, &stderr);
    common::debug_report::counter(&name, &lines, 0, "live_bytes")
}

/// A load/destroy cycle leaves behind exactly what a closed font keeps — its bytes, its
/// face index and its record, none of which a close frees (`.ai/canvas-threading.md` §7)
/// — and nothing the index's construction allocated along the way. `live_bytes` is an
/// exact count, so the growth must be exactly those three blocks: each collection is its
/// 40-byte header plus its payload, the record is `RESOURCE_RECORD_SIZE` (112), and the
/// pool rounds every request to 16 bytes (`mfb man debug pools`). A builder that leaked one
/// intermediate list per load — the cmap runs, the advances, the header — breaks it.
#[test]
fn a_load_destroy_cycle_keeps_only_the_fonts_own_blocks() {
    const FEW: usize = 40;
    const MANY: usize = 80;
    let font = minimal_truetype().len() as u64;
    let words = 25u64; // the fixture's index, `face_index_of_the_fixture_is_its_own_tables`
    let round = |n: u64| n.div_ceil(16) * 16;
    let per_cycle = round(40 + font) + round(40 + 8 * words) + 112;
    let few = live_after_cycles(FEW);
    let many = live_after_cycles(MANY);
    let grew = many.saturating_sub(few);
    assert_eq!(
        grew,
        per_cycle * (MANY - FEW) as u64,
        "{FEW} cycles left {few} live bytes and {MANY} left {many}; each cycle must keep \
         exactly a closed font's bytes, index and record, {per_cycle} bytes",
    );
}

/// ClassDef format 2: `(start, end, class)` ranges.
fn class_def2(ranges: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(be16(2));
    out.extend(be16(ranges.len() as u16));
    for (start, end, class) in ranges {
        out.extend(be16(*start));
        out.extend(be16(*end));
        out.extend(be16(*class));
    }
    out
}

/// ClassDef format 1: consecutive glyphs from `start`, one class each.
fn class_def1(start: u16, classes: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(be16(1));
    out.extend(be16(start));
    out.extend(be16(classes.len() as u16));
    for c in classes {
        out.extend(be16(*c));
    }
    out
}

/// PairPos format 2: a coverage, two ClassDefs and the `class1 × class2` matrix of
/// `(value1, value2)` records.
fn pair_pos2(
    firsts: &[u16],
    vf1: u16,
    vf2: u16,
    class1: Vec<u8>,
    class2: Vec<u8>,
    matrix: &[Vec<(Vec<u8>, Vec<u8>)>],
) -> Vec<u8> {
    let c1 = matrix.len();
    let c2 = matrix[0].len();
    let header = 16;
    let cell = value_size(vf1) + value_size(vf2);
    let records = c1 * c2 * cell;
    let coverage = coverage1(firsts);
    let cov_at = header + records;
    let cd1_at = cov_at + coverage.len();
    let cd2_at = cd1_at + class1.len();
    let mut out = Vec::new();
    out.extend(be16(2));
    out.extend(be16(cov_at as u16));
    out.extend(be16(vf1));
    out.extend(be16(vf2));
    out.extend(be16(cd1_at as u16));
    out.extend(be16(cd2_at as u16));
    out.extend(be16(c1 as u16));
    out.extend(be16(c2 as u16));
    for row in matrix {
        assert_eq!(row.len(), c2);
        for (v1, v2) in row {
            assert_eq!(v1.len() + v2.len(), cell);
            out.extend(v1);
            out.extend(v2);
        }
    }
    out.extend(coverage);
    out.extend(class1);
    out.extend(class2);
    out
}

/// An Extension subtable (lookup type 9) wrapping `subtable` of `kind`.
fn extension(kind: u16, subtable: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(be16(1));
    out.extend(be16(kind));
    out.extend(8u32.to_be_bytes());
    out.extend(subtable);
    out
}

/// A version-0 `kern` table with one format-0 subtable of `coverage`.
fn kern_table(coverage: u16, pairs: &[(u16, u16, i16)]) -> Vec<u8> {
    let mut sub = Vec::new();
    sub.extend(be16(0)); // version
    sub.extend(be16((14 + 6 * pairs.len()) as u16));
    sub.extend(be16(coverage));
    sub.extend(be16(pairs.len() as u16));
    sub.extend([0u8; 6]); // searchRange, entrySelector, rangeShift
    for (l, r, v) in pairs {
        sub.extend(be16(*l));
        sub.extend(be16(*r));
        sub.extend(v.to_be_bytes());
    }
    let mut out = Vec::new();
    out.extend(be16(0));
    out.extend(be16(1));
    out.extend(sub);
    out
}

const X_PLACEMENT: u16 = 0x0001;

const Y_PLACEMENT: u16 = 0x0002;

const Y_ADVANCE: u16 = 0x0008;

const X_ADVANCE_DEVICE: u16 = 0x0040;

/// (b) `GPOS` format 2 inside an Extension lookup: `A` is class 1 on the left, `V` class
/// 1 on the right, and only that cell is −120.
fn gpos_format2_extension() -> Vec<u8> {
    let zero = || (value_record(X_ADVANCE, &[0]), vec![]);
    let matrix = vec![
        vec![zero(), zero()],
        vec![zero(), (value_record(X_ADVANCE, &[-120]), vec![])],
    ];
    let sub = pair_pos2(
        &[GID_A],
        X_ADVANCE,
        0,
        class_def1(GID_A, &[1]),
        class_def2(&[(GID_V, GID_V, 1)]),
        &matrix,
    );
    layout_table(b"DFLT", b"kern", &[lookup(9, &[extension(2, sub)])])
}

/// (c) only a `kern` table: `A,A` = −30, `A,V` = −80.
fn kern_only() -> Vec<u8> {
    kern_table(0x0001, &[(GID_A, GID_A, -30), (GID_A, GID_V, -80)])
}

/// (e) a first ValueRecord carrying `XPlacement`, `XAdvance`, `YAdvance` and an
/// `XAdvDevice` offset (junk, never followed), and a second carrying placements only.
fn gpos_wide_value_records() -> Vec<u8> {
    let vf1 = X_PLACEMENT | X_ADVANCE | Y_ADVANCE | X_ADVANCE_DEVICE;
    let vf2 = X_PLACEMENT | Y_PLACEMENT;
    let set = vec![
        (
            GID_A,
            value_record(vf1, &[55, -30, 33, -1]),
            value_record(vf2, &[7, 7]),
        ),
        (
            GID_V,
            value_record(vf1, &[55, -120, 33, -1]),
            value_record(vf2, &[9, 9]),
        ),
    ];
    layout_table(
        b"DFLT",
        b"kern",
        &[lookup(2, &[pair_pos1(&[GID_A], vf1, vf2, &[set])])],
    )
}

/// The five kerning fixtures, by name.
fn kerning_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("gpos1", kerning_font(&[(b"GPOS", gpos_format1())])),
        (
            "gpos2ext",
            kerning_font(&[(b"GPOS", gpos_format2_extension())]),
        ),
        ("kern", kerning_font(&[(b"kern", kern_only())])),
        (
            "both",
            kerning_font(&[(b"GPOS", gpos_format1()), (b"kern", kern_only())]),
        ),
        (
            "wide",
            kerning_font(&[(b"GPOS", gpos_wide_value_records())]),
        ),
    ]
}

/// `measureText(…, 1000.0, s).width` for each string in `texts`, per fixture, in
/// thousandths of a pixel. At size 1000 in a 1000-unit em the scale is 1, so a width is
/// its advances in font units, exactly.
fn kerned_widths(name: &str, texts: &[&str]) -> Vec<(String, Vec<i64>)> {
    let fixtures = kerning_fixtures();
    let mut source = String::from(
        "IMPORT app\nIMPORT canvas\nIMPORT io\n\nFUNC widthOf(path AS String, s AS String) AS Integer\n  RES f AS canvas::Font = canvas::loadFont(path)\n  RETURN toInt(canvas::measureText(f, 1000.0, s).width * 1000.0)\nEND FUNC\n\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n",
    );
    for (fixture, _) in &fixtures {
        let mut line = format!("  io::print(\"{fixture}\"");
        for t in texts {
            line.push_str(&format!(
                " & \" \" & toString(widthOf(\"{fixture}.ttf\", \"{t}\"))"
            ));
        }
        line.push_str(")\n");
        source.push_str(&line);
    }
    source.push_str("END FUNC\n");
    let files: Vec<(String, Vec<u8>)> = fixtures
        .into_iter()
        .map(|(n, f)| (format!("{n}.ttf"), f))
        .collect();
    let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, f)| (n.as_str(), f.clone())).collect();
    run_files(name, &source, &files)
        .into_iter()
        .map(|line| {
            let mut words = line.split(' ');
            let fixture = words.next().unwrap_or_default().to_string();
            (
                fixture,
                words.map(|w| w.parse().expect("a width")).collect(),
            )
        })
        .collect()
}

/// `AV` measures `adv(A) + kern(A, V) + adv(V)`; `VA` is unkerned in every fixture; `AA`
/// takes the pair-set record before `A,V`, so a wrong ValueRecord size breaks one of the
/// two. `GPOS` wins over `kern` when a font has both.
#[test]
fn kern_pairs_measure_at_the_kerned_width() {
    let widths = kerned_widths("canvas_kern_widths", &["AV", "VA", "AA"]);
    let unkerned_av = (250 + 300) * 1000;
    let expect = |kern_av: i64, kern_aa: i64| {
        vec![
            unkerned_av + kern_av * 1000,
            unkerned_av,
            (500 + kern_aa) * 1000,
        ]
    };
    assert_eq!(
        widths,
        vec![
            ("gpos1".to_string(), expect(-120, -30)),
            ("gpos2ext".to_string(), expect(-120, 0)),
            ("kern".to_string(), expect(-80, -30)),
            ("both".to_string(), expect(-120, -30)),
            ("wide".to_string(), expect(-120, -30)),
        ],
    );
}

/// The kerning section, read back pair by pair: the `--debug` face check's `kern=` field
/// lists `left:right:x1:x2` for every glyph pair with a non-zero adjustment. Format 2
/// lists only its one non-zero cell; `GPOS` hides the `kern` table in "both"; the wide
/// ValueRecords read the same values as the narrow ones.
#[test]
fn face_kern_reads_each_fixtures_pairs() {
    let expected = [
        ("gpos1", "1:1:-30:0,1:2:-120:0"),
        ("gpos2ext", "1:2:-120:0"),
        ("kern", "1:1:-30:0,1:2:-80:0"),
        ("both", "1:1:-30:0,1:2:-120:0"),
        ("wide", "1:1:-30:0,1:2:-120:0"),
    ];
    for ((name, font), (want_name, want)) in kerning_fixtures().into_iter().zip(expected) {
        assert_eq!(name, want_name);
        let (check, index) = face_check(&format!("canvas_face_kern_{name}"), font);
        assert_check_clean(&check);
        let kern = check
            .split(' ')
            .find_map(|f| f.strip_prefix("kern="))
            .unwrap_or_else(|| panic!("no `kern=` in `{check}`"));
        assert_eq!(kern, want, "{name}: `{check}`");
        assert_eq!(index[0], 4, "{name}: layout version");
        assert!(index[9] > 0, "{name}: a kerning section");
    }
    let (check, index) = face_check("canvas_face_kern_none", minimal_truetype());
    assert!(
        check.ends_with("kern="),
        "a font with no kerning lists no pair: `{check}`"
    );
    assert_eq!(index[9], 0, "and has no section");
}

/// The rightmost lit column of `"AV"` drawn at size 100 in `font` with `paint`.
fn av_right_edge(name: &str, font: Vec<u8>, paint: &str) -> usize {
    let source = format!(
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("kerned.ttf")
  canvas::present([canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 200.0], text := "AV", fonts := [face], fontSize := 100.0, letterSpacing := 0.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])
END FUNC
"#
    );
    let (frame, _) = render_env_files(name, &source, &[], &[("kerned.ttf", font)]);
    let runs = lit_column_runs(&frame);
    assert!(!runs.is_empty(), "{name}: `AV` drew nothing");
    runs.last().unwrap().1
}

/// The drawn `V` moves left by exactly the kerned amount — −120 units at size 100 in a
/// 1000-unit em is 12 px — on the filled glyph run and on the stroked edges, the two draw
/// paths that place glyphs. Both read the one shaped run (bug-756), so a kern applied to
/// `measureText` alone would leave the drawn `V` where it was.
#[test]
fn a_kerned_pair_draws_at_the_kerned_distance_filled_and_stroked() {
    for (paint, label) in [
        ("canvas::fill(color::rgb(255, 255, 255))", "fill"),
        ("canvas::stroke(color::rgb(255, 255, 255), 2.0)", "stroke"),
    ] {
        let plain = av_right_edge(
            &format!("canvas_kern_draw_plain_{label}"),
            kerning_font(&[]),
            paint,
        );
        let kerned = av_right_edge(
            &format!("canvas_kern_draw_kerned_{label}"),
            kerning_font(&[(b"GPOS", gpos_format1())]),
            paint,
        );
        assert_eq!(
            plain - kerned,
            12,
            "{label}: the kerned `V` must end 12 px left of the unkerned one ({plain} → {kerned})",
        );
    }
}

/// A shipping font kerns as HarfBuzz does. `SFNS.ttf` (SF Pro) carries its `kern`
/// feature as `GPOS` PairPos formats 1 and 2 inside Extension lookups. At size 2048, its
/// `unitsPerEm`, the width is the summed advances in font units. The expected value is
/// HarfBuzz 12.2.0's: `hb-shape --features=-liga,-clig,-calt,kern --no-glyph-names
/// --no-clusters /System/Library/Fonts/SFNS.ttf "AVAWATo"` → advances
/// 1141+1132+1171+1762+1101+920+1087 = 8314 (9364 with `-kern`). The face's SHA-256 was
/// `1c81ead87ca36eee7d51dbe816c3202ba8052fc88228a46de930339c4ec1df22` when this was
/// recorded, and a different build of the font can kern differently, so the test skips,
/// logged, on any other file.
#[cfg(target_os = "macos")]
#[test]
fn a_real_font_kerns_as_harfbuzz_does() {
    const PATH: &str = "/System/Library/Fonts/SFNS.ttf";
    const SHA256: &str = "1c81ead87ca36eee7d51dbe816c3202ba8052fc88228a46de930339c4ec1df22";
    let Ok(bytes) = std::fs::read(PATH) else {
        eprintln!("skipping: {PATH} is not installed");
        return;
    };
    let digest = Command::new("shasum")
        .args(["-a", "256", PATH])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if !digest.starts_with(SHA256) {
        eprintln!(
            "skipping: {PATH} is not the recorded build ({} bytes, `{digest}`)",
            bytes.len()
        );
        return;
    }
    let lines = run(
        "canvas_kern_sfns",
        r#"IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("/System/Library/Fonts/SFNS.ttf")
  io::print(toString(toInt(canvas::measureText(f, 2048.0, "AVAWATo").width * 1000.0)))
END FUNC
"#,
    );
    assert_eq!(lines, vec!["8314000".to_string()]);
}
