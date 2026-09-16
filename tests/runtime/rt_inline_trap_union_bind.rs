//! Regression tests for the inline-`TRAP` union-wrap gap, found while fixing
//! bug-642 and filed into that bug doc as its widened scope.
//!
//! `ir::lower::lower_inline_trap` stages the trapped expression's value in a
//! temp typed with the **producer's** type and then delivers it to the target
//! with a bare `Bind name : <declared> = local $trap_valN`. Every other binding
//! path runs that delivery through `wrap_union_value`, which inserts the
//! `UnionWrap` that turns a variant value into the union's
//! `{tag@0, payload@8}` value. The inline-`TRAP` path does not, so a
//! union-typed binding initialized from a **variant**-typed fallible producer
//! ends up holding the bare variant with no tag.
//!
//! Two consequences, one loud and one silent:
//!
//! * **Silent (the dangerous one).** `MATCH` on such a binding reads a variant
//!   tag out of a value that has none and falls through EVERY case, with no
//!   diagnostic and a clean exit. The program simply skips the block. This hits
//!   data unions and resource unions alike.
//! * **Loud.** For a resource union the missing `UnionWrap` also makes the bind
//!   look like the aliasing shape (`NirValue::Local`) to
//!   `runtime::usage::push_op_helpers`, which then declares none of the
//!   variants' close helpers while `validate::capabilities::collect_bind_types`
//!   counts them all as used — the build dies with "NIR runtime call requires
//!   undeclared helper" (bug-642). `tests/cli/cli_thread_accept_res_bind.rs`
//!   carries that half.
//!
//! Each case is pinned against the same program written WITHOUT the inline
//! `TRAP`, which has always worked: the contrast is what shows the defect is
//! the desugar's and not the `MATCH` lowering's.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_trapunion_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

/// Scaffold an executable project for `source`, build it, and return the
/// executable; panics with the build output when it does not compile.
fn build(root: &Path, source: &str) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"trapunion\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    let out = Command::new(common::mfb_exe())
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
        "expected the inline-TRAP union program to build, but it failed:\n{combined}"
    );
    let exe = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build reported no executable path")
        .trim()
        .to_string();
    root.join(exe.strip_prefix("./").unwrap_or(&exe))
}

