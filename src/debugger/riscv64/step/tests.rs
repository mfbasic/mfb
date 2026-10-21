//! `next_addresses` over real riscv64 instructions (from the corpora of
//! `tools/oracles/riscv64-decode`), one row of plan-174-D's table each.

use super::*;

fn word(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn parcel(value: u16) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

/// The registers of a thread with `ra` 0x2001, `sp` 0x8000, `a1` 0, `a5`
/// 0x3000, `a7` 64 (`write`), and `ra`/`a7` as `set` changes them.
fn registers(set: &[(usize, u64)]) -> General {
    let mut general = [0u64; 32];
    general[1] = 0x2001;
    general[2] = 0x8000;
    general[15] = 0x3000;
    general[17] = 64;
    for (index, value) in set {
        general[*index] = *value;
    }
    general
}

/// `next_addresses` with no memory but a signal frame's saved `pc` at
/// 0x8130, and `code` placed at `code_at`.
fn next(code: &[u8], pc: u64, general: &General, program: &[(u64, Vec<u8>)]) -> Next {
    let mut read_u64 = |address: u64| match address {
        0x8130 => Ok(0x4444),
        other => Err(format!("unmapped {other:#x}")),
    };
    let mut read_code = |address: u64, length: usize| {
        program
            .iter()
            .find(|(at, _)| *at == address)
            .map(|(_, bytes)| {
                bytes
                    .iter()
                    .copied()
                    .chain([0x01, 0x00].repeat(2))
                    .take(length)
                    .collect()
            })
            .ok_or_else(|| format!("unmapped {address:#x}"))
    };
    next_addresses(code, pc, general, &mut read_u64, &mut read_code).expect("the next addresses")
}

#[test]
fn a_straight_instruction_of_either_size_goes_on_to_the_next() {
    let general = registers(&[]);
    // addi sp,sp,-208 (4 bytes); c.mv s1,a1 (2 bytes).
    assert_eq!(
        next(&word(0xf301_0113), 0x16b0, &general, &[]),
        Next::To(vec![0x16b4])
    );
    assert_eq!(
        next(&parcel(0x84ae), 0x100, &general, &[]),
        Next::To(vec![0x102])
    );
    // A word the decoder does not know (opcode custom-0) is 4 bytes too.
    assert_eq!(
        next(&word(0x0000_000b), 0x100, &general, &[]),
        Next::To(vec![0x104])
    );
}

#[test]
fn a_branch_goes_on_or_to_its_target() {
    let general = registers(&[]);
    // beq t4,t1,1060 at 0x1058; c.beqz a1,27606 at 0x275f4.
    assert_eq!(
        next(&word(0x006e_8463), 0x1058, &general, &[]),
        Next::To(vec![0x105c, 0x1060])
    );
    assert_eq!(
        next(&parcel(0xc989), 0x275f4, &general, &[]),
        Next::To(vec![0x275f6, 0x27606])
    );
}

#[test]
fn a_jump_goes_to_its_target_or_through_its_register() {
    let general = registers(&[]);
    // jal 27400 at 0x28842.
    assert_eq!(
        next(&word(0xbbff_e0ef), 0x28842, &general, &[]),
        Next::To(vec![0x27400])
    );
    // jalr -260(ra) at 0x195c, ra 0x10958 (the auipc before it).
    let called = registers(&[(1, 0x10958)]);
    assert_eq!(
        next(&word(0xefc0_80e7), 0x195c, &called, &[]),
        Next::To(vec![0x10854])
    );
    // ret, either size: to ra with bit 0 cleared.
    assert_eq!(
        next(&word(0x0000_8067), 0x100, &general, &[]),
        Next::To(vec![0x2000])
    );
    assert_eq!(
        next(&parcel(0x8082), 0x100, &general, &[]),
        Next::To(vec![0x2000])
    );
    // c.jr a5.
    assert_eq!(
        next(&parcel(0x8782), 0x100, &general, &[]),
        Next::To(vec![0x3000])
    );
}

#[test]
fn a_system_call_returns_after_itself_but_rt_sigreturn_to_the_saved_pc() {
    assert_eq!(
        next(&word(0x0000_0073), 0x100, &registers(&[]), &[]),
        Next::To(vec![0x104])
    );
    assert_eq!(
        next(&word(0x0000_0073), 0x100, &registers(&[(17, 139)]), &[]),
        Next::To(vec![0x4444])
    );
}

#[test]
fn a_breakpoint_instruction_is_not_stepped() {
    let general = registers(&[]);
    assert_eq!(
        next(&word(0x0010_0073), 0x100, &general, &[]),
        Next::Breakpoint
    );
    assert_eq!(
        next(&parcel(0x9002), 0x100, &general, &[]),
        Next::Breakpoint
    );
}

/// glibc 2.41's compare-and-swap at 0x28bb2: `lr.w.aq a5,(a2)`; `bnez
/// a5,28bbe`; `sc.w a3,a4,(a2)`; `bnez a3,28bb2`. The step runs through the
/// `sc` and stops after it, or where the `bnez` leaves the sequence.
#[test]
fn an_lr_runs_to_its_sc_or_to_a_branch_out() {
    let general = registers(&[]);
    let program = [
        (0x28bb6, parcel(0xe781)),
        (0x28bb8, word(0x18e6_26af)),
        (0x28bbc, parcel(0xfafd)),
    ];
    assert_eq!(
        next(&word(0x1406_27af), 0x28bb2, &general, &program),
        Next::To(vec![0x28bbc, 0x28bbe])
    );
}

#[test]
fn an_lr_with_no_sc_near_it_is_stepped_alone() {
    let general = registers(&[]);
    // Sixteen c.nops after the lr, and no sc.
    let program = (0..16)
        .map(|index| (0x1004 + 2 * index, parcel(0x0001)))
        .collect::<Vec<_>>();
    assert_eq!(
        next(&word(0x1406_27af), 0x1000, &general, &program),
        Next::To(vec![0x1004])
    );
}
