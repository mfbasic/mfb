//! bug-512: the program entry's RNG-seed scratch word must sit ABOVE the arena
//! state on every target, including Win64.
//!
//! The entry frame is `subtract_stack(ENTRY_STACK_SIZE + shadow)` with the arena
//! pinned at `sp + shadow` (Win64 reserves its 32-byte callee shadow space at the
//! frame bottom). The seed block writes 8 `BCryptGenRandom`/`getentropy` bytes to
//! an sp-relative scratch slot meant to be the one word past the arena state
//! (`ENTRY_SEED_SCRATCH_OFFSET == ARENA_STATE_SIZE`). Before the fix that offset
//! was NOT shifted by `shadow`, so on Win64 the write landed INSIDE the arena at
//! `arena + ARENA_STATE_SIZE - 32 == ARENA_STDIN_LOCAL_BUF_OFFSET` — the slot the
//! stdin path reads as an already-allocated 4 KiB buffer pointer ("NULL => not yet
//! allocated"). Eight CSPRNG bytes became a pointer stdin bytes were copied
//! through: an arbitrary pointer read/write. Linux/macOS (`shadow == 0`) were
//! never affected.
//!
//! No Windows box is needed: the entry is emitted the same way on every host, so
//! this inspects the `-ncode -target windows-x86_64` dump. It recovers the arena
//! base (`add_imm <arena>, rsp, <shadow>`), the arena-state size (the zero-loop
//! bound `add_imm <limit>, <arena>, ARENA_STATE_SIZE`) and the seed store
//! (`str_u64 <arena>, [rsp + K]`, the pre-fill before the `bl _mfb_rng_seed_at`),
//! then asserts the seed slot's arena-relative offset is past the arena state AND
//! collides with none of the arena offsets the stdin reader (`runtime.stdin.next_byte`,
//! symbol `_mfb_rt_stdin_next_byte`) dereferences.
//! `linux-x86_64` runs the same assertions as the `shadow == 0` control.

mod common;
use common::{build_ncode, temp_project};

use serde_json::Value;

// The MEM-40 trigger shape: `math::rand` makes the entry seed the RNG (the
// writer); `io::input` makes the module embed the stdin reader (the reader).
const SOURCE: &str = "\
IMPORT io\n\
IMPORT math\n\
\n\
SUB main()\n\
  LET n AS Integer = math::rand(1, 6)\n\
  LET line AS String = io::input(\"? \")\n\
  io::print(toString(n) & \":\" & line)\n\
END SUB\n";

fn imm(inst: &Value, key: &str) -> Option<usize> {
    inst[key].as_str().and_then(|s| s.parse::<usize>().ok())
}

