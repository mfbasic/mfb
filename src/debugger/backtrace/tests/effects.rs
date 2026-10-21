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
