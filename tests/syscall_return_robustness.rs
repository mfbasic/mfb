//! Regression test for bug-62: the fs/io runtime syscall loops must handle the
//! three degenerate returns that bug-51's short-transfer loops left unaddressed,
//! on every backend.
//!
//! 1. `EINTR` — a signal that interrupts a blocking `read`/`write` before any
//!    byte moves returns `-EINTR`; the loop must re-issue the identical syscall
//!    rather than report `ErrInput`/`ErrOutput`. Two errno conventions: every
//!    libc call (`read`/`lseek` on all backends, `write` on macOS/aarch64/riscv)
//!    exposes the code through the platform accessor (`___error` /
//!    `__errno_location`); the `linux-x86_64` `write` is a raw `svc` whose return
//!    value is `-errno`, so its `EINTR` check is `ret + EINTR == 0` (a `mov_imm 4`
//!    then add/compare) with no accessor call. The console-mode `io::` stdin
//!    readers are an exception: since plan-15 they consume the stdin broadcast log
//!    through `_mfb_rt_stdin_next_byte` rather than reading fd 0 inline, so the
//!    guard lives once in that shared helper and the readers just call it.
//! 2. `write() == 0` — a 0-byte return for a nonzero request must error, never
//!    spin. The stdout/File drains previously tested the result with `branch_lt`,
//!    so a 0 return advanced by zero and looped forever; the fix routes 0 to the
//!    drain's `_err` label with an equality branch.
//! 3. A failed reconcile rewind `lseek` (e.g. `ESPIPE` on a FIFO) must surface as
//!    a read/write error instead of silently dropping the unconsumed read-ahead;
//!    `fs::writeAll`/`writeAllBytes` now branch on the reconcile seek result.
//!
//! The runtime difference needs fault injection (a signal / a 0-return `write` /
//! a non-seekable handle), which the acceptance harness cannot stage portably, so
//! this test locks the codegen structure across all four backends instead. It
//! fails the moment any of the three guards is dropped from any backend.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// A program that instantiates every fs/io helper the fix touches: the buffered
/// `writeAll`/`writeAllBytes` (drain + reconcile + write loop), whole-file
/// `readAll`/`readAllBytes`, `fs::readLine`, and the io print/readLine/readChar/
/// readByte helpers. It imports both `fs` and `io`, so the errno accessor is
/// linked and the libc EINTR guards read it (a pure-`io::` program has no reason
/// to import it — a separate boundary that this test does not exercise).
const SOURCE: &str = "\
IMPORT io
IMPORT fs
IMPORT strings

