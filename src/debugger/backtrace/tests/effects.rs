//! `state_at` over the less common prologue and epilogue shapes: SIMD and
//! post-index saves, saves through a register set from `sp` or `fp`, and the
//! writes to `sp` the walk cannot follow. Encodings are the assembler's
//! (`clang -c -target arm64-apple-macos`, then `objdump -d`).

use super::*;

const STR_X30_SP: u32 = 0xf900_03fe;
const MOV_FP_SP: u32 = 0x9100_03fd;
const STP_FP_LR_PRE: u32 = 0xa9bf_7bfd;
const MOV_X0_X1: u32 = 0xaa01_03e0;

fn state(words: &[u32]) -> FrameState {
    state_at(words, 0x1000, 0x1000, Some(0x2000))
}

fn sp(delta: i64) -> FrameState {
    FrameState {
        sp: Some(delta),
        ..FrameState::ENTRY
    }
}

/// A SIMD pair's immediate is scaled by the register size: 8 for a `d`
/// pair, 16 for a `q` pair (Arm ARM, LDP/STP SIMD&FP: `scale = 2 + opc`).
#[test]
fn a_simd_pair_save_moves_sp_by_its_register_size() {
    // stp d8, d9, [sp, #-0x10]!
    assert_eq!(state(&[0x6dbf_27e8]), sp(-0x10));
    // stp q8, q9, [sp, #-0x20]!
    assert_eq!(state(&[0xadbf_27e8]), sp(-0x20));
    // ...and the matching post-index loads give it back.
    // ldp d8, d9, [sp], #0x10 / ldp q8, q9, [sp], #0x20
    assert_eq!(state(&[0x6dbf_27e8, 0x6cc1_27e8]), FrameState::ENTRY);
    assert_eq!(state(&[0xadbf_27e8, 0xacc1_27e8]), FrameState::ENTRY);
}

#[test]
fn a_pre_index_save_moves_sp_first_and_a_post_index_one_after() {
    // str x30, [sp, #-0x10]!: lr lands at the new sp.
    assert_eq!(
        state(&[0xf81f_0ffe]),
        FrameState {
            sp: Some(-0x10),
            lr: Some(-0x10),
            ..FrameState::ENTRY
        }
    );
    // str x30, [sp], #-0x10: lr lands at the old sp, which then moves.
    assert_eq!(
        state(&[0xf81f_07fe]),
        FrameState {
            sp: Some(-0x10),
            lr: Some(0),
            ..FrameState::ENTRY
        }
    );
    // stp x29, x30, [sp], #0x10: both saved at the old sp, sp moves up.
    assert_eq!(
        state(&[0xa881_7bfd]),
        FrameState {
            sp: Some(0x10),
            lr: Some(8),
            fp_saved: Some(0),
            fp: None,
        }
    );
    // stp x29, x30, [sp, #0x10]: a signed offset, sp unmoved.
    assert_eq!(
        state(&[0xa901_7bfd]),
        FrameState {
            sp: Some(0),
            lr: Some(0x18),
            fp_saved: Some(0x10),
            fp: None,
        }
    );
}

#[test]
fn a_post_index_reload_of_lr_gives_back_its_stack() {
    // str x30, [sp, #-0x10]! then ldr x30, [sp], #0x10.
    assert_eq!(state(&[0xf81f_0ffe, 0xf841_07fe]), FrameState::ENTRY);
    // ldr x21, [sp] restores neither fp nor lr.
    let saved = state(&[0xf81f_0ffe]);
    assert_eq!(state(&[0xf81f_0ffe, 0xf940_03f5]), saved);
}

#[test]
fn a_save_through_a_register_set_from_sp_is_followed() {
    // sub sp, sp, #0x150; add x17, sp, #0x20; str x30, [x17]
    let words = [0xd105_43ff, 0x9100_83f1, 0xf900_023e];
    assert_eq!(state(&words).lr, Some(-0x150 + 0x20));
    // str x30, [x17, #-0x8]!: x17 moves down 8 first.
    let words = [0xd105_43ff, 0x9100_83f1, 0xf81f_8e3e];
    assert_eq!(state(&words).lr, Some(-0x150 + 0x20 - 8));
    // A register never set from sp is not a frame address: nothing recorded.
    // str x30, [x17] alone.
    assert_eq!(state(&[0xf900_023e]), FrameState::ENTRY);
}

#[test]
fn a_save_through_fp_is_followed_and_fp_moves_with_a_writeback() {
    let record = [STP_FP_LR_PRE, MOV_FP_SP];
    // str x30, [x29, #0x8]: fp is CFA-0x10, so lr is at CFA-0x8.
    let mut words = record.to_vec();
    words.push(0xf900_07be);
    assert_eq!(state(&words).lr, Some(-8));
    // stp x19, x20, [x29, #-0x10]!: fp moves down 0x10.
    let mut words = record.to_vec();
    words.push(0xa9bf_53b3);
    assert_eq!(state(&words).fp, Some(-0x20));
}

#[test]
fn sp_restored_from_fp_follows_fp_and_other_writes_lose_it() {
    // A frame record, then sp moved by a register, then sub sp, x29, #0x10.
    let words = [STP_FP_LR_PRE, MOV_FP_SP, 0xcb30_63ff, 0xd100_43bf];
    assert_eq!(state(&words).sp, Some(-0x20));
    // mov sp, x29 (add sp, x29, #0).
    assert_eq!(
        state(&[STP_FP_LR_PRE, MOV_FP_SP, 0x9100_03bf]).sp,
        Some(-0x10)
    );
    // mov sp, x9 (add sp, x9, #0): no way to follow it.
    assert_eq!(state(&[0x9100_013f]).sp, None);
    // A store through sp once sp is lost records nothing.
    assert_eq!(state(&[0x9100_013f, STR_X30_SP]).lr, None);
    // add x0, x1, #0x10 touches no frame register.
    assert_eq!(state(&[0x9100_4020]), FrameState::ENTRY);
}

#[test]
fn simd_and_word_saves_and_indirect_branches() {
    // str d8, [sp, #0x8] is a prologue save: the body starts after it, so a
    // later `ret` restores the state with lr saved.
    // sub sp, sp, #0x150; str x30, [sp]; str d8, [sp, #0x8]; mov x0, x1;
    // ldr x30, [sp]; br x16
    let words = [
        0xd105_43ff,
        STR_X30_SP,
        0xfd00_07e8,
        MOV_X0_X1,
        0xf940_03fe,
        0xd61f_0200,
    ];
    assert_eq!(state(&words), state(&words[..3]));
    // stp w0, w1, [sp] is neither a frame move nor a save of fp or lr.
    assert_eq!(state(&[0x2900_07e0]), FrameState::ENTRY);
}
