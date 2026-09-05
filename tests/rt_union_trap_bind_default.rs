//! Regression tests for bug-444: binding a fallible call whose return type is a
//! **data union** to a `LET`/`MUT` with an inline `TRAP` failed native codegen
//! with `native code cannot materialize default value for type '<Union>'`.
//!
//! The `... TRAP ... END TRAP` desugar emits a `bind $trap_valN : <Union> =
//! <default>` temp, and `lower_default_value` had arms for scalars, `String`,
//! collections, resources, resource unions, and records — but a data union fell
//! into the record arm, missed `record_fields`, and hard-errored. The fix adds a
//! data-union arm: default the first statically-defaultable variant's record and
//! wrap it in the canonical flat union layout (`{tag@0, size@8, record@16}`,
//! plan-02 §4.3). The synthesized default is never observed by a program — on
//! the error path the `RECOVER` value (or handler divergence) supersedes it —
//! so these tests assert the *bind compiles* and the observable values are the
//! parse result / RECOVER value.
//!
//! Contrast cases that always worked (no-TRAP auto-propagate, record/scalar/
//! resource-union returns through TRAP) are covered by the existing suite; the
//! cases here are exactly the ones that failed before the fix.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug444_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    // Hermetic key store so the result is machine-independent (see test-accept.sh).
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug444_home"));
    command
}

/// Scaffold an executable project with `source` as `src/main.mfb`, build it,
/// and return the built executable path. Panics with the build output if the
/// build fails — before the bug-444 fix that is exactly the
/// "cannot materialize default value" codegen error.
fn build(root: &Path, source: &str) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"bug444\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    let out = mfb()
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "expected the union-through-TRAP bind to build, but it failed:\n{combined}"
    );
    let exe = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build reported no executable path")
        .trim()
        .to_string();
    // The path is reported relative to the project dir.
    root.join(exe.strip_prefix("./").unwrap_or(&exe))
}

fn run(exe: &Path) -> String {
    let out = Command::new(exe).output().expect("run built program");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "built program failed (exit {:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code()
    );
    stdout
}

/// The bug doc's minimal repro (RECOVER form): `json::parse` is the common
/// fallible union-returning builtin. The good parse binds the parsed value; the
/// failing parse binds the RECOVER value — the synthesized default is never
/// observed on either path.
#[test]
fn json_parse_union_trap_recover_binds() {
    let root = unique_root("json_recover");
    let exe = build(
        &root,
        concat!(
            "IMPORT io\n",
            "IMPORT json\n\n",
            "FUNC main() AS Integer\n",
            "  LET d AS json::Json = json::parse(\"{\\u{22}a\\u{22}:1}\") TRAP(e)\n",
            "    RECOVER json::JsonNull[NOTHING]\n",
            "  END TRAP\n",
            "  LET bad AS json::Json = json::parse(\"{oops\") TRAP(e)\n",
            "    RECOVER json::JsonNull[NOTHING]\n",
            "  END TRAP\n",
            "  io::print(json::stringify(d))\n",
            "  io::print(json::stringify(bad))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert!(
        stdout.contains("{\"a\":1}"),
        "good parse should bind the parsed value:\n{stdout}"
    );
    assert!(
        stdout.contains("null"),
        "failed parse should bind the RECOVER value:\n{stdout}"
    );
}

/// The diverging-handler form fails identically before the fix: the handler
/// never RECOVERs, but the desugar still creates the defaulted trap temp.
#[test]
fn json_parse_union_trap_diverging_handler_builds() {
    let root = unique_root("json_diverge");
    let exe = build(
        &root,
        concat!(
            "IMPORT io\n",
            "IMPORT json\n\n",
            "FUNC main() AS Integer\n",
            "  LET d AS json::Json = json::parse(\"[1,2]\") TRAP(e)\n",
            "    RETURN 3\n",
            "  END TRAP\n",
            "  io::print(json::stringify(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert!(
        stdout.contains("[1,2]"),
        "successful parse should bind through the diverging-form TRAP:\n{stdout}"
    );
}

/// A user-defined data union has the same gap: the default path must be generic
/// over data unions, not special-cased to `json::Json`.
#[test]
fn user_union_trap_bind_recovers_and_matches() {
    let root = unique_root("user_union");
    let exe = build(
        &root,
        concat!(
            "IMPORT io\n\n",
            "TYPE Circle\n",
            "  radius AS Integer\n",
            "END TYPE\n",
            "TYPE Rect\n",
            "  w AS Integer\n",
            "  h AS Integer\n",
            "END TYPE\n\n",
            "UNION Shape\n",
            "  Circle\n",
            "  Rect\n",
            "END UNION\n\n",
            "FUNC makeShape(n AS Integer) AS Shape\n",
            "  IF n < 0 THEN FAIL error(90004440, \"negative\")\n",
            "  RETURN Circle[n]\n",
            "END FUNC\n\n",
            "FUNC score(s AS Shape) AS Integer\n",
            "  MATCH s\n",
            "    CASE Circle(c)\n",
            "      RETURN c.radius\n",
            "    CASE Rect(r)\n",
            "      RETURN r.w + r.h\n",
            "  END MATCH\n",
            "END FUNC\n\n",
            "FUNC main() AS Integer\n",
            "  LET ok AS Shape = makeShape(7) TRAP(e)\n",
            "    RECOVER Rect[1, 2]\n",
            "  END TRAP\n",
            "  LET bad AS Shape = makeShape(-1) TRAP(e)\n",
            "    RECOVER Rect[20, 22]\n",
            "  END TRAP\n",
            "  io::print(\"ok=\" & toString(score(ok)))\n",
            "  io::print(\"bad=\" & toString(score(bad)))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert!(
        stdout.contains("ok=7"),
        "successful call should bind the real value through TRAP:\n{stdout}"
    );
    assert!(
        stdout.contains("bad=42"),
        "failing call should bind the RECOVER value, MATCH-readable:\n{stdout}"
    );
}
