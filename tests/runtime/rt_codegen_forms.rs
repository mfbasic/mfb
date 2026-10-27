//! plan-171: the instruction forms the native backends emit, read from the
//! `-ncode` dump of a program built for each target. Every plan-171 letter adds
//! the forms it teaches the backends here.
//!
//! plan-171-A — AArch64 load/store pairs. A frame saves every two adjacent
//! callee-saved slots of one kind with one `stp` and restores them with one
//! `ldp`; x86-64 and riscv64 have no pair form and never receive one.
//!
//! plan-171-B — compare-and-branch forms. A compare with zero becomes `cbz`/
//! `cbnz` (and a sign test `tbz`/`tbnz #63`) on AArch64, a single-bit test
//! `tbz`/`tbnz` on every target; x86-64 and riscv64 never receive `cbz`. At
//! `-O0` the row forms nothing.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, build_ncode_opt, temp_project};
use serde_json::Value;

/// The `tests/rt-behavior/functions/frame-pair-saves` program: `mix` keeps
/// seven values live across its calls, so its frame saves x21–x27 and `lr`.
const FRAME_PAIR_SAVES: &str =
    include_str!("../rt-behavior/functions/frame-pair-saves/src/main.mfb");

fn dump(target: &str) -> Value {
    let project = temp_project("frame_pair_saves", FRAME_PAIR_SAVES);
    let ncode = build_ncode(&project, target, "frame_pair_saves");
    let _ = std::fs::remove_dir_all(&project);
    ncode
}

fn functions(ncode: &Value) -> &Vec<Value> {
    ncode["functions"].as_array().expect("functions")
}

fn field<'a>(instruction: &'a Value, key: &str) -> Option<&'a str> {
    instruction.get(key).and_then(Value::as_str)
}

fn ops<'a>(function: &'a Value) -> impl Iterator<Item = &'a str> {
    function["instructions"]
        .as_array()
        .expect("instructions")
        .iter()
        .filter_map(|instruction| field(instruction, "op"))
}

/// An integer callee-saved register a frame records (`x19`–`x28`, or `lr`).
fn integer_save(register: &str) -> bool {
    register == "lr" || register.starts_with('x')
}

fn frame_saves_use_stp(target: &str) {
    let ncode = dump(target);
    let mix = functions(&ncode)
        .iter()
        .find(|f| f["name"] == "mix")
        .unwrap_or_else(|| panic!("{target}: no `mix` in the dump"));
    let saved = mix["frame"]["calleeSaved"].as_array().expect("calleeSaved");
    assert_eq!(
        saved.len(),
        8,
        "{target}: mix saves x21–x27 and lr: {saved:?}"
    );
    assert_eq!(
        ops(mix).filter(|op| *op == "stp").count(),
        4,
        "{target}: mix prologue"
    );
    assert!(
        ops(mix).filter(|op| *op == "ldp").count() >= 4,
        "{target}: mix epilogue"
    );

    // No function — compiled or runtime helper — saves two adjacent integer
    // slots of its frame with two single stores.
    for function in functions(&ncode) {
        let saved: Vec<&str> = function["frame"]["calleeSaved"]
            .as_array()
            .expect("calleeSaved")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let instructions = function["instructions"].as_array().expect("instructions");
        let saves = instructions.iter().map(|i| {
            (field(i, "op") == Some("str_u64")
                && field(i, "base") == Some("sp")
                && field(i, "src").is_some_and(|r| saved.contains(&r) && integer_save(r)))
            .then(|| field(i, "offset").and_then(|o| o.parse::<i64>().ok()))
            .flatten()
        });
        let saves: Vec<Option<i64>> = saves.collect();
        for (index, pair) in saves.windows(2).enumerate() {
            if let [Some(a), Some(b)] = pair {
                assert!(
                    (a - b).abs() != 8,
                    "{target}: {} saves adjacent slots with two `str`: {} / {}",
                    function["name"],
                    instructions[index],
                    instructions[index + 1]
                );
            }
        }
    }
}

#[test]
fn frame_saves_use_stp_on_macos_aarch64() {
    frame_saves_use_stp("macos-aarch64");
}

#[test]
fn frame_saves_use_stp_on_linux_aarch64() {
    frame_saves_use_stp("linux-aarch64");
}

