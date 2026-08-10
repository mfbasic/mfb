//! Regression test for bug-44 (and its parent bug-04): the C `int` return of
//! `fsync` and `close` must be narrowed to a signed 64-bit value (`sxtw` on
//! aarch64, `sext.w` on riscv64, `movsxd` on x86-64) *before* the 64-bit signed
//! relational compare inside the atomic-write helpers, on every backend.
//!
//! None of the ABIs we target guarantee the upper 32 bits of an `int` return
//! (AAPCS64 / Darwin arm64 leave `x0[63:32]` unspecified; x86-64 SysV leaves
//! `rax[63:32]` undefined). When a libc leaves those bits clear, a `-1`
//! (EIO/ENOSPC/EBADF) reads as `+4294967295`, the `b.lt` error branch is not
//! taken, and `fs::writeTextAtomic` / `fs::writeBytesAtomic` report a durability
//! failure as success.
//!
//! The defect is that the generated code compiles and looks right; only the
//! narrowing op at the comparison seam distinguishes fixed from broken. A
//! genuinely failing filesystem is required to observe the runtime difference
//! (the tmpfs/NFS harness in `bugs/completed-bugs/bug-44-c-int-return-width-fsync-close.md`),
//! which the macOS dev host cannot stage. This test therefore locks the codegen
//! structure across all four backends instead: every checked `fsync`/`close`
//! call reaches a `sxtw` seam — the very next op on aarch64/riscv64, or just
//! past a `mov rdi, rax` register move on x86-64 (whose C return lands in `rax`,
//! not the `rdi` working register) — and never a bare compare/branch. It fails
//! the moment the seam normalization is dropped from any backend.

mod common;
use common::build_ncode;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// The atomic-write helpers whose `fsync`/`close` results are relationally
/// compared. `fs.createTempFile` only closes on an already-failing cleanup path
/// (result unchecked), so it is deliberately excluded.
const HELPERS: &[&str] = &[
    "_mfb_rt_fs_fs_writeTextAtomic",
    "_mfb_rt_fs_fs_writeBytesAtomic",
];

const SOURCE: &str = "IMPORT fs\nIMPORT strings\n\nFUNC main AS Integer\n  fs::writeTextAtomic(\"/tmp/mfb_bug44/a.txt\", \"x\")\n  fs::writeBytesAtomic(\"/tmp/mfb_bug44/b.bin\", strings::toBytes(\"y\"))\n  RETURN 0\nEND FUNC\n";

fn temp_project(name: &str) -> PathBuf {
    common::temp_project(name, SOURCE)
}

/// A `bl`/`call` to the libc `fsync`/`close` wrapper (macOS prefixes `_`).
fn is_sync_or_close_call(op: &Value) -> Option<&'static str> {
    let target = op.get("target").and_then(Value::as_str)?;
    if target.ends_with("fsync") {
        Some("fsync")
    } else if target.ends_with("close") {
        Some("close")
    } else {
        None
    }
}

fn op_name(op: &Value) -> &str {
    op.get("op").and_then(Value::as_str).unwrap_or("")
}

/// The compare / conditional-branch mnemonics that consume the result. On a
/// broken build one of these sits directly after the call; the fix inserts a
/// `sxtw` in between. `rv.br` is riscv64's fused compare-and-branch.
fn is_compare_or_branch(op: &str) -> bool {
    matches!(
        op,
        "cmp" | "cmp_imm" | "b.lt" | "b.le" | "b.ge" | "b.gt" | "br_cc" | "fbr_cc" | "rv.br"
    )
}

