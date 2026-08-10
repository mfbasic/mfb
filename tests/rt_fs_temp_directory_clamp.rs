//! sec-3: `fs::tempDirectory()` forwards the platform hook's returned length into
//! the String allocation + copy loop. macOS `emit_temp_directory` forwards
//! `confstr`'s return, which per its contract is the size *required* to hold the
//! full path and can EXCEED the fixed 4096-byte buffer on truncation — so an
//! unclamped value would read past the buffer (an over-read). The shared caller
//! `lower_fs_temp_directory_helper` now clamps the returned length to the buffer
//! capacity (4096) before it drives the allocation.
//!
//! No real macOS produces a >4096 confstr result, so the over-read cannot be
//! observed at runtime on the dev host. This locks the codegen structure instead:
//! the `_mfb_rt_fs_fs_tempDirectory` helper must contain the clamp seam
//! (`cmp ret,4096` → `b.le …_temp_len_clamped` → `mov ret,4096`) before it uses
//! the length. It fails the moment the clamp is dropped from the shared caller.

mod common;
use common::build_ncode;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const SOURCE: &str = "IMPORT io\nIMPORT fs\n\nFUNC main AS Integer\n  io::print(fs::tempDirectory())\n  RETURN 0\nEND FUNC\n";

const HELPER: &str = "_mfb_rt_fs_fs_tempDirectory";

fn op_name(op: &Value) -> &str {
    op.get("op").and_then(Value::as_str).unwrap_or("")
}

/// The clamp seam pins the returned length (in the C return register, `x0` on
/// aarch64) to the 4096 buffer capacity: `cmp x0,4096` / `b.le …_temp_len_clamped`
/// / `mov x0,4096`. The `…_temp_len_clamped` branch target is unique to this
/// guard, so it is the anchor.
fn assert_clamp_present(target: &str) {
    let name = format!("fs_tempdir_clamp_{}", target.replace('-', "_"));
    let project = common::temp_project(&name, SOURCE);
    let ncode = build_ncode(&project, target, &name);

    let func = ncode["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(HELPER))
        .unwrap_or_else(|| panic!("{target}: helper {HELPER} not emitted"));
    let ins = func["instructions"].as_array().expect("instructions array");

    // Find the clamp branch (its target is the sec-3-specific label), then verify
    // the compare-against-4096 immediately precedes it and the saturating move to
    // 4096 immediately follows it.
    let mut found = false;
    for (i, op) in ins.iter().enumerate() {
        if op_name(op) != "b.le" {
            continue;
        }
        let is_clamp = op
            .get("target")
            .and_then(Value::as_str)
            .is_some_and(|t| t.ends_with("_temp_len_clamped"));
        if !is_clamp {
            continue;
        }
        let prev = &ins[i - 1];
        assert_eq!(
            op_name(prev),
            "cmp_imm",
            "{target}: clamp branch not preceded by a compare (sec-3):\n{prev}"
        );
        assert_eq!(
            prev.get("rhs").and_then(Value::as_str),
            Some("4096"),
            "{target}: clamp compares against the wrong capacity (sec-3):\n{prev}"
        );
        let next = &ins[i + 1];
        assert_eq!(
            op_name(next),
            "mov_imm",
            "{target}: clamp branch not followed by a saturating move (sec-3):\n{next}"
        );
        assert_eq!(
            next.get("value").and_then(Value::as_str),
            Some("4096"),
            "{target}: clamp saturates to the wrong capacity (sec-3):\n{next}"
        );
        found = true;
        break;
    }
    assert!(
        found,
        "{target}: {HELPER} has no `…_temp_len_clamped` clamp seam — the confstr \
         return length is used unclamped (sec-3 regression)"
    );

    let _ = fs::remove_dir_all(&project);
    let _ = PathBuf::new();
}

#[test]
fn temp_directory_length_is_clamped_macos_aarch64() {
    assert_clamp_present("macos-aarch64");
}
