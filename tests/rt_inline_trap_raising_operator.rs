//! Regression tests for bug-471: an inline `TRAP` silently failed to cover a
//! raising **operator** inside the trapped expression.
//!
//! bug-457 taught `ir::lower::lower_inline_trap` to lift every fallible *call*
//! nested in the trapped expression into its own `CallResult` + `If ResultIsOk`
//! check. An *operator* that raises — a division by zero, an arithmetic
//! overflow, a float that overflows to infinity — was left in place, so its
//! error was emitted with no capture active
//! (`emit_error_register_return`'s `raw_result_capture` is `None`) and
//! auto-propagated straight past the handler to the function-level trap. Same
//! escape class as bug-457, different mechanism.
//!
//! `mfb spec language error-model` §8.4 scopes an inline `TRAP` to the whole
//! expression and draws no distinction between a call and an operator, so an
//! error raised while evaluating the expression must reach the handler
//! regardless of which node raised it.
//!
//! The fix gives the IR a `Checked` node — "evaluate this value with its
//! domain-error exits captured, yielding `Result OF T`" — and lifts every
//! unconditionally-evaluated raising operator into its own checked bind, joining
//! bug-457's shared `$trap_failed` chain. The one shape it cannot desugar — a
//! raising operator in a **short-circuited** operand, where hoisting would
//! evaluate it unconditionally — is rejected with
//! `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`, exactly as bug-457 rejects a fallible
//! call there.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug471_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    // Hermetic key store so the result is machine-independent (see test-accept.sh).
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug471_home"));
    command
}

/// The helper prelude every case shares. `two`/`outer` are infallible so the
/// only thing that can fail in a scrutinee is the operator under test; `inner`
/// is fallible so an operator can be mixed with bug-457's shape; `note`/`shout`
/// print so evaluation order and skipping are observable.
const PRELUDE: &str = concat!(
    "IMPORT io\n\n",
    "FUNC inner(n AS Integer) AS Integer\n",
    "  IF n < 0 THEN FAIL error(90000001, \"inner failed\")\n",
    "  RETURN n * 2\n",
    "END FUNC\n\n",
    "FUNC outer(n AS Integer) AS Integer\n",
    "  RETURN n + 1\n",
    "END FUNC\n\n",
    "FUNC two(a AS Integer, b AS Integer) AS Integer\n",
    "  RETURN a * 100 + b\n",
    "END FUNC\n\n",
    "FUNC twoF(a AS Float, b AS Float) AS Float\n",
    "  RETURN b\n",
    "END FUNC\n\n",
    "FUNC pick(flag AS Boolean, b AS Integer) AS Integer\n",
    "  RETURN b\n",
    "END FUNC\n\n",
    "FUNC note(tag AS String, v AS Integer) AS Integer\n",
    "  io::print(\"eval \" & tag)\n",
    "  RETURN v\n",
    "END FUNC\n\n",
    "FUNC shout(tag AS String, v AS Integer) AS Integer\n",
    "  io::print(\"ran \" & tag)\n",
    "  RETURN v\n",
    "END FUNC\n\n",
);

/// Scaffold an executable project with `PRELUDE + source` as `src/main.mfb` and
/// run `mfb build` over it, returning the raw build output.
fn build_output(root: &Path, source: &str) -> Output {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"bug471\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(root.join("src/main.mfb"), format!("{PRELUDE}{source}")).expect("write source");
    mfb()
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build")
}

/// Build the project and return the built executable, panicking with the build
/// output when it does not compile.
fn build(root: &Path, source: &str) -> PathBuf {
    let out = build_output(root, source);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "expected the inline-TRAP program to build, but it failed:\n{combined}"
    );
    let exe = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build reported no executable path")
        .trim()
        .to_string();
    root.join(exe.strip_prefix("./").unwrap_or(&exe))
}

/// Run the program and return its stdout, requiring a clean exit — the bug's
/// symptom is exit 255 with the *uncaught* domain error on stderr.
fn run(exe: &Path) -> String {
    let out = Command::new(exe).output().expect("run built program");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the inline TRAP did not cover the raising operator (exit {:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code()
    );
    stdout
}

/// The bug doc's minimal repro: `1 / z` raises `7-705-0002` while evaluating an
/// argument of the trapped call, so the handler must run.
#[test]
fn nested_division_by_zero_reaches_the_inline_handler() {
    let root = unique_root("div");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = two(1 / z, 2) TRAP(e)\n",
            "    io::print(\"caught code=\" & toString(e.code))\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught code=77050002\nd=-1\n");
}

