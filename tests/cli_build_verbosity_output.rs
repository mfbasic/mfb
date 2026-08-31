//! Integration tests for `mfb build` verbosity output (plan-36).
//!
//! These shell out to the host `mfb` on a tiny executable project and assert on
//! the three output shapes:
//!   - default (`Normal`):  a `Building …` summary on stderr + the artifact line
//!     on stdout;
//!   - `-q`/`--quiet`:      only the artifact line (no summary, no timings);
//!   - `-v`/`--verbose`:    the summary + one `phase <name> …` line per front-end
//!     stage + the artifact line;
//!   - `-vv`/`-v -v`:       everything `-v` prints plus the `crate::trace`
//!     compile-profiler report (span tree, leaderboards, counters).
//!
//! The invariant the plan protects: the emitted executable bytes are identical
//! across all four levels (verbosity never reaches codegen), and the
//! `Wrote executable to <path>` line stays verbatim on stdout in every mode.

mod common;
use common::temp_project;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const SOURCE: &str = r#"
IMPORT io

FUNC main AS Integer
  io::print("hi")
  RETURN 0
END FUNC
"#;

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

fn artifact_path(output: &Output) -> PathBuf {
    let out = stdout(output);
    let path = out
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output executable path");
    PathBuf::from(path)
}

/// Default build: the summary appears (on stderr) and the artifact line is on
/// stdout, verbatim.
#[test]
fn default_build_prints_summary_and_artifact_line() {
    let project = temp_project("verbosity_default", SOURCE);
    let output = build_with(&project, &[]);

    let out = stdout(&output);
    let err = stderr(&output);

    // Artifact line is on stdout, unchanged.
    assert!(
        out.lines()
            .any(|line| line.starts_with("Wrote executable to ")),
        "artifact line missing from stdout:\n{out}"
    );
    // Summary line is on stderr, deterministic shape.
    assert!(
        err.contains("Building verbosity_default (executable) for "),
        "summary line missing from stderr:\n{err}"
    );
    // No phase timings in the default build.
    assert!(
        !err.contains("phase "),
        "default build must not emit phase timings:\n{err}"
    );
}

/// `-q`/`--quiet`: only the artifact line, no summary, no timings.
#[test]
fn quiet_build_prints_only_the_artifact_line() {
    let project = temp_project("verbosity_quiet", SOURCE);
    for flag in ["-q", "--quiet"] {
        let output = build_with(&project, &[flag]);
        let out = stdout(&output);
        let err = stderr(&output);

        assert!(
            out.lines()
                .any(|line| line.starts_with("Wrote executable to ")),
            "{flag}: artifact line missing from stdout:\n{out}"
        );
        assert!(
            !err.contains("Building "),
            "{flag}: quiet build must not print the summary:\n{err}"
        );
        assert!(
            !err.contains("phase "),
            "{flag}: quiet build must not print phase timings:\n{err}"
        );
    }
}

/// `-v`/`--verbose`: the summary plus one `phase <name>` line per front-end
/// stage, matched by name (never by the non-deterministic ms value), plus the
/// artifact line.
#[test]
fn verbose_build_prints_phase_lines() {
    let project = temp_project("verbosity_verbose", SOURCE);
    for flag in ["-v", "--verbose"] {
        let output = build_with(&project, &[flag]);
        let out = stdout(&output);
        let err = stderr(&output);

        assert!(
            out.lines()
                .any(|line| line.starts_with("Wrote executable to ")),
            "{flag}: artifact line missing from stdout:\n{out}"
        );
        assert!(
            err.contains("Building verbosity_verbose (executable) for "),
            "{flag}: summary line missing from stderr:\n{err}"
        );
        for phase in ["parse", "resolve", "verify", "codegen+link"] {
            assert!(
                err.lines().any(
                    |line| line.starts_with(&format!("phase {phase} ")) && line.ends_with("ms")
                ),
                "{flag}: missing `phase {phase} …ms` line in:\n{err}"
            );
        }
    }
}

