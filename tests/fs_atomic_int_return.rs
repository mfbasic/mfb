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
//! (the tmpfs/NFS harness in `planning/bug-44-c-int-return-width-fsync-close.md`),
//! which the macOS dev host cannot stage. This test therefore locks the codegen
//! structure across all four backends instead: every checked `fsync`/`close`
//! call is immediately followed by a `sxtw`, and never directly by a
//! compare/branch. It fails the moment the seam normalization is dropped from
//! any backend.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The atomic-write helpers whose `fsync`/`close` results are relationally
/// compared. `fs.createTempFile` only closes on an already-failing cleanup path
/// (result unchecked), so it is deliberately excluded.
const HELPERS: &[&str] = &[
    "_mfb_rt_fs_fs_writeTextAtomic",
    "_mfb_rt_fs_fs_writeBytesAtomic",
];

const SOURCE: &str = "IMPORT fs\nIMPORT strings\n\nFUNC main AS Integer\n  fs::writeTextAtomic(\"/tmp/mfb_bug44/a.txt\", \"x\")\n  fs::writeBytesAtomic(\"/tmp/mfb_bug44/b.bin\", strings::toBytes(\"y\"))\n  RETURN 0\nEND FUNC\n";

fn temp_project(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
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
        let next = ins.get(i + 1).map(op_name).unwrap_or("");

        // Regression guard: a checked site must never feed the compare/branch
        // straight from the raw C `int` — the `sxtw` seam must intervene.
        assert!(
            !is_compare_or_branch(next),
            "{target}/{symbol}: `{kind}` result flows into `{next}` without a \
             sign-extend seam — bug-44 regression (the C int return is compared \
             at 64 bits with the upper word unnormalized)",
        );

        if next == "sxtw" {
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
