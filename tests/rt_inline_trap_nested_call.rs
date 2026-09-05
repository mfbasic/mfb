//! Regression tests for bug-457: an inline `TRAP` silently failed to cover a
//! fallible call **nested** inside the trapped expression.
//!
//! `ir::lower::lower_inline_trap` converted only the *outermost* node of the
//! trapped expression into a `CallResult`, so exactly one `Result` was produced
//! and checked. A fallible call nested one level in (`outer(inner())`) stayed a
//! plain `Call`, auto-propagated past the handler to the function-level trap,
//! and the handler never ran — with no diagnostic.
//!
//! `mfb spec language error-model` §8.8 desugars a call as `MATCH g(x) … CASE
//! Error(e): PROPAGATE to enclosing TRAP region`, and an inline `TRAP` **is** a
//! TRAP region, so a nested call's error must reach it.
//!
//! The fix hoists every fallible call in the trapped expression that is
//! evaluated unconditionally into its own `CallResult` + `If ResultIsOk` check
//! ahead of the residual expression, nesting the checks so a `RECOVER` skips the
//! rest of the expression. The one shape it cannot desugar — a fallible call in
//! a **short-circuited** operand (the right side of `AND`/`OR`), where hoisting
//! would call it unconditionally — is rejected with
//! `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL` instead of being silently miscompiled.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug457_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    // Hermetic key store so the result is machine-independent (see test-accept.sh).
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug457_home"));
    command
}

