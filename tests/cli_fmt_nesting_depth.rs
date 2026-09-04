//! bug-502 (audit-3 FE-02): `mfb fmt` re-indented by nesting depth with no
//! ceiling, so a deeply nested source inflated quadratically on the way out
//! (40 KB → 8 MB here; 1.3 MB → 8.2 GB in the audit) and the result was written
//! back over the user's file with a plain `fs::write` — a run that died mid-write
//! left the source truncated. The formatter now refuses past
//! `fmt::MAX_NESTING_DEPTH` open blocks with the parser's block-depth diagnostic,
//! and every rewrite goes through a sibling temporary file + rename, so the
//! original is replaced only by a complete result.
//!
//! Drives the real `mfb` binary: a hostile tower must exit 1 with the located
//! diagnostic and leave the file byte-for-byte intact (no temporary beside it);
//! an ordinary file must still format exactly as before.

use std::process::{Command, Output};

mod common;
use common::*;

/// `SUB main()` wrapping `opens` nested `IF TRUE THEN … END IF` blocks.
fn tower(opens: usize) -> String {
    let mut source = String::from("SUB main()\n");
    for _ in 0..opens {
        source.push_str("IF TRUE THEN\n");
    }
    source.push_str("io::print(\"x\")\n");
    for _ in 0..opens {
        source.push_str("END IF\n");
    }
    source.push_str("END SUB\n");
    source
}

fn fmt_file(file: &std::path::Path, extra: &[&str]) -> Output {
    Command::new(mfb_exe())
        .arg("fmt")
        .args(extra)
        .arg(file)
        .output()
        .expect("run mfb fmt")
}

fn sibling_names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read source dir")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn fmt_refuses_a_too_deep_file_and_leaves_it_intact() {
    let hostile = tower(2000);
    let project = temp_project("bug502_fmt_too_deep", &hostile);
    let file = project.join("src").join("main.mfb");

    let output = fmt_file(&file, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a too-deep file must be refused with exit 1, not formatted or killed.\n\
         status: {}\nstderr:\n{stderr}",
        output.status
    );
    assert!(
        stderr.contains("MFB_PARSE_BLOCK_TOO_DEEP") && stderr.contains("nesting is too deep"),
        "expected the located block-depth diagnostic, got:\n{stderr}"
    );
    // The diagnostic is itself bounded: three echoed lines, not the tower.
    assert!(
        stderr.len() < 4096,
        "stderr is {} bytes:\n{stderr}",
        stderr.len()
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        hostile,
        "the source must be byte-for-byte untouched after a refusal"
    );
    assert_eq!(sibling_names(&project.join("src")), vec!["main.mfb"]);
}

#[test]
fn fmt_check_reports_a_too_deep_file_without_writing() {
    let hostile = tower(2000);
    let project = temp_project("bug502_fmt_too_deep_check", &hostile);
    let file = project.join("src").join("main.mfb");

    let output = fmt_file(&file, &["--check"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "status: {}\n{stderr}",
        output.status
    );
    assert!(stderr.contains("MFB_PARSE_BLOCK_TOO_DEEP"), "{stderr}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), hostile);
}

#[test]
fn fmt_still_formats_an_ordinary_file_identically() {
    // A well-formed, moderately nested program: the cap must not change one
    // byte of what the formatter produced before it existed.
    let source = "import io\nfunc main as Integer\nfor i = 1 to 3\nif i > 1 then\nio::print(toString(i))\nend if\nnext\nreturn 0\nend func\n";
    let expected = "IMPORT io\nFUNC main AS Integer\n  FOR i = 1 TO 3\n    IF i > 1 THEN\n      io::print(toString(i))\n    END IF\n  NEXT\n  RETURN 0\nEND FUNC\n";
    let project = temp_project("bug502_fmt_ordinary", source);
    let file = project.join("src").join("main.mfb");

    let output = fmt_file(&file, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), expected);
    assert_eq!(sibling_names(&project.join("src")), vec!["main.mfb"]);
    // A second run is a fixed point.
    let output = fmt_file(&file, &["--check"]);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn fmt_formats_a_tower_exactly_at_the_cap() {
    // `SUB` is frame 1, so 1023 `IF`s fill the stack to the cap without
    // crossing it: still formatted, with bounded output.
    let source = tower(1023);
    let project = temp_project("bug502_fmt_at_cap", &source);
    let file = project.join("src").join("main.mfb");

    let output = fmt_file(&file, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&file).unwrap();
    assert!(formatted.starts_with("SUB main()\n  IF TRUE THEN\n    IF TRUE THEN\n"));
    assert!(formatted.len() > source.len());
}