/// An integer multiply that overflows raises `7-705-0010` from the same
/// `emit_error_register_return` seam.
#[test]
fn nested_integer_overflow_reaches_the_inline_handler() {
    let root = unique_root("mul");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 4000000000\n",
            "  LET d = two(z * z, 2) TRAP(e)\n",
            "    io::print(\"caught code=\" & toString(e.code))\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught code=77050010\nd=-1\n");
}

/// `MOD` by zero routes through the same divisor check as `/`.
#[test]
fn nested_mod_by_zero_reaches_the_inline_handler() {
    let root = unique_root("mod");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = two(7 MOD z, 2) TRAP(e)\n",
            "    io::print(\"caught code=\" & toString(e.code))\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught code=77050002\nd=-1\n");
}

/// A `Float` multiply that overflows to infinity raises at plan-17's
/// observation boundary, which sits inside the trapped expression.
#[test]
fn nested_float_overflow_reaches_the_inline_handler() {
    let root = unique_root("float");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Float = 1.0e308\n",
            "  LET d = twoF(z * z, 2.0) TRAP(e)\n",
            "    io::print(\"caught code=\" & toString(e.code))\n",
            "    RECOVER -1.0\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught code=77050015\nd=-1.00\n");
}

/// Unary negation of the minimum `Integer` overflows.
#[test]
fn nested_unary_negation_overflow_reaches_the_inline_handler() {
    let root = unique_root("neg");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = -9223372036854775807 - 1\n",
            "  LET d = two(-z, 2) TRAP(e)\n",
            "    io::print(\"caught code=\" & toString(e.code))\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught code=77050010\nd=-1\n");
}

/// The operator sits inside a *fallible* call's argument, so both bug-457's
/// lifting and bug-471's must apply to the same expression.
#[test]
fn raising_operator_inside_a_fallible_call_reaches_the_handler() {
    let root = unique_root("mixed");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = inner(1 / z) TRAP(e)\n",
            "    io::print(\"caught code=\" & toString(e.code))\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught code=77050002\nd=-1\n");
}

/// A `RECOVER` must skip the *remainder* of the trapped expression: once the
/// operator raises, the enclosing call is never made.
#[test]
fn recover_skips_the_rest_of_the_trapped_expression() {
    let root = unique_root("skip");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = shout(\"outer\", 1 / z) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 5\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(
        run(&exe),
        "handled\nd=5\n",
        "the enclosing call must not run after the operator raised"
    );
}

/// Lifting the operator must not pull it ahead of an effectful argument that
/// precedes it in source order.
#[test]
fn lifting_preserves_left_to_right_argument_order() {
    let root = unique_root("order");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = two(note(\"A\", 1), 1 / z) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 5\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "eval A\nhandled\nd=5\n");
}

/// An operator that does *not* raise still delivers its value: the checked
/// bind's `Ok` arm carries the result into the trap's value slot.
#[test]
fn a_raising_operator_that_succeeds_still_delivers_its_value() {
    let root = unique_root("ok");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 5\n",
            "  LET d = two(10 / z, 3) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "d=203\n");
}

/// The first failure in evaluation order wins: the operator raises before the
/// fallible call is ever made.
#[test]
fn the_first_failure_in_evaluation_order_wins() {
    let root = unique_root("first");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = two(1 / z, inner(-1)) TRAP(e)\n",
            "    io::print(\"caught code=\" & toString(e.code))\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(
        run(&exe),
        "caught code=77050002\nd=-1\n",
        "the operator's 7-705-0002 precedes inner's 9-000-0001"
    );
}

/// …and in the other order the fallible call's error is the one reported.
#[test]
fn a_preceding_call_failure_still_wins() {
    let root = unique_root("callfirst");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = two(inner(-1), 1 / z) TRAP(e)\n",
            "    io::print(\"caught code=\" & toString(e.code))\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught code=90000001\nd=-1\n");
}

