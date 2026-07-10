//! Native runtime regression tests for bug-61: two numeric-codegen defects in
//! `builder_numeric.rs` / `builder_fixed_math.rs`.
//!
//! (1) `Fixed` division of the exact representable minimum (`-2147483648.0`,
//!     raw `i64::MIN`) wrongly trapped `ErrOverflow` because the quotient was
//!     range-checked by magnitude (`2^31 > 2^31 - 1`) instead of by the signed
//!     value. The sibling `*` already admitted `-2^31`.
//! (2) The integer and `Fixed` `^` operators (and `math::pow` for `Fixed`) used
//!     a linear countdown loop, so a bounded base (`|base| <= 1`) with a huge
//!     exponent never overflowed and iterated the full exponent — an effective
//!     hang / CPU DoS.
//!
//! These build a tiny program with the host `mfb`, run the produced executable
//! under a wall-clock bound, and assert on its output. The `^` cases HANG on the
//! unfixed compiler, so every run is bounded by a timeout that fails the test if
//! the loop is ever reintroduced.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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

fn build_project(project: &Path) -> PathBuf {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
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

/// Run `executable` with a wall-clock bound. Panics if it does not finish in
/// time (the bug-61 `^` hang would trip this). Returns its exit status and
/// captured stdout.
fn run_bounded(executable: &Path, timeout: Duration) -> (ExitStatus, String) {
    let mut child = Command::new(executable)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn executable");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            let mut stdout = String::new();
            if let Some(mut pipe) = child.stdout.take() {
                pipe.read_to_string(&mut stdout).ok();
            }
            return (status, stdout);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "execution exceeded {timeout:?} — the bounded-base `^` loop appears \
                 to iterate the full exponent (bug-61 hang reintroduced)"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn build_and_run(name: &str, source: &str) -> (ExitStatus, String) {
    run_bounded(
        &build_project(&temp_project(name, source)),
        Duration::from_secs(30),
    )
}

// --- item (1): Fixed division of the representable minimum ---------------------

/// The exact result `-2147483648.0` (raw `i64::MIN`) is representable and must
/// be returned, not trapped. Pre-fix this exits 255 with `ErrOverflow`.
#[test]
fn native_fixed_divide_admits_representable_minimum() {
    let (status, out) = build_and_run(
        "fixed_div_min",
        r#"
IMPORT io

FUNC main AS Integer
  io::print(toString(toFixed(-2147483648) / toFixed(1)))
  io::print(toString(toFixed(-2147483648) / toFixed(2)))
  io::print(toString(toFixed(-2147483648) / toFixed(-2)))
  io::print(toString(toFixed(-2147483647) / toFixed(1)))
  io::print(toString(toFixed(2147483647) / toFixed(1)))
  RETURN 0
END FUNC
"#,
    );
    assert!(status.success(), "program trapped unexpectedly");
    assert_eq!(
        out,
        "-2147483648.00\n-1073741824.00\n1073741824.00\n-2147483647.00\n2147483647.00\n"
    );
}

/// The genuinely unrepresentable `+2^31` result (`-2^31 / -1`) must still trap,
/// and so must an out-of-range magnitude. Guards against over-widening the fix.
#[test]
fn native_fixed_divide_still_traps_positive_overflow() {
    let (status, _out) = build_and_run(
        "fixed_div_pos_overflow",
        r#"
IMPORT io

FUNC main AS Integer
  io::print(toString(toFixed(-2147483648) / toFixed(-1)))
  RETURN 0
END FUNC
"#,
    );
    assert!(
        !status.success(),
        "+2147483648.0 is unrepresentable and must still trap ErrOverflow"
    );
}

#[test]
fn native_fixed_divide_still_traps_out_of_range_magnitude() {
    let (status, _out) = build_and_run(
        "fixed_div_big_overflow",
        r#"
IMPORT io

FUNC main AS Integer
  io::print(toString(toFixed(2000000) / toFixed(0.0001)))
  RETURN 0
END FUNC
"#,
    );
    assert!(
        !status.success(),
        "a quotient of magnitude 2e10 must still trap ErrOverflow"
    );
}

// --- item (2): bounded-base `^` must not iterate the exponent ------------------

/// Integer `^` with a base in {-1, 0, 1} has bounded powers; pre-fix it loops
/// up to `i64::MAX` times (a true hang for the `1 ^ i64::MAX` case). Post-fix it
/// resolves in closed form. `|base| >= 2` is unchanged (still terminates via the
/// overflow trap).
#[test]
fn native_integer_pow_bounded_base_terminates() {
    let (status, out) = build_and_run(
        "int_pow_bounded",
        r#"
IMPORT io

FUNC main AS Integer
  io::print(toString(1 ^ 9223372036854775807))
  io::print(toString(0 ^ 9223372036854775807))
  io::print(toString(0 ^ 0))
  io::print(toString(-1 ^ 9223372036854775807))
  io::print(toString(-1 ^ 9223372036854775806))
  io::print(toString(2 ^ 10))
  io::print(toString(3 ^ 0))
  io::print(toString(5 ^ 1))
  RETURN 0
END FUNC
"#,
    );
    assert!(status.success(), "program trapped unexpectedly");
    assert_eq!(out, "1\n0\n1\n-1\n1\n1024\n1\n5\n");
}

/// The `Fixed` `^` operator with `±1.0` base and a large exponent. Pre-fix this
/// iterates the exponent (a multi-second DoS) and returns the wrong value; the
/// closed-form fast path returns the correct value immediately.
#[test]
fn native_fixed_pow_operator_unit_base_terminates() {
    let (status, out) = build_and_run(
        "fixed_pow_op_unit",
        r#"
IMPORT io

FUNC main AS Integer
  io::print(toString(toFixed(1) ^ toFixed(2000000000)))
  io::print(toString(toFixed(-1) ^ toFixed(2000000001)))
  io::print(toString(toFixed(-1) ^ toFixed(2000000000)))
  RETURN 0
END FUNC
"#,
    );
    assert!(status.success(), "program trapped unexpectedly");
    assert_eq!(out, "1.00\n-1.00\n1.00\n");
}

/// `math::pow` for `Fixed` shares the same loop. Repeat the `±1.0`-base call
/// enough times that the pre-fix linear loop (~2.1e9 iterations each) far
/// exceeds the timeout, while the post-fix closed form stays instant. Also
/// asserts the value is correct.
#[test]
fn native_math_pow_fixed_unit_base_terminates() {
    let (status, out) = build_and_run(
        "math_pow_fixed_unit",
        r#"
IMPORT io
IMPORT math

FUNC main AS Integer
  MUT acc AS Fixed = toFixed(0)
  FOR i = 1 TO 40
    acc = acc + math::pow(toFixed(-1), toFixed(2147483647))
  NEXT
  io::print(toString(acc))
  io::print(toString(math::pow(toFixed(1), toFixed(2147483647))))
  RETURN 0
END FUNC
"#,
    );
    assert!(status.success(), "program trapped unexpectedly");
    // 40 * (-1)^odd == -40.0; and 1.0^n == 1.0.
    assert_eq!(out, "-40.00\n1.00\n");
}
