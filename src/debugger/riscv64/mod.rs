//! riscv64 as the debugger reads it: the descriptor the shared code asks
//! ([`RISCV64`]), and the register file the Linux backend reads and writes
//! ([`registers`]).
//!
//! Registers are named by their ABI names only (`a0`, `ra`, `s0`, `fa0`), as the
//! compiler and `objdump` name them; `x10` and `fp` name nothing.

mod registers;
pub(crate) mod step;
pub(crate) use registers::*;

use super::arch::{Arch, ArchHelp, Decoded, Effect, ElfArch, Reg};
use crate::arch::riscv64::decode::{self, Flow, F, X};

/// riscv64, as the shared debugger code reads it.
pub(crate) const RISCV64: Arch = Arch {
    name: "riscv64",
    pc: "pc",
    sp: "sp",
    link: "ra",
    frame: "s0",
    arguments: ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"],
    is_register,
    settable,
    holds,
    alignment: 2,
    longest: 4,
    decode: decoded,
    length: decode::length,
    effect,
    // No pointer authentication: a return address is a plain address.
    address_mask: u64::MAX,
    // Neither kernel the boxes run provides hardware watchpoints.
    watch_span: None,
    help: ArchHelp {
        jump: "(ra, sp, gp, tp,
                  t0-t6, s0-s11, a0-a7, fcsr; a value is a number or a
                  <where>); the thread stays paused",
        watch: "[<commands>]: not on riscv64 Linux, which has no
                  watchpoints; wp says so and sets none",
        groups: "  gpr           The general-purpose registers: ra, gp, tp, t0 to t6, s0 to
                s11, a0 to a7
  simd          The floating-point registers: ft0 to ft11, fs0 to fs11,
                fa0 to fa7
  sys           The system registers: sp, pc, fcsr",
    },
    elf: ELF,
};

/// Whether `word` names a riscv64 register an address can be in: `pc`, or an
/// integer register by its ABI name (`zero` aside).
fn is_register(word: &str) -> bool {
    word == "pc" || X[1..].contains(&word)
}

/// What `jump` sets, as its refusals say.
const JUMP_SETS: &str = "jump sets ra, sp, gp, tp, t0-t6, s0-s11, a0-a7 and fcsr";

/// Refuse a register `jump` does not set: it sets every integer register but
/// `zero`, and `fcsr`, and moves the `pc` itself.
fn settable(name: &str) -> Result<(), String> {
    if name == "fcsr" || X[1..].contains(&name) {
        return Ok(());
    }
    if name == "pc" {
        return Err(format!(
            "'pc' is where the thread goes: give it as the place, jump <where>; {JUMP_SETS}"
        ));
    }
    if name == "zero" {
        return Err(format!("'zero' is always 0; {JUMP_SETS}"));
    }
    if F.contains(&name) {
        return Err(format!(
            "'{name}' is a floating-point register; {JUMP_SETS}"
        ));
    }
    Err(format!("no register '{name}'; {JUMP_SETS}"))
}

/// Refuse a value register `name` cannot hold: `fcsr` holds 32 bits.
fn holds(name: &str, value: u64) -> Result<(), String> {
    if name == "fcsr" && value > u64::from(u32::MAX) {
        return Err(format!(
            "cannot set fcsr to 0x{value:016x}: it holds 32 bits"
        ));
    }
    Ok(())
}

/// The instruction at the start of `bytes`, as `objdump` spells it, with the
/// mnemonic padded to the column AArch64 listings use.
fn decoded(bytes: &[u8], pc: u64) -> Option<Decoded> {
    let instruction = decode::decode(bytes, pc)?;
    let text = match instruction.text.split_once('\t') {
        Some((mnemonic, operands)) => format!("{mnemonic:<6} {operands}"),
        None => instruction.text,
    };
    Some(Decoded {
        length: instruction.length,
        text,
        returns: instruction.flow == Flow::Return,
        target: instruction.target,
        value: instruction.value.map(i128::from),
        page: instruction
            .page
            .map(|(register, address)| (u32::from(register), address)),
        page_offset: instruction
            .page_offset
            .map(|(register, offset)| (u32::from(register), offset)),
    })
}

/// An integer register by the part it plays in a frame: `sp` (`x2`), the link
/// register `ra` (`x1`), the frame pointer `s0` (`x8`), or another.
fn role(number: u8) -> Reg {
    match number {
        1 => Reg::Link,
        2 => Reg::Sp,
        8 => Reg::Frame,
        number => Reg::Other(number),
    }
}

