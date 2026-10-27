//! plan-171-B: riscv64 `tbz`/`tbnz` — `andi` (bits 0–10) or `slli` (any other
//! bit) into the scratch, then `rv.br`'s long form — and the refused
//! AArch64-only `cbz`/`cbnz`. Oracle words: `riscv64-unknown-elf-as
//! -march=rv64gc` (`.option norvc`) of the spelled sequences, read back with
//! `riscv64-unknown-elf-objdump -d`.

use super::*;

fn branch(op: &str, register: &str, bit: &str, scratch: &str, target: &str) -> CodeInstruction {
    ci(
        op,
        &[
            ("register", register),
            ("bit", bit),
            ("scratch", scratch),
            ("target", target),
        ],
    )
}

#[test]
fn encodes_both_bit_test_shapes_at_their_boundaries() {
    let text = encode_text(vec![
        // andi t6, t3, 1; bnez t6, +8; j l1
        branch("tbz", "t3", "0", "t6", "l1"),
        // andi t4, s1, 1024; beqz t4, +8; j l1
        branch("tbnz", "s1", "10", "t4", "l1"),
        // slli t5, t3, 52; bltz t5, +8; j l1
        branch("tbz", "t3", "11", "t5", "l1"),
        // slli t5, s9, 0; bgez t5, +8; j l1
        branch("tbnz", "s9", "63", "t5", "l1"),
        ci("label", &[("name", "l1")]),
        ci("ret", &[]),
    ]);
    assert_eq!(
        words(&text),
        vec![
            0x001e_7f93,
            0x000f_9463,
            0x0280_006f,
            0x4004_fe93,
            0x000e_8463,
            0x01c0_006f,
            0x034e_1f13,
            0x000f_4463,
            0x0100_006f,
            0x000c_9f13,
            0x000f_5463,
            0x0040_006f,
            0x0000_8067,
        ]
    );
}

#[test]
fn refuses_cbz_and_a_bad_bit_test() {
    for op in ["cbz", "cbnz"] {
        let ins = ci(op, &[("register", "t3"), ("target", "l")]);
        let err = try_encode(vec![ins, ci("ret", &[])]).unwrap_err();
        assert!(err.contains("AArch64-only"), "{op}: {err}");
    }
    let bad_bit = branch("tbz", "t3", "64", "t4", "l");
    assert!(try_encode(vec![bad_bit, ci("label", &[("name", "l")])])
        .unwrap_err()
        .contains("outside 0..=63"));
    let zero_scratch = branch("tbz", "t3", "3", "zero", "l");
    assert!(
        try_encode(vec![zero_scratch, ci("label", &[("name", "l")])])
            .unwrap_err()
            .contains("scratch")
    );
}