SUB main()
    RES f AS fs::File = fs::openFile(\"/tmp/mfb_bug62/a.txt\", \"w\")
    fs::setBuffered(f, TRUE)
    fs::writeAll(f, \"x\")
    fs::writeAllBytes(f, strings::toBytes(\"y\"))
    fs::flush(f)
    fs::close(f)
    RES g AS fs::File = fs::openFile(\"/tmp/mfb_bug62/a.txt\", \"r\")
    LET a AS String = fs::readAll(g)
    fs::close(g)
    RES h AS fs::File = fs::openFile(\"/tmp/mfb_bug62/a.txt\", \"r\")
    LET b AS List OF Byte = fs::readAllBytes(h)
    fs::close(h)
    RES k AS fs::File = fs::openFile(\"/tmp/mfb_bug62/a.txt\", \"r\")
    LET l AS String = fs::readLine(k)
    fs::close(k)
    LET z AS String = io::readLine()
    LET c AS String = io::readChar()
    LET n AS Byte = io::readByte()
    io::print(a & l & z & c & toString(len(b)) & toString(n))
END SUB
";

/// Helpers whose underlying transfer is a `write` (raw `svc` on `linux-x86_64`,
/// libc elsewhere).
const WRITE_HELPERS: &[&str] = &[
    "_mfb_rt_fs_fs_writeAll",
    "_mfb_rt_fs_fs_writeAllBytes",
    "_mfb_rt_fs_file_drain",
    "_mfb_rt_io_stdout_drain",
    "_mfb_rt_io_io_print",
];

/// Helpers whose transfer is a direct libc `read` on every backend — the EINTR
/// guard lives inline in each. The `io::` stdin readers are NOT here: in console
/// mode (plan-15) they consume the broadcast log through `_mfb_rt_stdin_next_byte`
/// instead of issuing `read(0,…,1)` themselves, so the guard moved into that
/// helper (see `STDIN_READ_HELPERS` / `STDIN_NEXT_BYTE`).
const READ_HELPERS: &[&str] = &[
    "_mfb_rt_fs_fs_readAll",
    "_mfb_rt_fs_fs_readAllBytes",
    "_mfb_rt_fs_fs_readLine",
];

/// The console-mode `io::` stdin readers. Each pulls its bytes from the stdin
/// broadcast log via `_mfb_rt_stdin_next_byte` (plan-15 §4.3) rather than reading
/// fd 0 directly, so instead of an inline EINTR guard they must call that helper —
/// which owns the guard for all of them.
const STDIN_READ_HELPERS: &[&str] = &[
    "_mfb_rt_io_io_readLine",
    "_mfb_rt_io_io_readChar",
    "_mfb_rt_io_io_readByte",
];

/// The cooperative per-thread stdin reader that the console-mode `io::` readers
/// route through. Its blocking refill is the one `read(0,…)` for stdin, so it
/// carries the single EINTR guard (errno accessor + `EINTR`==4 compare) that used
/// to be duplicated across every stdin read site.
const STDIN_NEXT_BYTE: &str = "_mfb_rt_stdin_next_byte";

const DRAINS: &[&str] = &["_mfb_rt_fs_file_drain", "_mfb_rt_io_stdout_drain"];

/// `writeAll`/`writeAllBytes` reconcile the read buffer before the write and must
/// check that rewind `lseek`.
const RECONCILE_HELPERS: &[&str] = &["_mfb_rt_fs_fs_writeAll", "_mfb_rt_fs_fs_writeAllBytes"];

fn temp_project(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"bug62\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), SOURCE).expect("write source");
    root
}

fn build_ncode(project: &Path, target: &str, name: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg("-ncode")
        .arg("-target")
        .arg(target)
        .arg(project)
        .output()
        .expect("run mfb build -ncode");
    assert!(
        output.status.success(),
        "mfb build -ncode -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let path = project.join(format!("{name}.ncode"));
    let text = fs::read_to_string(&path).expect("read ncode dump");
    serde_json::from_str(&text).expect("parse ncode json")
}

fn op_name(op: &Value) -> &str {
    op.get("op").and_then(Value::as_str).unwrap_or("")
}

fn target_field(op: &Value) -> &str {
    op.get("target").and_then(Value::as_str).unwrap_or("")
}

/// A call (`bl`/`call`/`jal`/`jalr`) to the platform errno accessor.
fn is_errno_accessor_call(op: &Value) -> bool {
    matches!(op_name(op), "bl" | "call" | "jal" | "jalr")
        && matches!(target_field(op), "___error" | "__errno_location")
}

/// A literal `4` (`EINTR`) materialized as a compare RHS (aarch64/x86 libc), a
/// loaded immediate (riscv fusion), or a folded `add_imm` immediate (the x86 raw
/// `-errno` add-check — `add rax, 4; cmp rax, 0; b.eq retry`; whether the `4` is a
/// separate `mov_imm value=4` or folded into `add_imm imm=4` depends on register
/// pressure, so accept either).
fn is_eintr_literal(op: &Value) -> bool {
    op.get("rhs").and_then(Value::as_str) == Some("4")
        || op.get("value").and_then(Value::as_str) == Some("4")
        || op.get("imm").and_then(Value::as_str) == Some("4")
}

/// A raw kernel syscall (`linux-x86_64` `write` is a bare `svc`).
fn is_raw_syscall(op: &Value) -> bool {
    matches!(op_name(op), "svc" | "syscall")
}

/// A call (`bl`/`call`/`jal`/`jalr`) to the shared stdin-broadcast reader.
fn is_stdin_next_byte_call(op: &Value) -> bool {
    matches!(op_name(op), "bl" | "call" | "jal" | "jalr")
        && target_field(op) == STDIN_NEXT_BYTE
}

/// An equality conditional branch to `target` (aarch64/x86 `b.eq`, riscv fused
/// `rv.br cond=eq`).
fn is_eq_branch_to(op: &Value, suffix: &str) -> bool {
    let eq = op_name(op) == "b.eq"
        || (op_name(op) == "rv.br" && op.get("cond").and_then(Value::as_str) == Some("eq"));
    eq && target_field(op).ends_with(suffix)
}

/// Any integer conditional branch (used to prove the reconcile seek is checked).
fn is_conditional_branch(op: &Value) -> bool {
    matches!(
        op_name(op),
        "b.eq" | "b.ne" | "b.lt" | "b.le" | "b.gt" | "b.ge" | "b.hi" | "b.lo" | "b.ls" | "rv.br"
    )
}

fn find_helper<'a>(functions: &'a [Value], target: &str, symbol: &str) -> &'a Value {
    functions
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("{target}: helper {symbol} not emitted"))
}

fn instructions<'a>(func: &'a Value) -> &'a [Value] {
    func["instructions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn assert_target(target: &str) {
    let name = format!("bug62_{}", target.replace('-', "_"));
    let project = temp_project(&name);
    let ncode = build_ncode(&project, target, "bug62");
    assert_eq!(
        ncode["target"].as_str(),
        Some(target),
        "ncode target field mismatch",
    );
    let functions = ncode["functions"].as_array().expect("functions array");

    // Item 1 — EINTR retry on every direct-read helper (all libc: the accessor
    // must be consulted and the code compared against EINTR).
    for helper in READ_HELPERS {
        let func = find_helper(functions, target, helper);
        let ins = instructions(func);
        assert!(
            ins.iter().any(is_errno_accessor_call),
            "{target}/{helper}: no errno-accessor call — EINTR retry dropped from a read loop (bug-62)",
        );
        assert!(
            ins.iter().any(is_eintr_literal),
            "{target}/{helper}: no EINTR (4) comparison — EINTR retry dropped from a read loop (bug-62)",
        );
    }

    // Item 1 (stdin) — the console-mode `io::` readers no longer read fd 0 inline;
    // each must route through `_mfb_rt_stdin_next_byte` (plan-15 §4.3), and that
    // helper is where the single stdin EINTR guard now lives. Proving both keeps
    // the guard from silently vanishing: the readers must call it, and it must
    // consult the errno accessor and compare against EINTR.
    for helper in STDIN_READ_HELPERS {
        let func = find_helper(functions, target, helper);
        assert!(
            instructions(func).iter().any(is_stdin_next_byte_call),
            "{target}/{helper}: no call to {STDIN_NEXT_BYTE} — stdin read is not routed through the broadcast reader (plan-15)",
        );
    }
    {
        let func = find_helper(functions, target, STDIN_NEXT_BYTE);
        let ins = instructions(func);
        assert!(
            ins.iter().any(is_errno_accessor_call),
            "{target}/{STDIN_NEXT_BYTE}: no errno-accessor call — EINTR retry dropped from the stdin refill loop (bug-62)",
        );
        assert!(
            ins.iter().any(is_eintr_literal),
            "{target}/{STDIN_NEXT_BYTE}: no EINTR (4) comparison — EINTR retry dropped from the stdin refill loop (bug-62)",
        );
    }

    // Item 1 — EINTR retry on every write helper. On linux-x86_64 the write is a
    // raw `svc` whose `-errno` return is checked with a `mov_imm 4` add-compare
    // (no accessor); every other backend routes the write through libc.
    let raw_write = target == "linux-x86_64";
    for helper in WRITE_HELPERS {
        let func = find_helper(functions, target, helper);
        let ins = instructions(func);
        if raw_write {
            assert!(
                ins.iter().any(is_raw_syscall) && ins.iter().any(is_eintr_literal),
                "{target}/{helper}: raw-svc write is missing the `-EINTR` add-check (bug-62)",
            );
        } else {
            assert!(
                ins.iter().any(is_errno_accessor_call) && ins.iter().any(is_eintr_literal),
                "{target}/{helper}: no errno/EINTR check — EINTR retry dropped from a write loop (bug-62)",
            );
        }
    }

    // Item 2 — a 0-byte drain write is routed to `_err` by an equality branch
    // (the old `branch_lt` let 0 fall through and spin).
    for helper in DRAINS {
        let func = find_helper(functions, target, helper);
        let err = format!("{helper}_err");
        assert!(
            instructions(func).iter().any(|op| is_eq_branch_to(op, &err)),
            "{target}/{helper}: a 0-byte write is not routed to `{err}` — the drain can spin on write()==0 (bug-62)",
        );
    }

    // Item 3 — the single reconcile `lseek` in each whole-file writer is now
    // checked (a conditional branch follows it); pre-fix it fell straight into the
    // buffer-invalidation stores.
    for helper in RECONCILE_HELPERS {
        let func = find_helper(functions, target, helper);
        let ins = instructions(func);
        let seek_idx = ins
            .iter()
            .position(|op| {
                matches!(op_name(op), "bl" | "call" | "jal" | "jalr")
                    && target_field(op).ends_with("lseek")
            })
            .unwrap_or_else(|| panic!("{target}/{helper}: no reconcile lseek emitted"));
        let checked = ins[seek_idx + 1..]
            .iter()
            .take(5)
            .any(is_conditional_branch);
        assert!(
            checked,
            "{target}/{helper}: the reconcile lseek result is not checked — a failed rewind is silently dropped (bug-62)",
        );
    }

    let _ = fs::remove_dir_all(&project);
}

#[test]
fn syscall_return_robustness_macos_aarch64() {
    assert_target("macos-aarch64");
}

#[test]
fn syscall_return_robustness_linux_aarch64() {
    assert_target("linux-aarch64");
}

#[test]
fn syscall_return_robustness_linux_x86_64() {
    assert_target("linux-x86_64");
}

#[test]
fn syscall_return_robustness_linux_riscv64() {
    assert_target("linux-riscv64");
}