/// What the instruction at the start of `bytes`, at `address`, does to its
/// function's frame. A compressed instruction reads as the base one it expands
/// to, so `c.addi16sp` is an `sp` adjustment and `c.sdsp ra` a save.
fn effect(bytes: &[u8], address: u64) -> Effect {
    let Some(instruction) = decode::decode(bytes, address) else {
        return Effect::Other;
    };
    let word = instruction.word;
    let (rd, rs1, rs2) = (decode::rd(word), decode::rs1(word), decode::rs2(word));
    match word & 0x7f {
        // addi: `nop` and the other writes to `zero` are hints.
        0x13 if decode::funct3(word) == 0 => {
            let offset = decode::i_imm(word);
            match (rd, rs1) {
                (0, _) => Effect::Hint,
                (2, 2) => Effect::AdjustSp(offset),
                (2, 8) => Effect::SpFromFp { offset },
                (2, _) => Effect::SpUnknown,
                (register, 2) => Effect::SpAddress {
                    register: role(register),
                    offset,
                },
                (register, from) => constant_add(register, from, offset),
            }
        }
        // `sub sp,sp,t0` and `add sp,sp,t0`: a frame too big for `addi`.
        0x33 if rd == 2
            && rs1 == 2
            && decode::funct3(word) == 0
            && matches!(decode::funct7(word), 0 | 0x20) =>
        {
            Effect::SpByRegister {
                register: role(rs2),
                subtract: decode::funct7(word) == 0x20,
            }
        }
        // Any other write to `sp` (`andi sp,sp,-16`, `ld sp,…`).
        0x03 | 0x13 | 0x17 | 0x1b | 0x33 | 0x37 | 0x3b if rd == 2 => Effect::SpUnknown,
        // The constants a large frame's size is built from: `lui`, then `addi`
        // or `addiw` on the same register.
        0x37 if rd != 0 => Effect::Constant {
            register: role(rd),
            value: Some(decode::u_imm(word)),
        },
        0x1b if rd != 0 && decode::funct3(word) == 0 => constant_add(rd, rs1, decode::i_imm(word)),
        // Any other write to an integer register leaves it unknown.
        0x13 | 0x17 | 0x1b | 0x33 | 0x3b if rd != 0 => Effect::Constant {
            register: role(rd),
            value: None,
        },
        // A doubleword store saves a register; the narrower ones, and a
        // floating-point store (`fsd fs0,8(sp)`, a callee-saved save), save none
        // the walk follows.
        0x23 if decode::funct3(word) == 3 => Effect::Store {
            base: role(rs1),
            pairs: vec![(role(rs2), decode::s_imm(word))],
        },
        0x23 | 0x27 => Effect::Store {
            base: role(rs1),
            pairs: Vec::new(),
        },
        0x03 => Effect::Load {
            registers: vec![role(rd)],
            sp_delta: None,
        },
        // `ret` and `jr`: control leaves; a `jalr` that links is a call.
        0x67 => match instruction.flow {
            Flow::Indirect { link: true, .. } => Effect::Other,
            _ => Effect::Leave { target: None },
        },
        // `j`: control goes to its target; a `jal` that links is a call.
        0x6f if rd == 0 => Effect::Leave {
            target: instruction.target,
        },
        _ => Effect::Other,
    }
}

/// `register = from + offset`: a constant outright when `from` is `zero`
/// (`li`).
fn constant_add(register: u8, from: u8, offset: i64) -> Effect {
    if from == 0 {
        Effect::Constant {
            register: role(register),
            value: Some(offset),
        }
    } else {
        Effect::ConstantAdd {
            register: role(register),
            from: role(from),
            offset,
        }
    }
}

/// The compiler's riscv64 ELF: `R_RISCV_JUMP_SLOT` (5) and `R_RISCV_64` (2)
/// bind its GOT slots, and an import stub is `auipc t3` / `ld t3, lo(t3)` /
/// `jr t3`, 12 bytes (`crate::os::linux::link::emit_import_stub`).
const ELF: ElfArch = ElfArch {
    machine: 243,
    jump_slot: 5,
    data_slot: 2,
    stub_size: 12,
    stub_slot,
};

/// The GOT slot the import stub at `start` loads: `auipc t3, hi`; `ld t3,
/// lo(t3)`; `jalr zero, 0(t3)`.
fn stub_slot(code: &[u8], start: u64) -> Option<u64> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(code.get(at..at + 4)?.try_into().ok()?))
    };
    let (auipc, ld, jr) = (word(0)?, word(4)?, word(8)?);
    // auipc t3; ld t3, ?(t3) (funct3 3, rs1 = rd = x28); jalr x0, 0(t3).
    if auipc & 0xfff != 0xe17 || ld & 0xf_ffff != 0xe_3e03 || jr != 0x000e_0067 {
        return None;
    }
    Some(
        start
            .wrapping_add_signed(decode::u_imm(auipc))
            .wrapping_add_signed(decode::i_imm(ld)),
    )
}