/// Run the program and return its stdout, requiring a clean exit.
fn run(exe: &Path) -> String {
    let out = Command::new(exe).output().expect("run built program");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the inline-TRAP union program failed (exit {:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code()
    );
    stdout
}

/// A data union, its two record variants, and a fallible producer returning the
/// VARIANT type (`Num`), not the union — the shape that needs the wrap.
const DATA_PRELUDE: &str = concat!(
    "IMPORT io\n\n",
    "TYPE Num\n  v AS Integer\nEND TYPE\n\n",
    "TYPE Txt\n  s AS String\nEND TYPE\n\n",
    "UNION Val\n  Num\n  Txt\nEND UNION\n\n",
    "FUNC makeNum(n AS Integer) AS Num\n",
    "  IF n < 0 THEN FAIL error(77050004, \"bad \" & toString(n))\n",
    "  RETURN Num[n]\n",
    "END FUNC\n\n",
);

/// The contrast: the same union binding WITHOUT an inline `TRAP` matches its
/// variant, as it always has. If this ever goes red the defect is in the union
/// or `MATCH` lowering generally, not in the `TRAP` desugar, and the test below
/// is measuring the wrong thing.
#[test]
fn a_plain_union_bind_from_a_variant_producer_matches_its_variant() {
    let root = unique_root("data_plain");
    let source = format!(
        "{DATA_PRELUDE}\
         SUB main()\n\
        \x20 LET v AS Val = makeNum(7)\n\
        \x20 MATCH v\n\
        \x20   CASE Num(m)\n\
        \x20     io::print(\"matched-num \" & toString(m.v))\n\
        \x20   CASE Txt(t)\n\
        \x20     io::print(\"matched-txt\")\n\
        \x20 END MATCH\n\
        \x20 io::print(\"after\")\n\
         END SUB\n"
    );
    let stdout = run(&build(&root, &source));
    assert!(
        stdout.contains("matched-num 7"),
        "a plain union bind must match its variant, got:\n{stdout}"
    );
}

/// The bug: the same binding through an inline `TRAP` whose handler never fires.
/// Before the fix this printed only `after` — `MATCH` fell through every case
/// and the program exited 0, so nothing but the missing line said anything was
/// wrong.
#[test]
fn a_trap_bound_data_union_matches_its_variant() {
    let root = unique_root("data_trap");
    let source = format!(
        "{DATA_PRELUDE}\
         SUB main()\n\
        \x20 LET v AS Val = makeNum(7) TRAP(e)\n\
        \x20   io::print(\"trapped\")\n\
        \x20   EXIT SUB\n\
        \x20 END TRAP\n\
        \x20 MATCH v\n\
        \x20   CASE Num(m)\n\
        \x20     io::print(\"matched-num \" & toString(m.v))\n\
        \x20   CASE Txt(t)\n\
        \x20     io::print(\"matched-txt\")\n\
        \x20 END MATCH\n\
        \x20 io::print(\"after\")\n\
         END SUB\n"
    );
    let stdout = run(&build(&root, &source));
    assert!(
        stdout.contains("matched-num 7"),
        "a `TRAP`-bound union initialized from a variant-typed producer must carry the \
         union tag, so MATCH selects the variant; it fell through every case instead \
         (bug-642 widened scope). Got:\n{stdout}"
    );
    assert!(
        !stdout.contains("trapped"),
        "the handler must not fire for a successful producer, got:\n{stdout}"
    );
}

/// The handler path of the same binding still reaches the handler — the wrap
/// must not disturb the error route.
#[test]
fn a_trap_bound_data_union_still_reaches_its_handler() {
    let root = unique_root("data_trap_fires");
    let source = format!(
        "{DATA_PRELUDE}\
         SUB main()\n\
        \x20 LET v AS Val = makeNum(0 - 1) TRAP(e)\n\
        \x20   io::print(\"trapped\")\n\
        \x20   EXIT SUB\n\
        \x20 END TRAP\n\
        \x20 MATCH v\n\
        \x20   CASE Num(m)\n\
        \x20     io::print(\"matched-num \" & toString(m.v))\n\
        \x20   CASE Txt(t)\n\
        \x20     io::print(\"matched-txt\")\n\
        \x20 END MATCH\n\
         END SUB\n"
    );
    let stdout = run(&build(&root, &source));
    assert!(
        stdout.contains("trapped") && !stdout.contains("matched-"),
        "a failing producer must reach the inline handler and skip the delivery, got:\n{stdout}"
    );
}

/// The resource-union half, and bug-642's own reproduction shape. A real `fs::`
/// call is present so the program gets past the helper-declaration failure and
/// the RUNTIME behaviour is what is under test here; the build-only half lives
/// in `tests/cli/cli_thread_accept_res_bind.rs`.
#[test]
fn a_trap_bound_resource_union_matches_its_variant() {
    let root = unique_root("res_trap");
    let source = "IMPORT io\n\
                  IMPORT udp\n\
                  IMPORT fs\n\n\
                  UNION Chan\n\
                 \x20 udp::Socket\n\
                 \x20 fs::File\n\
                  END UNION\n\n\
                  SUB probe(port AS Integer)\n\
                 \x20 RES c AS Chan = udp::bind(\"127.0.0.1\", port) TRAP(e)\n\
                 \x20   io::print(\"trapped\")\n\
                 \x20   EXIT SUB\n\
                 \x20 END TRAP\n\
                 \x20 MATCH c\n\
                 \x20   CASE udp::Socket(s)\n\
                 \x20     io::print(\"matched-udp\")\n\
                 \x20   CASE fs::File(f)\n\
                 \x20     io::print(\"matched-file\")\n\
                 \x20 END MATCH\n\
                  END SUB\n\n\
                  SUB main()\n\
                 \x20 IF fs::fileExists(\"/definitely/not/here\") THEN\n\
                 \x20   io::print(\"huh\")\n\
                 \x20 END IF\n\
                 \x20 probe(0)\n\
                  END SUB\n";
    let stdout = run(&build(&root, source));
    assert!(
        stdout.contains("matched-udp"),
        "a `TRAP`-bound resource union must carry the union tag so MATCH selects the live \
         variant; it fell through every case instead (bug-642 widened scope). Got:\n{stdout}"
    );
}
