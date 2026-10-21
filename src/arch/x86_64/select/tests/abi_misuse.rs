//! The x86-64 ABI-token realization refuses the constructions that cannot be right.
//!
//! These are construction bugs, not user input, so each is a panic with a message
//! naming it: a token past the end of its register bank would otherwise realize to
//! the wrong register and corrupt a call quietly, and a residual `xN` means a
//! token was never realized.

use super::*;

#[test]
#[should_panic(expected = "Win64 emits no syscall boundary")]
fn a_syscall_argument_token_under_win64_is_refused() {
    realize_abi_operand(AbiConvention::Sys, AbiRole::Arg, 0, X86Abi::Win64);
}

#[test]
#[should_panic(expected = "Win64 emits no syscall boundary")]
fn a_syscall_result_token_under_win64_is_refused() {
    realize_abi_operand(AbiConvention::Sys, AbiRole::Ret, 0, X86Abi::Win64);
}

#[test]
#[should_panic(expected = "ABI token index 9 out of range")]
fn a_token_past_the_end_of_its_bank_is_refused() {
    realize_abi_operand(AbiConvention::Mfb, AbiRole::Arg, 9, X86Abi::SysV);
}

#[test]
fn the_syscall_result_is_rax_on_sysv() {
    assert_eq!(
        realize_abi_operand(AbiConvention::Sys, AbiRole::Ret, 0, X86Abi::SysV),
        "rax"
    );
}

/// A register spelled `x5` after ABI-token realization is a token nobody realized;
/// a debug build fails loudly rather than encode an arm register on x86.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "residual x5 on x86")]
fn a_residual_low_arm_register_is_refused_in_a_debug_build() {
    let mut instructions = vec![CodeInstruction::new("mov")
        .physical_field("dst", "x5")
        .physical_field("src", "x10")];
    realize_x86_residual(&mut instructions, X86Abi::SysV);
}