#[test]
fn no_pair_form_reaches_x86_64_or_riscv64() {
    for target in ["linux-x86_64", "linux-riscv64", "windows-x86_64"] {
        let ncode = dump(target);
        for function in functions(&ncode) {
            assert!(
                ops(function).all(|op| op != "stp" && op != "ldp"),
                "{target}: {} carries a pair",
                function["name"]
            );
        }
    }
}

/// The `tests/rt-behavior/control-flow/branch-zero-and-bit-tests` program.
const BRANCH_TESTS: &str =
    include_str!("../rt-behavior/control-flow/branch-zero-and-bit-tests/src/main.mfb");

fn branch_dump(target: &str, level: &str) -> Value {
    let project = temp_project("branch_zero_and_bit_tests", BRANCH_TESTS);
    let ncode = build_ncode_opt(&project, target, "branch_zero_and_bit_tests", level);
    let _ = std::fs::remove_dir_all(&project);
    ncode
}

fn function<'a>(ncode: &'a Value, name: &str, target: &str) -> &'a Value {
    functions(ncode)
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("{target}: no `{name}` in the dump"))
}

/// The bits `function`'s `tbz`/`tbnz` test, sorted.
fn tested_bits(function: &Value) -> Vec<u32> {
    let mut bits: Vec<u32> = function["instructions"]
        .as_array()
        .expect("instructions")
        .iter()
        .filter(|i| matches!(field(i, "op"), Some("tbz" | "tbnz")))
        .filter_map(|i| field(i, "bit").and_then(|b| b.parse().ok()))
        .collect();
    bits.sort_unstable();
    bits.dedup();
    bits
}

#[test]
fn compare_with_zero_and_bit_tests_use_the_aarch64_forms() {
    for target in ["macos-aarch64", "linux-aarch64"] {
        let ncode = branch_dump(target, "1");
        let zero = function(&ncode, "zeroTest", target);
        assert!(ops(zero).any(|op| op == "cbz"), "{target}: IF x = 0");
        assert!(ops(zero).any(|op| op == "cbnz"), "{target}: IF x <> 0");
        assert_eq!(tested_bits(zero), vec![63], "{target}: the sign tests");
        let bits = function(&ncode, "bitTests", target);
        assert_eq!(tested_bits(bits), vec![0, 10, 11, 30, 31, 62], "{target}");
        assert_eq!(
            tested_bits(function(&ncode, "far", target)),
            vec![11],
            "{target}"
        );
    }
}

#[test]
fn bit_tests_use_test_or_bt_on_x86_64_and_no_cbz_reaches_it() {
    for target in ["linux-x86_64", "windows-x86_64"] {
        let ncode = branch_dump(target, "1");
        let bits = function(&ncode, "bitTests", target);
        assert_eq!(tested_bits(bits), vec![0, 10, 11, 30, 31, 62], "{target}");
        for f in functions(&ncode) {
            assert!(
                ops(f).all(|op| op != "cbz" && op != "cbnz"),
                "{target}: {} carries a cbz",
                f["name"]
            );
        }
    }
}

#[test]
fn bit_tests_use_a_scratch_shift_or_mask_on_riscv64() {
    let ncode = branch_dump("linux-riscv64", "1");
    let bits = function(&ncode, "bitTests", "linux-riscv64");
    assert_eq!(tested_bits(bits), vec![0, 10, 11, 30, 31, 62]);
    for f in functions(&ncode) {
        for i in f["instructions"].as_array().expect("instructions") {
            let op = field(i, "op");
            assert!(!matches!(op, Some("cbz" | "cbnz")), "{}: cbz", f["name"]);
            if matches!(op, Some("tbz" | "tbnz")) {
                assert!(field(i, "scratch").is_some(), "{}: {i}", f["name"]);
            }
        }
    }
}

#[test]
fn at_o0_no_compare_and_branch_form_is_formed() {
    for target in ["macos-aarch64", "linux-x86_64", "linux-riscv64"] {
        let ncode = branch_dump(target, "0");
        for f in functions(&ncode) {
            assert!(
                ops(f).all(|op| !matches!(op, "cbz" | "cbnz" | "tbz" | "tbnz")),
                "{target}: {} at -O0",
                f["name"]
            );
        }
    }
}
