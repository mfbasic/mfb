//! plan-161-B (bug-740 mechanism (b)): at the default `-O1` a leaf helper's
//! locals, parameters and operand temporaries live in registers, not in frame
//! slots — the "Stack slot promotion" row plus the allocator's live-across-call
//! rule. brogue's `gi` (`x * DROWS + y`) stored and reloaded every one of them.
//!
//! The pin is the emitted `.ncode` for AArch64: at `-O1` the helper's body has
//! no `sp`-based word load or store except the `lr` save and restore; at `-O0`
//! (the control, so the assertion is not vacuous) it still round-trips through
//! the frame. The program's output is the same at both levels, including an
//! overflow raised inside the helper.

#[path = "../common/mod.rs"]
mod common;
use common::{mfb_exe, temp_project};
use std::path::Path;
use std::process::Command;

const SOURCE: &str = "\
IMPORT io\n\
LET ROWS AS Integer = 29\n\
\n\
FUNC cell(x AS Integer, y AS Integer) AS Integer\n\
  RETURN x * ROWS + y\n\
END FUNC\n\
\n\
FUNC main() AS Nothing\n\
  io::print(toString(cell(2, 3)))\n\
  LET n AS Integer = cell(9223372036854775807, 1) TRAP(e)\n\
    io::print(\"code=\" & toString(e.code) & \" line=\" & toString(e.source.line) & \" char=\" & toString(e.source.char))\n\
    RETURN\n\
  END TRAP\n\
  io::print(toString(n))\n\
END FUNC\n";

fn build(project: &Path, level: u8, extra: &[&str]) -> String {
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg(format!("-O{level}"))
        .args(extra)
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "-O{level} build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 build output")
}

/// The `sp`-based word loads/stores in `function`'s body, other than the
/// `lr` save/restore.
fn frame_traffic(project: &Path, level: u8, function: &str) -> usize {
    build(
        project,
        level,
        &["-q", "-ncode", "-target", "macos-aarch64"],
    );
    let dump = std::fs::read_dir(project)
        .expect("project dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "ncode"))
        .expect("an .ncode dump");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dump).expect("read dump")).expect("json");
    let body = json["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["name"] == function)
        .unwrap_or_else(|| panic!("no function {function}"));
    body["instructions"]
        .as_array()
        .expect("instructions")
        .iter()
        .filter(|i| {
            (i["op"] == "ldr_u64" && i["dst"] != "lr" || i["op"] == "str_u64" && i["src"] != "lr")
                && i["base"] == "sp"
        })
        .count()
}

fn run(project: &Path, level: u8) -> String {
    let stdout = build(project, level, &[]);
    let executable = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable");
    let run = Command::new(executable).output().expect("run program");
    assert!(run.status.success(), "-O{level} program failed");
    String::from_utf8(run.stdout).expect("utf8 program output")
}

#[test]
fn a_leaf_helper_keeps_its_values_in_registers() {
    let project = temp_project("stack_slot_promotion", SOURCE);
    assert!(
        frame_traffic(&project, 0, "cell") > 0,
        "control: at -O0 `cell` round-trips its values through the frame"
    );
    assert_eq!(
        frame_traffic(&project, 1, "cell"),
        0,
        "at -O1 `cell` touches the frame only to save lr"
    );
}

#[test]
fn promotion_changes_no_result() {
    let project = temp_project("stack_slot_promotion_run", SOURCE);
    let unoptimized = run(&project, 0);
    assert_eq!(unoptimized, "61\ncode=77050010 line=5 char=10\n");
    assert_eq!(run(&project, 1), unoptimized);
}
