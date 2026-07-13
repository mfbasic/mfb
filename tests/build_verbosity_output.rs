//! Integration tests for `mfb build` verbosity output (plan-36).
//!
//! These shell out to the host `mfb` on a tiny executable project and assert on
//! the three output shapes:
//!   - default (`Normal`):  a `Building …` summary on stderr + the artifact line
//!     on stdout;
//!   - `-q`/`--quiet`:      only the artifact line (no summary, no timings);
//!   - `-v`/`--verbose`:    the summary + one `phase <name> …` line per front-end
//!     stage + the artifact line.
//!
//! The invariant the plan protects: the emitted executable bytes are identical
//! across all three levels (verbosity never reaches codegen), and the
//! `Wrote executable to <path>` line stays verbatim on stdout in every mode.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

const SOURCE: &str = r#"
IMPORT io

FUNC main AS Integer
  io::print("hi")
  RETURN 0
END FUNC
"#;

fn build_with(project: &std::path::Path, flags: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mfb"));
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
                err.lines()
                    .any(|line| line.starts_with(&format!("phase {phase} ")) && line.ends_with("ms")),
                "{flag}: missing `phase {phase} …ms` line in:\n{err}"
            );
        }
    }
}

/// `-q -v` (either order) is rejected as a usage error.
#[test]
fn quiet_and_verbose_conflict_is_rejected() {
    let project = temp_project("verbosity_conflict", SOURCE);
    for args in [&["-q", "-v"][..], &["-v", "-q"][..]] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_mfb"));
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
/// emitted executable is byte-identical across all three levels.
#[test]
fn artifact_bytes_identical_across_verbosity_levels() {
    let project = temp_project("verbosity_bytes", SOURCE);

    let normal = build_with(&project, &[]);
    let normal_bytes = fs::read(artifact_path(&normal)).expect("read normal artifact");

    let quiet = build_with(&project, &["-q"]);
    let quiet_bytes = fs::read(artifact_path(&quiet)).expect("read quiet artifact");

    let verbose = build_with(&project, &["-v"]);
    let verbose_bytes = fs::read(artifact_path(&verbose)).expect("read verbose artifact");

    assert_eq!(
        normal_bytes, quiet_bytes,
        "quiet build produced different artifact bytes than the default build"
    );
    assert_eq!(
        normal_bytes, verbose_bytes,
        "verbose build produced different artifact bytes than the default build"
    );
}