/// `-vv` / `-v -v`: everything `-v` prints, plus the compile-profiler report —
/// the span tree, the per-function leaderboard, and the size counters — and a
/// `codegen: <stage> <N>ms` completion line for each streamed sub-stage.
#[test]
fn trace_build_prints_the_compile_profiler_report() {
    let project = temp_project("verbosity_trace", SOURCE);
    for flags in [&["-vv"][..], &["-v", "-v"][..]] {
        let output = build_with(&project, flags);
        let out = stdout(&output);
        let err = stderr(&output);

        assert!(
            out.lines()
                .any(|line| line.starts_with("Wrote executable to ")),
            "{flags:?}: artifact line missing from stdout:\n{out}"
        );
        // `-vv` is a superset of `-v`: the phase lines are still there.
        for phase in ["parse", "resolve", "verify", "codegen+link"] {
            assert!(
                err.lines().any(
                    |line| line.starts_with(&format!("phase {phase} ")) && line.ends_with("ms")
                ),
                "{flags:?}: missing `phase {phase} …ms` line in:\n{err}"
            );
        }
        // The three report sections.
        for section in [
            "--- trace: span tree ---",
            "--- trace: slowest lower_function",
            "--- trace: counters ---",
        ] {
            assert!(
                err.contains(section),
                "{flags:?}: missing `{section}` in:\n{err}"
            );
        }
        // Every top-level phase gets a tree row, and each has at least one
        // nested row beneath it.
        //
        // Deliberately not asserting a *named* deep row (`monomorphize`, say):
        // on a project this small every sub-step finishes in well under a
        // millisecond and the renderer folds it into the `(N rows under 1.0ms)`
        // summary. That fold is the feature working, not a missing span, so
        // pinning a name here would make the test a hostage to how fast the
        // machine happens to be.
        for phase in ["parse", "resolve", "verify", "codegen+link"] {
            let row = err
                .lines()
                .position(|line| line.starts_with(phase))
                .unwrap_or_else(|| panic!("{flags:?}: missing `{phase}` tree row in:\n{err}"));
            let next = err.lines().nth(row + 1).unwrap_or("");
            assert!(
                next.starts_with("  "),
                "{flags:?}: `{phase}` has no nested rows; next line was `{next}` in:\n{err}"
            );
        }
        // The counters are written from the *deepest* instrumentation points —
        // `NIR functions` from the shared NIR lowering, `machine instructions`
        // from per-function codegen — so their presence is the timing-independent
        // proof that the deep hooks ran, which the folded tree rows cannot give.
        for counter in ["NIR functions", "machine instructions", "IR functions"] {
            assert!(
                err.lines().any(|line| line.starts_with(counter)),
                "{flags:?}: missing `{counter}` counter in:\n{err}"
            );
        }
        // Each streamed codegen sub-stage gets a completion time, including the
        // last one (the stage is closed explicitly once codegen returns).
        for stage in ["emitting native code", "linking executable"] {
            assert!(
                err.lines()
                    .any(|line| line.starts_with(&format!("codegen: {stage} "))
                        && line.ends_with("ms")),
                "{flags:?}: missing `codegen: {stage} …ms` completion line in:\n{err}"
            );
        }
    }
}

/// The profiler is `-vv`-only: `-v` keeps its exact pre-existing output, with
/// no report and no per-stage completion times.
#[test]
fn verbose_build_has_no_trace_report() {
    let project = temp_project("verbosity_no_trace", SOURCE);
    let err = stderr(&build_with(&project, &["-v"]));
    assert!(
        !err.contains("--- trace:"),
        "the compile profiler must be -vv-only:\n{err}"
    );
    // A bare `codegen: <stage>` line, never a `codegen: <stage> <N>ms` one.
    assert!(
        !err.lines()
            .any(|line| line.starts_with("codegen: ") && line.ends_with("ms")),
        "-v must not print per-stage completion times:\n{err}"
    );
}

/// `-q -v` (either order) is rejected as a usage error.
#[test]
fn quiet_and_verbose_conflict_is_rejected() {
    let project = temp_project("verbosity_conflict", SOURCE);
    for args in [&["-q", "-v"][..], &["-v", "-q"][..]] {
        let mut cmd = Command::new(common::mfb_exe());
        cmd.arg("build");
        for a in args {
            cmd.arg(a);
        }
        let output = cmd.arg(&project).output().expect("run mfb build");
        assert!(
            !output.status.success(),
            "{args:?} should fail but succeeded"
        );
        let err = stderr(&output);
        assert!(
            err.contains("at most one of -q / -v"),
            "{args:?}: expected conflict message, got:\n{err}"
        );
    }
}

/// The invariant that matters most: verbosity never reaches codegen, so the
/// emitted executable is byte-identical across all four levels.
///
/// `-vv` is the one that could plausibly break it — the compile profiler opens
/// spans *inside* codegen — so it is covered here rather than left to the
/// argument that a timing sink "obviously" cannot change anything.
#[test]
fn artifact_bytes_identical_across_verbosity_levels() {
    let project = temp_project("verbosity_bytes", SOURCE);

    let normal = build_with(&project, &[]);
    let normal_bytes = fs::read(artifact_path(&normal)).expect("read normal artifact");

    let quiet = build_with(&project, &["-q"]);
    let quiet_bytes = fs::read(artifact_path(&quiet)).expect("read quiet artifact");

    let verbose = build_with(&project, &["-v"]);
    let verbose_bytes = fs::read(artifact_path(&verbose)).expect("read verbose artifact");

    let trace = build_with(&project, &["-vv"]);
    let trace_bytes = fs::read(artifact_path(&trace)).expect("read trace artifact");

    assert_eq!(
        normal_bytes, quiet_bytes,
        "quiet build produced different artifact bytes than the default build"
    );
    assert_eq!(
        normal_bytes, verbose_bytes,
        "verbose build produced different artifact bytes than the default build"
    );
    assert_eq!(
        normal_bytes, trace_bytes,
        "-vv (compile profiler) produced different artifact bytes than the default build"
    );
}
