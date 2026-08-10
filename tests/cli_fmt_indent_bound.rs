//! bug-220: `mfb fmt --indent` accepted an unbounded value, so `indent_str`'s
//! `" ".repeat(level * width)` could be driven into a multiply/capacity-overflow
//! panic or a multi-GB allocation. `parse_indent` now clamps to `0..=256`
//! (mirroring `parse_spec_width`) and reports a clean range error instead.
//!
//! This drives the real `mfb` binary with an absurd `--indent` on a nested file
//! and asserts it exits with a *bounded* error (a normal non-zero exit code with
//! a range message), never dies by signal (panic/abort) and never hangs on a
//! huge allocation.

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

mod common;
use common::*;

/// A file with real nesting so a pre-fix run would multiply `level * width`.
const NESTED_SOURCE: &str = "IMPORT io\n\
FUNC main AS Integer\n\
  FOR i = 1 TO 3\n\
    IF i > 1 THEN\n\
      io::print(toString(i))\n\
    END IF\n\
  NEXT\n\
  RETURN 0\n\
END FUNC\n";

fn assert_bounded_range_error(output: &std::process::Output, indent: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.code().is_some(),
        "--indent={indent}: mfb was killed by signal {:?} (overflow panic?) \
         instead of reporting a bounded error.\nstderr:\n{stderr}",
        output.status.signal()
    );
    assert!(
        !output.status.success(),
        "--indent={indent}: an out-of-range indent must be an error, not success"
    );
    assert!(
        stderr.contains("--indent") && stderr.contains("256"),
        "--indent={indent}: expected a clean `must be between 0 and 256` range \
         error, got:\nstderr:\n{stderr}"
    );
}

#[test]
fn fmt_rejects_indent_at_usize_max() {
    let project = temp_project("bug220_fmt_indent_max", NESTED_SOURCE);
    let file = project.join("src").join("main.mfb");

    let output = Command::new(mfb_exe())
        .args(["fmt", "--indent=18446744073709551615"])
        .arg(&file)
        .output()
        .expect("run mfb fmt --indent=<usize::MAX>");

    assert_bounded_range_error(&output, "18446744073709551615");
}

#[test]
fn fmt_rejects_fat_fingered_large_indent() {
    let project = temp_project("bug220_fmt_indent_big", NESTED_SOURCE);
    let file = project.join("src").join("main.mfb");

    let output = Command::new(mfb_exe())
        .args(["fmt", "--indent=100000000"])
        .arg(&file)
        .output()
        .expect("run mfb fmt --indent=100000000");

    assert_bounded_range_error(&output, "100000000");
}