/// An `Assign` target (`x = … TRAP`) shares the value slot with the `RECOVER`.
#[test]
fn an_assign_target_recovers_from_a_raising_operator() {
    let root = unique_root("assign");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  MUT d AS Integer = 99\n",
            "  d = two(1 / z, 2) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 42\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "handled\nd=42\n");
}

/// A handler that never reads `e` still runs: plan-64-I's error-elision path
/// must survive the new checked bind.
#[test]
fn a_handler_that_ignores_the_error_still_recovers() {
    let root = unique_root("noerr");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = two(1 / z, 2) TRAP(e)\n",
            "    RECOVER 8\n",
            "  END TRAP\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "d=8\n");
}

/// A callee whose body raises **only** through an operator is fallible, and the
/// desugar has to know it (bug-471, second defect).
///
/// `fallible::analyze` decided a function could fail only if its body could
/// `FAIL`, `PROPAGATE`, or call something fallible — it never looked at the
/// operators, so `FUNC fltDiv(a, b) / RETURN a / b` was recorded **infallible**.
/// Once anything in the trapped expression is lifted, `check_root` consults that
/// verdict and leaves the root call unchecked, and the callee's real error
/// auto-propagates past the handler.
///
/// **This predates bug-471.** Measured on a release compiler built from the
/// merge-base (`5815262c4`) with bug-457's lift triggered by a fallible call
/// instead of an operator — `fltDiv(toFloat(inner(1)), 0.0) TRAP(e)` exits 255
/// with an uncaught `7-705-0015` there too. bug-471 only widened its reach, by
/// making a raising operator another thing that fills `hoists`.
#[test]
fn a_callee_that_raises_only_through_an_operator_is_still_checked() {
    let root = unique_root("calleeop");
    let exe = build(
        &root,
        concat!(
            "FUNC fltDiv(a AS Float, b AS Float) AS Float\n",
            "  RETURN a / b\n",
            "END FUNC\n",
            "FUNC main() AS Integer\n",
            // Nothing is lifted here, so the root is checked unconditionally:
            // this arm worked before the fix and is the control.
            "  LET x = fltDiv(1.0, 0.0) TRAP(e)\n",
            "    io::print(\"plain=\" & toString(e.code))\n",
            "    RECOVER 0.0\n",
            "  END TRAP\n",
            // `0.0 - 1.0` is lifted, so `check_root` asks the oracle whether
            // `fltDiv` can fail. It can.
            "  LET y = fltDiv(0.0 - 1.0, 0.0) TRAP(e)\n",
            "    io::print(\"hoisted=\" & toString(e.code))\n",
            "    RECOVER 0.0\n",
            "  END TRAP\n",
            "  io::print(\"x=\" & toString(x) & \" y=\" & toString(y))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(
        run(&exe),
        "plain=77050015\nhoisted=77050015\nx=0.00 y=0.00\n",
        "the root call must stay checked when its callee raises through an operator"
    );
}

/// The same hole reached the way it existed *before* bug-471: bug-457's lift is
/// triggered by a nested fallible **call**, and the root callee raises only
/// through an operator. This is the shape that reproduces on the merge-base
/// compiler, so it dates the defect independently of the operator lift.
#[test]
fn bug457s_lift_also_keeps_an_operator_raising_root_checked() {
    let root = unique_root("calleeop457");
    let exe = build(
        &root,
        concat!(
            "FUNC fltDiv(a AS Float, b AS Float) AS Float\n",
            "  RETURN a / b\n",
            "END FUNC\n",
            "FUNC main() AS Integer\n",
            "  LET y = fltDiv(toFloat(inner(1)), 0.0) TRAP(e)\n",
            "    io::print(\"caught=\" & toString(e.code))\n",
            "    RECOVER 0.0\n",
            "  END TRAP\n",
            "  io::print(\"y=\" & toString(y))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught=77050015\ny=0.00\n");
}

/// Build the project with `--ir` and return the emitted IR JSON.
fn build_ir(root: &Path, source: &str) -> String {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"bug471\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(root.join("src/main.mfb"), format!("{PRELUDE}{source}")).expect("write source");
    let out = mfb()
        .arg("build")
        .arg(root)
        .arg("--ir")
        .output()
        .expect("run mfb build --ir");
    assert!(
        out.status.success(),
        "build --ir failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    fs::read_to_string(root.join("bug471.ir")).expect("read emitted IR")
}

/// The one exemption in `fallible::is_total_literal_negation`, pinned where a
/// reader will look for it: `f(-1)` is a *negative literal*, not a computed
/// negation, so it is left in place, while `f(-b)` over a binding is lifted into
/// a `checked` bind. Removing the exemption would wrap every negative literal in
/// a whole `Result` materialization for a negation that provably succeeds; the
/// wrong version of it would drop a real `ErrOverflow`.
#[test]
fn a_negative_literal_is_not_lifted_but_a_computed_negation_is() {
    let literal = build_ir(
        &unique_root("irlit"),
        concat!(
            "FUNC main() AS Integer\n",
            "  LET d = two(-1, 2) TRAP(e)\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  RETURN d\n",
            "END FUNC\n",
        ),
    );
    assert!(
        !literal.contains("\"kind\": \"checked\""),
        "a negative literal must not be lifted into a checked bind:\n{literal}"
    );

    let computed = build_ir(
        &unique_root("ircomputed"),
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT b AS Integer = 5\n",
            "  LET d = two(-b, 2) TRAP(e)\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  RETURN d\n",
            "END FUNC\n",
        ),
    );
    assert!(
        computed.contains("\"kind\": \"checked\""),
        "a negation over a binding must be lifted into a checked bind:\n{computed}"
    );
}

/// A raising operator in a **short-circuited** operand cannot be lifted —
/// hoisting would evaluate it unconditionally — so it is reported rather than
/// left to escape the handler silently, exactly as bug-457 reports a fallible
/// call there.
#[test]
fn a_short_circuited_raising_operator_is_rejected() {
    let root = unique_root("shortcircuit");
    let out = build_output(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  MUT t AS Boolean = true\n",
            "  LET d = pick(t AND (1 / z) > 0, 2) TRAP(e)\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  RETURN d\n",
            "END FUNC\n",
        ),
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "a short-circuited raising operator must not compile silently:\n{combined}"
    );
    assert!(
        combined.contains("TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL"),
        "expected TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL, got:\n{combined}"
    );
}

/// Control (unchanged by this fix): a scrutinee that is a bare operator with
/// nothing fallible nested in it is still rejected — an inline `TRAP` traps a
/// call, and `1 / z` alone is not one. The fix widens what is *covered* inside a
/// trapped expression, not what may *be* one.
#[test]
fn a_bare_operator_scrutinee_is_still_rejected() {
    let root = unique_root("bare");
    let out = build_output(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = 1 / z TRAP(e)\n",
            "    RECOVER -1\n",
            "  END TRAP\n",
            "  RETURN d\n",
            "END FUNC\n",
        ),
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "expected a diagnostic:\n{combined}");
    assert!(
        combined.contains("TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"),
        "expected TYPE_INLINE_TRAP_REQUIRES_FALLIBLE, got:\n{combined}"
    );
}

/// Control (bug-457's shape): a nested fallible call with no operator in sight
/// still reaches the handler, so the new lifting cannot have displaced it.
#[test]
fn bug457s_nested_call_shape_still_works() {
    let root = unique_root("bug457");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET b = outer(inner(-1)) TRAP(e)\n",
            "    io::print(\"caught nested code=\" & toString(e.code))\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "caught nested code=90000001\nb=0\n");
}

/// Control: an operator that raises *outside* any inline `TRAP` still
/// propagates to the function-level trap. Capturing it inside a trap region
/// must not make it unobservable everywhere else.
#[test]
fn a_raising_operator_outside_a_trap_still_propagates() {
    let root = unique_root("outside");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  LET d = two(1 / z, 2)\n",
            "  io::print(\"d=\" & toString(d))\n",
            "  RETURN 0\n",
            "  TRAP(e)\n",
            "    io::print(\"function trap code=\" & toString(e.code))\n",
            "    RETURN 0\n",
            "  END TRAP\n",
            "END FUNC\n",
        ),
    );
    assert_eq!(run(&exe), "function trap code=77050002\n");
}
