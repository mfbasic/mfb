//! Integration test for `mfb build` codegen sub-stage progress (bug-393).
//!
//! The `phase codegen+link <N>ms` line is a post-hoc total, printed only after
//! the whole stage completes. On a large program that stage runs for over a
//! minute of complete silence, so a slow-but-working build is indistinguishable
//! from a hang. Under `-v`/`--verbose` the compiler now emits one `codegen: …`
//! line as it enters each `write_executable` sub-stage, so a long build is
//! visibly progressing and the slow sub-stage is named.
//!
//! The invariants (mirrors `cli_build_verbosity_output.rs`): the sub-stage lines
//! are stderr + verbose-only, so default/`-q` output is unchanged and the
//! `Wrote executable to <path>` stdout line stays verbatim in every mode.

mod common;
use common::temp_project;
use std::process::{Command, Output};

const SOURCE: &str = r#"
IMPORT io

FUNC main AS Integer
  io::print("hi")
  RETURN 0
END FUNC
"#;

/// The codegen sub-stage lines, in the order `write_executable` enters them.
/// The Linux backends loop over libc flavors, so the plan/emit/encode/link
/// lines repeat per flavor — the assertions check first-occurrence order, which
/// holds regardless of flavor count.
const SUBSTAGES: &[&str] = &[
    "codegen: lowering module",
    "codegen: planning + regalloc",
    "codegen: emitting native code",
    "codegen: encoding image",
    "codegen: linking executable",
];

fn build_with(project: &std::path::Path, flags: &[&str]) -> Output {
    let mut cmd = Command::new(common::mfb_exe());
    cmd.arg("build");
    for flag in flags {
        cmd.arg(flag);
    }
    let output = cmd.arg(project).output().expect("run mfb build");
    assert!(
        output.status.success(),
        "build {flags:?} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf8 stdout")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("utf8 stderr")
}

/// `-v`/`--verbose`: every codegen sub-stage line appears on stderr, in the
/// order the pipeline enters them, and the `phase codegen+link …ms` total still
/// follows.
#[test]
fn verbose_build_prints_codegen_substage_lines() {
    let project = temp_project("progress_verbose", SOURCE);
    for flag in ["-v", "--verbose"] {
        let output = build_with(&project, &[flag]);
        let out = stdout(&output);
        let err = stderr(&output);

        // Artifact line is on stdout, unchanged.
        assert!(
            out.lines()
                .any(|line| line.starts_with("Wrote executable to ")),
            "{flag}: artifact line missing from stdout:\n{out}"
        );

        // Each sub-stage line is present, and their first occurrences are in
        // pipeline order.
        let mut last_pos = 0usize;
        for stage in SUBSTAGES {
            let pos = err
                .find(stage)
                .unwrap_or_else(|| panic!("{flag}: missing `{stage}` line in stderr:\n{err}"));
            assert!(
                pos >= last_pos,
                "{flag}: `{stage}` appears out of pipeline order in:\n{err}"
            );
            last_pos = pos;
        }

        // The post-hoc total still prints, after the sub-stage lines.
        let total = err
            .find("phase codegen+link ")
            .unwrap_or_else(|| panic!("{flag}: missing `phase codegen+link` total in:\n{err}"));
        assert!(
            total >= last_pos,
            "{flag}: `phase codegen+link` total must follow the sub-stage lines in:\n{err}"
        );
    }
}

/// Default and `-q` builds must not emit any `codegen:` sub-stage line — they
/// are verbose-only — and the artifact line stays on stdout.
#[test]
fn default_and_quiet_builds_have_no_codegen_substage_lines() {
    let project = temp_project("progress_quiet", SOURCE);
    for flags in [&[][..], &["-q"][..]] {
        let output = build_with(&project, flags);
        let out = stdout(&output);
        let err = stderr(&output);

        assert!(
            out.lines()
                .any(|line| line.starts_with("Wrote executable to ")),
            "{flags:?}: artifact line missing from stdout:\n{out}"
        );
        assert!(
            !err.contains("codegen: "),
            "{flags:?}: codegen sub-stage lines must be verbose-only:\n{err}"
        );
    }
}
