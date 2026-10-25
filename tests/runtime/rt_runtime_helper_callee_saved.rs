//! bug-774: the runtime helpers the register allocator trusts to preserve the
//! callee-saved registers really do, on every target.
//!
//! `call_clobber_mask` (`src/codegen/engine/regalloc/analysis.rs`) treats a call to
//! an `_mfb_*` runtime helper as destroying every integer register, except for the
//! helpers in `PCS_CLEAN_RUNTIME_HELPERS`, which clobber only the PCS caller-saved
//! set. A value live across one of those calls may therefore sit in a callee-saved
//! register — a loop's whole state, across an in-place `append`'s grow path. If a
//! helper ever wrote a callee-saved register without saving it, that value would be
//! silently corrupted in the caller, and only past whatever threshold first takes
//! the grow path.
//!
//! This test re-derives the audit from the `-ncode` dump of each target: starting at
//! the trusted helpers and following every `bl` into another `_mfb_*` body, each
//! function must save, in its frame, every callee-saved register it names. The one
//! exception is the module-globals base (`x19`, `r15`, `s11`), which a helper may
//! read but never write. A call to a non-`_mfb_` symbol (the OS allocator) is an
//! ABI call and preserves callee-saved registers by that ABI.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, temp_project};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// Mirrors `PCS_CLEAN_RUNTIME_HELPERS` in `src/codegen/engine/regalloc/analysis.rs`.
const TRUSTED: &[&str] = &[
    "_mfb_arena_alloc",
    "_mfb_arena_free",
    "_mfb_rt_drop_owned_collection",
];

/// A program that reaches all three: an append that grows, a list dropped at scope
/// end, and a temporary freed after use.
const SOURCE: &str = "\
IMPORT io\n\
IMPORT collections\n\
FUNC build(n AS Integer) AS List OF Integer\n\
  MUT out AS List OF Integer = []\n\
  FOR i = 1 TO n\n\
    out = collections::append(out, i)\n\
  NEXT\n\
  RETURN out\n\
END FUNC\n\
FUNC main() AS Integer\n\
  MUT total AS Integer = 0\n\
  FOR r = 1 TO 3\n\
    LET xs AS List OF Integer = build(100 * r)\n\
    total = total + len(collections::mid(xs, 1, 10))\n\
  NEXT\n\
  io::print(toString(total))\n\
  RETURN 0\n\
END FUNC\n";

/// The callee-saved integer registers of a target's ABI, and its globals base.
fn abi(target: &str) -> (BTreeSet<String>, &'static str) {
    let names: Vec<String> = match target {
        "macos-aarch64" | "linux-aarch64" => (19..=28).map(|i| format!("x{i}")).collect(),
        "linux-riscv64" => (0..=11).map(|i| format!("s{i}")).collect(),
        "linux-x86_64" => ["rbx", "rbp", "r12", "r13", "r14", "r15"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        // Win64 adds `rdi` and `rsi` to the SysV set, and makes `xmm6`–`xmm15`
        // callee-saved (bug-994).
        "windows-x86_64" => ["rbx", "rbp", "rdi", "rsi", "r12", "r13", "r14", "r15"]
            .iter()
            .map(|s| s.to_string())
            .chain((6..=15).map(|i| format!("xmm{i}")))
            .collect(),
        other => panic!("no ABI table for {other}"),
    };
    let globals = match target {
        "macos-aarch64" | "linux-aarch64" => "x19",
        "linux-riscv64" => "s11",
        _ => "r15",
    };
    (names.into_iter().collect(), globals)
}

fn audit(target: &str) {
    let project = temp_project("helper_callee_saved", SOURCE);
    let ncode = build_ncode(&project, target, "helper_callee_saved");
    let _ = std::fs::remove_dir_all(&project);
    let functions: HashMap<&str, &Value> = ncode["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .filter_map(|f| Some((f["symbol"].as_str()?, f)))
        .collect();
    let (callee_saved, globals) = abi(target);
    let mut seen = BTreeSet::new();
    let mut todo: Vec<String> = TRUSTED.iter().map(|s| s.to_string()).collect();
    while let Some(symbol) = todo.pop() {
        if !seen.insert(symbol.clone()) {
            continue;
        }
        let function = functions
            .get(symbol.as_str())
            .unwrap_or_else(|| panic!("{target}: no body for `{symbol}` in the -ncode dump"));
        let saved: BTreeSet<&str> = function["frame"]["calleeSaved"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for instruction in function["instructions"].as_array().expect("instructions") {
            let fields = instruction.as_object().expect("instruction object");
            for (key, value) in fields {
                let Some(register) = value.as_str() else {
                    continue;
                };
                if !callee_saved.contains(register) {
                    continue;
                }
                if register == globals {
                    // `dst2` is an `ldp`'s second destination (plan-171-A).
                    assert!(
                        key != "dst" && key != "dst2",
                        "{target}: `{symbol}` writes the globals base `{register}`: {instruction}"
                    );
                    continue;
                }
                assert!(
                    saved.contains(register),
                    "{target}: `{symbol}` names callee-saved `{register}` without saving it \
                     in its frame ({saved:?}): {instruction}"
                );
            }
            if fields.get("op").and_then(Value::as_str) == Some("bl") {
                let callee = fields["target"].as_str().expect("bl target");
                if callee.starts_with("_mfb_") {
                    todo.push(callee.to_string());
                }
            }
        }
    }
    // The walk reached past the three roots (the allocator's own helpers).
    assert!(
        seen.len() > TRUSTED.len(),
        "{target}: audited only {seen:?}"
    );
}

#[test]
fn trusted_helpers_preserve_callee_saved_on_macos_aarch64() {
    audit("macos-aarch64");
}

#[test]
fn trusted_helpers_preserve_callee_saved_on_linux_aarch64() {
    audit("linux-aarch64");
}

#[test]
fn trusted_helpers_preserve_callee_saved_on_linux_x86_64() {
    audit("linux-x86_64");
}

#[test]
fn trusted_helpers_preserve_callee_saved_on_windows_x86_64() {
    audit("windows-x86_64");
}

#[test]
fn trusted_helpers_preserve_callee_saved_on_linux_riscv64() {
    audit("linux-riscv64");
}
