//! The x86-64 integer encoder's register-allocation edge cases: the sequences the
//! allocator forces when a result lands on rcx (the shift count register), when a
//! shifted value aliases rcx, and the zero-store and carry forms that skip their
//! REX prefix or their move. Every expected sequence is hand-assembled from the
//! x86-64 reference, and `bytes` asserts the size predictor agrees.

use super::*;

/// A 32-bit variable rotate whose result lands on rcx (Win64's result bank starts
/// there): rcx cannot be both the count and the result, so the rotate runs in a
/// scratch register distinct from rcx, the value and the count, which is saved
/// and restored around it.
///
/// `ror rdx,cl` of rax by rbx: push rdx ; mov edx,eax ; mov ecx,ebx ; ror edx,cl ;
/// mov ecx,edx ; pop rdx.
#[test]
fn a_32_bit_rotate_into_rcx_runs_in_a_scratch_and_moves_the_result() {
    assert_eq!(
        bytes("rorv_w", &[("dst", "rcx"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x52, 0x89, 0xC2, 0x89, 0xD9, 0xD3, 0xCA, 0x89, 0xD1, 0x5A]
    );
    // The scratch is the first register that is none of rcx, the value, the count.
    assert_eq!(
        bytes("rorv_w", &[("dst", "rcx"), ("lhs", "rdx"), ("rhs", "rax")]),
        [0x53, 0x89, 0xD3, 0x89, 0xC1, 0xD3, 0xCB, 0x89, 0xD9, 0x5B]
    );
}

/// A 32-bit variable rotate of a value that lives in rcx, into an extended
/// register: the count staging overwrites rcx, so the value is reloaded from the
/// copy `push rcx` saved, with REX.B for the extended destination.
///
/// push rcx ; mov ecx,ebx ; mov r9d,[rsp] ; ror r9d,cl ; pop rcx.
#[test]
fn a_32_bit_rotate_of_rcx_into_an_extended_register_reloads_the_saved_copy() {
    assert_eq!(
        bytes("rorv_w", &[("dst", "r9"), ("lhs", "rcx"), ("rhs", "rbx")]),
        [0x51, 0x89, 0xD9, 0x41, 0x8B, 0x0C, 0x24, 0x41, 0xD3, 0xC9, 0x59]
    );
}

/// A zero store through a low base register needs no REX prefix; it stores an
/// immediate zero of the access width (`mov r/m32, 0` and `mov r/m8, 0`).
#[test]
fn zero_stores_through_a_low_base_carry_no_rex_prefix() {
    assert_eq!(
        bytes(
            "str_u32",
            &[("src", "xzr"), ("base", "rax"), ("offset", "8")]
        ),
        [0xC7, 0x80, 8, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        bytes(
            "str_u8",
            &[("src", "xzr"), ("base", "rax"), ("offset", "8")]
        ),
        [0xC6, 0x80, 8, 0, 0, 0, 0]
    );
}

/// `add_carry` into a low carry-out register needs no REX on the `setc`/`movzx`
/// pair, and with a zero addend, no carry-in and `dst == lhs` it is only the carry
/// materialization: there is nothing to add and nothing to move.
#[test]
fn add_carry_into_a_low_carry_out_register_has_no_rex_and_adds_nothing_for_a_zero_addend() {
    // add rbx,rdi ; setb al ; movzx rax,al
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "rax"),
                ("lhs", "rbx"),
                ("rhs", "rdi"),
                ("carry_in", "xzr"),
            ]
        ),
        [0x48, 0x01, 0xFB, 0x0F, 0x92, 0xC0, 0x48, 0x0F, 0xB6, 0xC0]
    );
    // setb al ; movzx rax,al
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "rax"),
                ("lhs", "rbx"),
                ("rhs", "xzr"),
                ("carry_in", "xzr"),
            ]
        ),
        [0x0F, 0x92, 0xC0, 0x48, 0x0F, 0xB6, 0xC0]
    );
}

/// `sub_borrow` with a zero subtrahend, no borrow-in and `dst == lhs` subtracts and
/// moves nothing: only the borrow-out (always 0 here, from `setb`) is materialized.
#[test]
fn sub_borrow_of_a_zero_subtrahend_into_its_own_lhs_only_materializes_the_borrow() {
    assert_eq!(
        bytes(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "rax"),
                ("lhs", "rbx"),
                ("rhs", "xzr"),
                ("borrow_in", "xzr"),
            ]
        ),
        [0x0F, 0x92, 0xC0, 0x48, 0x0F, 0xB6, 0xC0]
    );
}

/// plan-171-A: `stp`/`ldp` are AArch64-only. x86-64's `supports_pair_access` is
/// `false`, so nothing builds one for this target; if one ever arrived, the
/// encoder names the form instead of guessing a byte sequence.
#[test]
fn the_aarch64_pair_forms_are_refused() {
    for (op, first, second) in [("stp", "src", "src2"), ("ldp", "dst", "dst2")] {
        let ins = CodeInstruction::new(op)
            .physical_field(first, "r12")
            .physical_field(second, "r14")
            .physical_field("base", "rsp")
            .field("offset", "16")
            .field("addressing", "offset")
            .field("width", "8");
        let err = enc_err(&ins);
        assert!(err.contains("AArch64-only"), "{op}: {err}");
    }
}