fn function<'a>(ncode: &'a Value, name: &str) -> &'a [Value] {
    ncode["functions"]
        .as_array()
        .expect("ncode has a functions array")
        .iter()
        .find(|f| f["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("ncode has no function {name}"))["instructions"]
        .as_array()
        .expect("function has instructions")
}

/// `(arena_register, shadow, arena_state_size, seed_slot_sp_offset)` from the
/// program entry.
fn entry_layout(entry: &[Value]) -> (String, usize, usize, usize) {
    // First `add_imm <reg>, rsp, <shadow>` after the frame is carved: the arena pin.
    let (arena_idx, arena_reg, shadow) = entry
        .iter()
        .enumerate()
        .find_map(|(i, inst)| {
            (inst["op"].as_str() == Some("add_imm") && inst["src"].as_str() == Some("rsp")).then(
                || {
                    (
                        i,
                        inst["dst"].as_str().unwrap().to_string(),
                        imm(inst, "imm").unwrap(),
                    )
                },
            )
        })
        .expect("entry pins the arena register off rsp");
    // The zero loop's limit is `arena + ARENA_STATE_SIZE`, emitted right after the pin.
    let arena_state_size = entry[arena_idx..]
        .iter()
        .find_map(|inst| {
            (inst["op"].as_str() == Some("add_imm")
                && inst["src"].as_str() == Some(arena_reg.as_str()))
            .then(|| imm(inst, "imm").unwrap())
        })
        .expect("entry computes the arena-state zero-loop limit off the arena register");
    // The seed block ends with `bl _mfb_rng_seed_at`, whose second argument is
    // read back from the seed slot. Anchor there rather than on the entropy call:
    // Win64 calls `BCryptGenRandom`, Linux issues a raw `getentropy` syscall.
    let seed_call_idx = entry
        .iter()
        .position(|inst| {
            inst["op"].as_str() == Some("bl") && inst["target"].as_str() == Some("_mfb_rng_seed_at")
        })
        .expect("entry seeds the RNG through _mfb_rng_seed_at");
    let before_seed = &entry[..seed_call_idx];
    // The pre-fill: `str_u64 <arena>, [rsp + K]` (the arena address is the fallback seed).
    let seed_slot = before_seed
        .iter()
        .rev()
        .find_map(|inst| {
            (inst["op"].as_str() == Some("str_u64")
                && inst["src"].as_str() == Some(arena_reg.as_str())
                && inst["base"].as_str() == Some("rsp"))
            .then(|| imm(inst, "offset").unwrap())
        })
        .expect("entry pre-fills the seed scratch with the arena address");
    // Sanity: the same slot is what the entropy call is pointed at and read back from.
    let pointed = before_seed.iter().rev().find_map(|inst| {
        (inst["op"].as_str() == Some("add_imm") && inst["src"].as_str() == Some("rsp"))
            .then(|| imm(inst, "imm").unwrap())
    });
    assert_eq!(
        pointed,
        Some(seed_slot),
        "entropy buffer pointer must address the seed slot"
    );
    let read_back = before_seed.iter().rev().find_map(|inst| {
        (inst["op"].as_str() == Some("ldr_u64") && inst["base"].as_str() == Some("rsp"))
            .then(|| imm(inst, "offset").unwrap())
    });
    assert_eq!(
        read_back,
        Some(seed_slot),
        "seed read-back must address the seed slot"
    );
    (arena_reg, shadow, arena_state_size, seed_slot)
}

/// Every arena-relative offset the stdin reader materializes as
/// `mov_imm <r>, K ; … ; add <r>, <arena>, <r>` (the past-`addi`-range access
/// shape). Register-allocator spills/reloads may sit between the two, so the
/// `add` is looked for within a short window rather than adjacently.
fn stdin_arena_offsets(reader: &[Value], arena_reg: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    for (i, inst) in reader.iter().enumerate() {
        if inst["op"].as_str() != Some("mov_imm") {
            continue;
        }
        let Some(k) = inst["value"].as_str().and_then(|s| s.parse::<usize>().ok()) else {
            continue;
        };
        let dst = inst["dst"].as_str().unwrap_or("");
        let is_arena_add = |n: &Value| {
            n["op"].as_str() == Some("add")
                && ((n["lhs"].as_str() == Some(arena_reg) && n["rhs"].as_str() == Some(dst))
                    || (n["rhs"].as_str() == Some(arena_reg) && n["lhs"].as_str() == Some(dst)))
        };
        if reader[i + 1..].iter().take(8).any(is_arena_add) {
            offsets.push(k);
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    assert!(
        !offsets.is_empty(),
        "stdin reader dereferences no arena-relative slot; the inspection shape drifted"
    );
    offsets
}

fn assert_seed_scratch_is_past_the_arena(target: &str) {
    let project = temp_project("codegen_win64_entry_seed_scratch", SOURCE);
    let ncode = build_ncode(&project, target, "codegen_win64_entry_seed_scratch");
    let entry = function(&ncode, "program.entry");
    let (arena_reg, shadow, arena_state_size, seed_slot) = entry_layout(entry);
    assert!(
        seed_slot >= shadow,
        "{target}: seed slot sp+{seed_slot} is below the arena base sp+{shadow}"
    );
    let seed_arena_offset = seed_slot - shadow;
    assert!(
        seed_arena_offset >= arena_state_size,
        "{target}: seed scratch at arena+{seed_arena_offset} lies INSIDE the arena state \
         (ARENA_STATE_SIZE={arena_state_size}, shadow={shadow}) — bug-512"
    );
    let stdin_offsets =
        stdin_arena_offsets(function(&ncode, "runtime.stdin.next_byte"), &arena_reg);
    assert!(
        !stdin_offsets.contains(&seed_arena_offset),
        "{target}: seed scratch arena+{seed_arena_offset} aliases a stdin arena slot {stdin_offsets:?}"
    );
    for off in &stdin_offsets {
        assert!(
            *off < arena_state_size,
            "{target}: stdin slot arena+{off} lies past ARENA_STATE_SIZE={arena_state_size}"
        );
    }
}

#[test]
fn win64_entry_seed_scratch_lies_past_the_arena_state() {
    assert_seed_scratch_is_past_the_arena("windows-x86_64");
}

#[test]
fn linux_x86_64_entry_seed_scratch_lies_past_the_arena_state() {
    assert_seed_scratch_is_past_the_arena("linux-x86_64");
}
