//! Native runtime regression test for bug-135: the `Float ^ Float` operator's
//! kernel was repeated multiplication whose only loop exit was `exponent == 0`,
//! so a large whole exponent (e.g. `1.0e18`) iterated ~1e18 times and effectively
//! hung the program. Routing the operator through the same fdlibm scalar-pow
//! kernel `math::pow` already uses (keeping the whole/non-negative domain guards)
//! makes it terminate. The `^` cases HANG on the unfixed compiler, so each run is
//! bounded by a wall-clock timeout that fails the test if the loop returns.

mod common;
use common::run_bounded;
use common::temp_project;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::fs;

fn build_project(project: &Path) -> PathBuf {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let path = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output executable path");
    PathBuf::from(path)
}

/// Run `executable` with a wall-clock bound. Panics if it does not finish in time
/// (the bug-135 `^` hang trips this). Returns its exit status and captured stdout.

#[test]
fn float_pow_operator_large_exponent_terminates() {
    // `2.0 ^ 1.0e18` overflows; `0.5 ^ 1.0e18` underflows to 0. Both must return
    // PROMPTLY (via the fdlibm kernel) — the pre-fix linear loop would run ~1e18
    // iterations. Small whole exponents still give the exact power.
    let source = "\
IMPORT io
FUNC f(b AS Float, e AS Float) AS String
  LET r = b ^ e
  RETURN toString(r)
  TRAP(x)
    RETURN \"ERR:\" & toString(x.code)
  END TRAP
END FUNC
FUNC main() AS Integer
  io::print(f(2.0, 3.0))
  io::print(f(2.0, 10.0))
  io::print(f(2.0, 1.0e18))
  io::print(f(0.5, 1.0e18))
  RETURN 0
END FUNC
";
    let project = temp_project("bug135_float_pow", source);
    let exe = build_project(&project);
    let (status, stdout) = run_bounded(
        &exe,
        Duration::from_secs(15),
        "the bug-135 `^` linear loop was reintroduced",
    );
    assert!(
        status.success(),
        "program exited unsuccessfully: {status:?}"
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.first().copied(), Some("8.00"), "2.0^3.0");
    assert_eq!(lines.get(1).copied(), Some("1024.00"), "2.0^10.0");
    // 2.0^1e18 overflows -> ErrFloatOverflow (77050015); 0.5^1e18 -> 0.0.
    assert_eq!(
        lines.get(2).copied(),
        Some("ERR:77050015"),
        "2.0^1e18 must trap ErrFloatOverflow, not hang"
    );
    assert_eq!(
        lines.get(3).copied(),
        Some("0.00"),
        "0.5^1e18 underflows to 0"
    );
    let _ = fs::remove_dir_all(&project);
}