fn assert_helper_normalized(target: &str, func: &Value) {
    let symbol = func["symbol"].as_str().unwrap_or("<none>");
    let ins = func["instructions"]
        .as_array()
        .unwrap_or_else(|| panic!("{target}/{symbol}: no instructions array"));

    let mut fsync_calls = 0usize;
    let mut narrowed_fsync = 0usize;
    let mut narrowed_close = 0usize;

    for (i, op) in ins.iter().enumerate() {
        let Some(kind) = is_sync_or_close_call(op) else {
            continue;
        };
        // The narrowing `sxtw` immediately follows the call on aarch64/riscv64,
        // where the C `int` return already lands in the working register. On
        // x86-64 the return is in `rax` while the helper works in `rdi`, so a
        // `mov rdi, rax` register-move seam sits between the call and the `sxtw`
        // (plan-85 reads fs C-returns from `%retC`/rax). Skip those pure register
        // moves before inspecting the seam so the check is backend-agnostic.
        let mut j = i + 1;
        while ins.get(j).map(op_name) == Some("mov") {
            j += 1;
        }
        let seam = ins.get(j).map(op_name).unwrap_or("");

        // Regression guard: past any register-move shuffle, a checked site must
        // reach the `sxtw` seam, never a bare compare/branch fed by the
        // un-narrowed C `int` — bug-44 (the C int return is compared at 64 bits
        // with the upper word unnormalized).
        assert!(
            !is_compare_or_branch(seam),
            "{target}/{symbol}: `{kind}` result flows into `{seam}` without a \
             sign-extend seam — bug-44 regression (the C int return is compared \
             at 64 bits with the upper word unnormalized)",
        );

        if seam == "sxtw" {
            match kind {
                "fsync" => narrowed_fsync += 1,
                "close" => narrowed_close += 1,
                _ => unreachable!(),
            }
        }
        if kind == "fsync" {
            fsync_calls += 1;
        }
    }

    // The durable data `fsync` is a checked, relationally-compared site and must
    // carry the seam. The parent-directory `fsync` added for crash durability
    // (bug-166) is intentionally best-effort — the atomic rename already
    // succeeded, so a directory that cannot be fsynced must not fail the write —
    // and therefore carries no seam, exactly like the cleanup closes. The
    // regression guard above (no sync/close result flows straight into a
    // compare/branch) is what actually protects bug-44 on every site.
    assert!(
        narrowed_fsync > 0,
        "{target}/{symbol}: the checked data fsync is not sign-extended \
         ({narrowed_fsync} of {fsync_calls} fsync site(s) narrowed)",
    );
    // At least the durable close is checked and narrowed (cleanup closes on the
    // error path are intentionally unchecked and carry no seam).
    assert!(
        narrowed_close > 0,
        "{target}/{symbol}: the checked `close` is not sign-extended",
    );
}

fn assert_target_normalized(target: &str) {
    // Per-target name so the temp dir and `<name>.ncode` never collide when the
    // four backend tests run in parallel.
    let name = format!("fs_atomic_int_{}", target.replace('-', "_"));
    let project = temp_project(&name);
    let ncode = build_ncode(&project, target, &name);
    assert_eq!(
        ncode["target"].as_str(),
        Some(target),
        "ncode target field mismatch",
    );
    let functions = ncode["functions"].as_array().expect("functions array");
    for helper in HELPERS {
        let func = functions
            .iter()
            .find(|f| f["symbol"].as_str() == Some(helper))
            .unwrap_or_else(|| panic!("{target}: helper {helper} not emitted"));
        assert_helper_normalized(target, func);
    }
    let _ = fs::remove_dir_all(&project);
}

#[test]
fn fsync_and_close_int_return_narrowed_macos_aarch64() {
    assert_target_normalized("macos-aarch64");
}

#[test]
fn fsync_and_close_int_return_narrowed_linux_x86_64() {
    assert_target_normalized("linux-x86_64");
}

#[test]
fn fsync_and_close_int_return_narrowed_linux_aarch64() {
    assert_target_normalized("linux-aarch64");
}

#[test]
fn fsync_and_close_int_return_narrowed_linux_riscv64() {
    assert_target_normalized("linux-riscv64");
}
