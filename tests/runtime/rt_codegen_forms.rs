//! plan-171: the instruction forms the native backends emit, read from the
//! `-ncode` dump of a program built for each target. Every plan-171 letter adds
//! the forms it teaches the backends here.
//!
//! plan-171-A — AArch64 load/store pairs. A frame saves every two adjacent
//! callee-saved slots of one kind with one `stp` and restores them with one
//! `ldp`; x86-64 and riscv64 have no pair form and never receive one.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, temp_project};
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
