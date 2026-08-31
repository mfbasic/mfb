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

use std::process::Command;

/// Build a `--app` program, run it headless, and return its stdout lines.
fn run(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let build = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .arg(&project)
        .output()
        .expect("run mfb build -app");
    assert!(
        build.status.success(),
        "mfb build -app failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );

    let binary = app_binary(&project, name);
    let out = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        out.status.success(),
        "program exited {:?}:\n{}\n{}",
        out.status.code(),
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

/// The built executable, which on macOS is inside an `.app` bundle.
fn app_binary(project: &std::path::Path, name: &str) -> std::path::PathBuf {
    let bundle = project
        .join("build")
        .join(format!("{name}.app"))
        .join("Contents")
        .join("MacOS")
        .join(name);
    if bundle.exists() {
        return bundle;
    }
    let plain = project.join("build").join(name);
    if plain.exists() {
        return plain;
    }
    project
        .join("build")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
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
  LET r AS FontRef = canvas::fontRef(f)
  IF r.id = 0 THEN
    RETURN label & ": accepted with a zero handle"
  END IF
  canvas::destroyFont(f)
  RETURN label & ": accepted"
END FUNC

SUB main()
  app::setMode(Mode.Canvas)
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
  app::setMode(Mode.Canvas)
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