/// The helper prelude every case shares: a fallible `inner`, an infallible
/// `outer`/`two`, and a `note` that prints so evaluation order is observable.
const PRELUDE: &str = concat!(
    "IMPORT io\n",
    "IMPORT strings\n",
    "IMPORT encoding\n",
    "IMPORT fs\n\n",
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
        "{\"name\":\"bug457\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
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
/// symptom is a non-zero exit with the *uncaught* `9-000-0001` on stderr.
fn run(exe: &Path) -> String {
    let out = Command::new(exe).output().expect("run built program");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the inline TRAP did not cover the nested call (exit {:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code()
    );
    stdout
}

/// The bug doc's minimal repro: the fallible call is one level in, so its error
/// must still reach the inline handler.
#[test]
fn nested_fallible_call_reaches_the_inline_handler() {
    let root = unique_root("nested");
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
    let stdout = run(&exe);
    assert_eq!(
        stdout, "caught nested code=90000001\nb=0\n",
        "the nested call's error must run the handler and RECOVER 0"
    );
}

/// The outermost-call form, which always worked: kept here so a fix that moves
/// the check cannot silently break the shape the bug did *not* affect.
#[test]
fn outermost_fallible_call_still_reaches_the_handler() {
    let root = unique_root("outermost");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET a = inner(-1) TRAP(e)\n",
            "    io::print(\"caught outermost code=\" & toString(e.code))\n",
            "    RECOVER 7\n",
            "  END TRAP\n",
            "  io::print(\"a=\" & toString(a))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "caught outermost code=90000001\na=7\n");
}

/// Two levels of nesting: the error is raised three calls deep and must still
/// reach the handler.
#[test]
fn deeply_nested_fallible_call_reaches_the_handler() {
    let root = unique_root("deep");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET b = outer(outer(inner(-1))) TRAP(e)\n",
            "    io::print(\"caught deep code=\" & toString(e.code))\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "caught deep code=90000001\nb=0\n");
}

/// A `RECOVER` must skip the *remainder* of the trapped expression: once the
/// nested call fails, the enclosing call is never made.
#[test]
fn recover_skips_the_rest_of_the_trapped_expression() {
    let root = unique_root("skip");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET b = shout(\"outer\", inner(-1)) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 5\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(
        stdout, "handled\nb=5\n",
        "the enclosing call must not run after the nested call failed"
    );
}

/// Argument evaluation stays left to right: hoisting the fallible second
/// argument must not pull it ahead of the effectful first one.
#[test]
fn hoisting_preserves_left_to_right_argument_order() {
    let root = unique_root("order");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET b = two(note(\"A\", 1), inner(-1)) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 5\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(
        stdout, "eval A\nhandled\nb=5\n",
        "`note(\"A\", 1)` is evaluated before the fallible second argument"
    );
}

/// The first failing argument wins and the second is never evaluated.
#[test]
fn first_failing_argument_short_circuits_the_second() {
    let root = unique_root("first");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET b = two(inner(-1), shout(\"second\", 3)) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 5\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "handled\nb=5\n");
}

/// The success path still delivers the whole expression's value.
#[test]
fn nested_success_path_delivers_the_full_expression_value() {
    let root = unique_root("success");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET b = two(note(\"A\", 1), outer(inner(3))) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 5\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "eval A\nb=107\n");
}

/// A nested fallible call under an *infallible* outermost built-in: the handler
/// is no longer dead, so it must run rather than the error escaping.
#[test]
fn nested_fallible_call_under_an_infallible_builtin_root() {
    let root = unique_root("infallible_root");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET s = toString(inner(-1)) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER \"fallback\"\n",
            "  END TRAP\n",
            "  io::print(\"s=\" & s)\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "handled\ns=fallback\n");
}

/// A trapped expression whose *root* is not a call but which does contain a
/// fallible call: §8.4 scopes an inline `TRAP` to the whole expression, so the
/// nested call's error is trapped rather than rejected as "not a call".
#[test]
fn non_call_root_with_a_nested_fallible_call_is_trapped() {
    let root = unique_root("binary_root");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET c = inner(-1) + 1 TRAP(e)\n",
            "    io::print(\"caught binary code=\" & toString(e.code))\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"c=\" & toString(c))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "caught binary code=90000001\nc=0\n");
}

/// A trapped expression with no call at all is still rejected: there is nothing
/// to trap, so `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` must survive the fix.
#[test]
fn a_trapped_expression_with_no_call_is_still_rejected() {
    let root = unique_root("no_call");
    let out = build_output(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET c = 1 + 1 TRAP(e)\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"c=\" & toString(c))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success() && combined.contains("TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"),
        "a call-free trapped expression must still be rejected:\n{combined}"
    );
}

/// The one shape the desugar cannot cover: hoisting a fallible call out of a
/// short-circuited operand would call it unconditionally, so it is rejected
/// instead of silently escaping the handler.
#[test]
fn a_fallible_call_in_a_short_circuited_operand_is_rejected() {
    let root = unique_root("short_circuit");
    let out = build_output(
        &root,
        concat!(
            "FUNC positive(n AS Integer) AS Boolean\n",
            "  RETURN inner(n) > 0\n",
            "END FUNC\n\n",
            "FUNC main() AS Integer\n",
            "  LET ok = TRUE AND positive(-1) TRAP(e)\n",
            "    RECOVER FALSE\n",
            "  END TRAP\n",
            "  io::print(\"ok=\" & toString(ok))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success() && combined.contains("TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL"),
        "a fallible call in a short-circuited operand must be diagnosed:\n{combined}"
    );
}

/// The handler still runs exactly once when the nested call fails: a desugar
/// that duplicated the handler into every check would print twice.
#[test]
fn the_handler_runs_once_per_failure() {
    let root = unique_root("once");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET b = two(inner(2), inner(-1)) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 5\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "handled\nb=5\n");
}

/// A diverging handler (no `RECOVER`) still diverges from a nested failure.
#[test]
fn a_diverging_handler_covers_a_nested_failure() {
    let root = unique_root("diverge");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET b = outer(inner(-1)) TRAP(e)\n",
            "    io::print(\"diverging\")\n",
            "    RETURN 0\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 1\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "diverging\n");
}

/// The bare-statement (`Discard`) form of the inline `TRAP` covers a nested
/// failure too — it shares the desugar but has no value slot.
#[test]
fn the_discard_form_covers_a_nested_failure() {
    let root = unique_root("discard");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  outer(inner(-1)) TRAP(e)\n",
            "    io::print(\"handled discard\")\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"after\")\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "handled discard\nafter\n");
}

/// The assignment form of the inline `TRAP` covers a nested failure too.
#[test]
fn the_assign_form_covers_a_nested_failure() {
    let root = unique_root("assign");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  MUT b AS Integer = 99\n",
            "  b = outer(inner(-1)) TRAP(e)\n",
            "    io::print(\"handled assign\")\n",
            "    RECOVER 4\n",
            "  END TRAP\n",
            "  io::print(\"b=\" & toString(b))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "handled assign\nb=4\n");
}

/// A nested fallible *built-in* (`strings::mid` raises when the span runs past
/// the end) reaches the handler as well — the fix is not user-`FUNC` specific.
#[test]
fn a_nested_fallible_builtin_reaches_the_handler() {
    let root = unique_root("builtin");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET n = len(strings::mid(\"ab\", 0, 9)) TRAP(e)\n",
            "    io::print(\"handled builtin\")\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"n=\" & toString(n))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "handled builtin\nn=0\n");
}

/// A user `FUNC len(r AS Ring)` overloading the general built-in
/// (`tests/rt-behavior/functions/func_override_len_user`) reaches lowering under
/// the **overload-mangled** target `len$Ring`, not bare `len` — so the failing
/// override is judged on its own body and lifted, while the infallible built-in
/// `len` keeps its census verdict. Pins that the mangled spelling is the one the
/// fallibility oracle sees: were the two to share the target `len`, the census
/// would call this failing call infallible and its error would escape the
/// handler again.
#[test]
fn a_fallible_user_override_of_an_infallible_builtin_is_still_covered() {
    let root = unique_root("override");
    let exe = build(
        &root,
        concat!(
            "TYPE Ring\n",
            "  items AS List OF Integer\n",
            "END TYPE\n\n",
            "FUNC len(r AS Ring) AS Integer\n",
            "  FAIL error(90000003, \"ring len failed\")\n",
            "END FUNC\n\n",
            "FUNC main() AS Integer\n",
            "  LET r AS Ring = Ring[[1, 2, 3]]\n",
            "  LET n = outer(len(r)) TRAP(e)\n",
            "    io::print(\"caught override code=\" & toString(e.code))\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"n=\" & toString(n))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "caught override code=90000003\nn=0\n");
}

/// A nested **conversion** whose handler reads the error, which is the shape
/// that first broke: `toInt`/`toFloat`/`toFixed`/`toByte`/`toMoney`/`toScalar`
/// are the plan-64-I "error provably unused" candidates
/// (`function_lowering.rs:is_trap_discard_conversion`). That analysis found the
/// paired error local by matching `Bind err = ResultError(result)` only, so the
/// check chain's `Assign $trap_err = ResultError(result)` was invisible to it:
/// every chained conversion was mis-classified as error-discardable, codegen
/// emitted the error tag with NO `Error` block, and the `Assign` then read one
/// that did not exist -- killing the process. Caught by `tests/acceptance`
/// (`expectTrap(toInt(toFloat("1e20")), …)`), which `cargo test` does not run
/// and the execution-free artifact-gate cannot see.
#[test]
fn a_nested_conversion_whose_handler_reads_the_error() {
    let root = unique_root("conversion");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET n = toInt(toFloat(\"1e20\")) TRAP(e)\n",
            "    io::print(\"caught conversion code=\" & toString(e.code))\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"n=\" & toString(n))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(
        stdout, "caught conversion code=77050010\nn=0\n",
        "the overflow must reach the handler with a real Error attached"
    );
}

/// The same chain with a handler that does NOT read the error: the plan-64-I
/// elision is still correct here (nothing observes the `Error`), so this pins
/// that the fix above did not simply disable the optimisation.
#[test]
fn a_nested_conversion_whose_handler_ignores_the_error() {
    let root = unique_root("conversion_noerr");
    let exe = build(
        &root,
        concat!(
            "FUNC main() AS Integer\n",
            "  LET n = toInt(toFloat(\"1e20\")) TRAP(e)\n",
            "    io::print(\"handled\")\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(\"n=\" & toString(n))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
    let stdout = run(&exe);
    assert_eq!(stdout, "handled\nn=0\n");
}

/// The shape that FOUND this bug, pinned. plan-110-D's `26e5d057c` rewrote
/// `tests/rt-behavior/tls/tls-poll-rt` from a bound read into
///
///     LET chunk = encoding::utf8Decode(tls::read(conn, 4096)) TRAP(e) ...
///
/// -- a decode wrapping a fallible read -- and the read's `ErrConnectionClosed`
/// then escaped the handler, so the drain loop never terminated. That fixture
/// was worked around by binding the read again, which left the composition
/// itself unpinned; this pins it.
///
/// Deliberately offline. The failure was in the desugar, not in TLS, so a live
/// peer would buy a capability gate and a server dependency for no extra
/// coverage of THIS bug: `encoding::utf8Decode` over any fallible
/// byte-producing read lowers to the identical IR (infallible-decode root,
/// fallible nested call, handler reading `e`). `fs::readBytes` on a missing path
/// is that shape, deterministically, everywhere.
#[test]
fn a_decode_wrapping_a_fallible_read_reaches_the_handler() {
    let root = unique_root("decode_read");
    let missing = root.join("no-such-file.bin");
    let exe = build(
        &root,
        &format!(
            concat!(
                "FUNC main() AS Integer\n",
                "  LET text = encoding::utf8Decode(fs::readBytes(\"{}\")) TRAP(e)\n",
                "    io::print(\"caught read code=\" & toString(e.code))\n",
                "    RECOVER \"fallback\"\n",
                "  END TRAP\n",
                "  io::print(\"text=\" & text)\n",
                "  RETURN 0\n",
                "END FUNC\n",
            ),
            missing.display()
        ),
    );
    let stdout = run(&exe);
    assert!(
        stdout.starts_with("caught read code=") && stdout.ends_with("text=fallback\n"),
        "the read's error must reach the handler, not escape it:\n{stdout}"
    );
}
