//! The shared prompt code driving a riscv64 program: register names, 2- and
//! 4-byte instructions, and locations, through the `Recorder` fake playing
//! riscv64.

use super::*;
use crate::debugger::riscv64::RISCV64;

/// A riscv64 `Recorder` with code.
fn riscv64() -> Recorder {
    let mut inferior = Recorder::new(Ending::Killed);
    inferior.arch = &RISCV64;
    inferior.code = true;
    inferior
}

#[test]
fn on_riscv64_list_reg_shows_the_role_registers() {
    let mut inferior = riscv64();
    let (out, err) = drive(&mut inferior, "list reg\nq\n", false);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
Register | Hex                | Decimal | Role
a0       | 0x000000000000002a | (42)    | MFB arg, MFB result, C arg, C result
a1       | 0x0000000000000000 | (0)     | MFB arg, MFB result, C arg, C result
a2       | 0x0000000000000000 | (0)     | MFB arg, MFB result, C arg
a3       | 0x0000000000000000 | (0)     | MFB arg, MFB result, C arg
a4       | 0x0000000000000000 | (0)     | MFB arg, C arg
a5       | 0x0000000000000000 | (0)     | MFB arg, C arg
a6       | 0x0000000000000000 | (0)     | MFB arg, C arg
a7       | 0x0000000000000000 | (0)     | MFB arg, C arg
s2       | 0x0000000000000000 | (0)     | Current Thread
s10      | 0x0000000000000000 | (0)     | Closure
s11      | 0x0000000000000000 | (0)     | Arena

{}
{PROMPT}Killed process 42.\n",
            list_reg_usage(&RISCV64)
        )
    );
    assert!(
        list_reg_usage(&RISCV64).contains("list reg a0, a1, sp"),
        "{}",
        list_reg_usage(&RISCV64)
    );
}

#[test]
fn on_riscv64_dis_lists_two_and_four_byte_instructions() {
    let mut inferior = riscv64();
    inferior.pc = 0x1014;
    // `main` (0x1010, 16 bytes): addi sp,sp,-208; sd ra,0(sp); c.mv s1,a1;
    // c.jr ra; a reserved compressed word (c.addi4spn with no immediate); then
    // the fake's own c.nop.
    let program = [
        0xf301_0113_u32.to_le_bytes().to_vec(),
        0x0011_3023_u32.to_le_bytes().to_vec(),
        vec![0xae, 0x84, 0x82, 0x80, 0x04, 0x00],
    ]
    .concat();
    inferior.program = Some((0x1010, program));
    let (out, err) = drive(&mut inferior, "dis main\nq\n", false);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
prog`main:
    0x0000000000001010 <+0>:  addi   sp,sp,-208
->  0x0000000000001014 <+4>:  sd     ra,0(sp)
    0x0000000000001018 <+8>:  mv     s1,a1
    0x000000000000101a <+10>: ret
    0x000000000000101c <+12>: .short 0x0004
    0x000000000000101e <+14>: nop
{PROMPT}Killed process 42.
"
        )
    );
}

/// `dis -<n>` counts back `n` instructions, 2 or 4 bytes each, decoding from
/// the function's start, and into the function before it when `n` reaches
/// past its start.
#[test]
fn on_riscv64_dis_counts_back_by_instructions() {
    let mut inferior = riscv64();
    inferior.pc = 0x101a;
    let program = [
        0xf301_0113_u32.to_le_bytes().to_vec(),
        0x0011_3023_u32.to_le_bytes().to_vec(),
        vec![0xae, 0x84, 0x82, 0x80],
    ]
    .concat();
    inferior.program = Some((0x1010, program));
    let (out, err) = drive(&mut inferior, "dis -3 4\ndis -5 2\ndis 2\nq\n", false);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "{PROMPT}\
prog`main:
    0x0000000000001010 <+0>:  addi   sp,sp,-208
    0x0000000000001014 <+4>:  sd     ra,0(sp)
    0x0000000000001018 <+8>:  mv     s1,a1
->  0x000000000000101a <+10>: ret
{PROMPT}\
prog`_mfb_rt_print:
    0x000000000000100c <+4>: nop
    0x000000000000100e <+6>: nop
{PROMPT}\
prog`main:
->  0x000000000000101a <+10>: ret
    0x000000000000101c <+12>: nop
{PROMPT}Killed process 42.
"
        )
    );
}

#[test]
fn on_riscv64_locations_take_abi_names() {
    let mut inferior = riscv64();
    inferior.words = vec![(0x8000, 0x4847_4645_4443_4241)];
    let (out, err) = drive(
        &mut inferior,
        "show mem sp+16 8\nshow mem a0 8\nshow mem x10 8\nj ra a0=5\nq\n",
        false,
    );
    assert_eq!(
        out,
        format!(
            "{PROMPT}Memory at 0x0000000000008000 (sp+0x10 of thread 0):
0x0000000000008000 41 42 43 44 45 46 47 48                         | A B C D E F G H
{PROMPT}{PROMPT}{PROMPT}Moved thread 0 to 0x0000000000000000; set a0 = 0x0000000000000005 (5).
{PROMPT}Killed process 42.
"
        )
    );
    assert_eq!(
        err,
        "error: unmapped 0x2a
error: no register or function is named 'x10'; type 'list reg' or 'list func' for them
"
    );
    assert_eq!(
        parse("j pc a0=1", &RISCV64).err(),
        None,
        "a0 is settable on riscv64"
    );
    assert_eq!(
        parse("j pc x0=1", &RISCV64),
        Err(
            "no register 'x0'; jump sets ra, sp, gp, tp, t0-t6, s0-s11, a0-a7 and fcsr".to_string()
        )
    );
    assert_eq!(
        parse("j pc fa0=1", &RISCV64),
        Err(
            "'fa0' is a floating-point register; jump sets ra, sp, gp, tp, t0-t6, s0-s11, \
             a0-a7 and fcsr"
                .to_string()
        )
    );
    assert_eq!(
        parse("dis a0", &RISCV64),
        Ok(Command::Disassemble(Disassembly::At(
            Location::parse("a0").unwrap(),
            None
        )))
    );
    assert_eq!(
        parse("dis x10", &RISCV64),
        Ok(Command::Disassemble(Disassembly::Named(
            "x10".to_string(),
            None
        )))
    );
}
