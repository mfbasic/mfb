//! AArch64 as the debugger reads it: the descriptor the shared code asks
//! ([`AARCH64`]), and the register file the macOS and Linux backends read and
//! write ([`registers`]).

#[cfg(any(
    target_os = "macos",
    all(target_os = "linux", target_arch = "aarch64"),
    test
))]
mod registers;
#[cfg(any(
    target_os = "macos",
    all(target_os = "linux", target_arch = "aarch64"),
    test
))]
pub(crate) use registers::*;

use super::arch::{Arch, ArchHelp, Decoded, Effect, ElfArch, Reg};
use crate::arch::aarch64::decode::{self as a64, Flow};

/// AArch64, as the shared debugger code reads it.
pub(crate) const AARCH64: Arch = Arch {
    name: "aarch64",
    pc: "pc",
    sp: "sp",
    link: "lr",
    frame: "fp",
    arguments: ["x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7"],
    is_register,
    settable,
    holds,
    alignment: 4,
    longest: 4,
    decode,
    length,
    effect,
    address_mask: ADDRESS_MASK,
    watch_span: Some(8),
    help: ArchHelp {
        jump: "(x0-x28, fp, lr, sp,
                  nzcv, fpsr, fpcr; a value is a number or a <where>); the
                  thread stays paused",
        watch: "[<commands>], <where> as show mem; <size> is 1 to 8 bytes
                  (default 8) that do not cross a multiple of 8
                  Remove one: wp r <where>",
        groups: "  gpr           The general-purpose registers: x0 to x28, fp (x29), lr (x30)
  simd          The SIMD and floating-point registers: v0 to v31
  sys           The system registers: sp, pc, nzcv, fpsr, fpcr",
    },
    elf: ELF,
};

/// The bits of the processor state the `nzcv` register shows: the N, Z, C and
/// V flags.
pub(crate) const NZCV_BITS: u64 = 0xf000_0000;

/// The bits of a user-space address: 47 on macOS, 48 on Linux. Return
/// addresses carry no pointer-authentication bits in an arm64 process; masking
/// strips any that a system library's code might have left, which sit above
/// these.
const ADDRESS_MASK: u64 = if cfg!(target_os = "linux") {
    (1 << 48) - 1
} else {
    (1 << 47) - 1
};

/// Whether `word` names an AArch64 register an address can be in: `x0`–`x30`,
/// `fp`, `lr`, `sp` or `pc`.
fn is_register(word: &str) -> bool {
    matches!(word, "fp" | "lr" | "sp" | "pc")
        || word.strip_prefix('x').is_some_and(|number| {
            (number == "0" || !number.starts_with('0'))
                && number.parse::<u8>().is_ok_and(|number| number <= 30)
        })
}

/// Refuse a register `jump` does not set: it sets `x0`–`x28`, `fp`, `lr`,
/// `sp`, `nzcv`, `fpsr` and `fpcr`, and moves the `pc` itself.
fn settable(name: &str) -> Result<(), String> {
    let general = name.strip_prefix('x').is_some_and(|number| {
        (number == "0" || !number.starts_with('0'))
            && number.parse::<u8>().is_ok_and(|number| number <= 28)
    });
    if general || matches!(name, "fp" | "lr" | "sp" | "nzcv" | "fpsr" | "fpcr") {
        return Ok(());
    }
    let sets = "jump sets x0-x28, fp, lr, sp, nzcv, fpsr and fpcr";
    if name == "pc" {
        return Err(format!(
            "'pc' is where the thread goes: give it as the place, jump <where>; {sets}"
        ));
    }
    if name
        .strip_prefix('v')
        .is_some_and(|number| number.parse::<u8>().is_ok_and(|number| number <= 31))
    {
        return Err(format!("'{name}' is a vector register; {sets}"));
    }
    Err(format!("no register '{name}'; {sets}"))
}

/// Refuse a value register `name` cannot hold: `nzcv` holds the four flags,
/// `fpsr` and `fpcr` 32 bits.
fn holds(name: &str, value: u64) -> Result<(), String> {
    match name {
        "nzcv" if value & !NZCV_BITS != 0 => Err(format!(
            "cannot set nzcv to 0x{value:016x}: it holds only the flags, bits 28 to 31 \
             (0x00000000f0000000)"
        )),
        "fpsr" | "fpcr" if value > u64::from(u32::MAX) => Err(format!(
            "cannot set {name} to 0x{value:016x}: it holds 32 bits"
        )),
        _ => Ok(()),
    }
}

/// The 32-bit little-endian word at the start of `bytes`.
fn word(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

/// The instruction at the start of `bytes`, by the decoder that follows lldb.
fn decode(bytes: &[u8], pc: u64) -> Option<Decoded> {
    let instruction = a64::decode(word(bytes)?, pc)?;
    Some(Decoded {
        length: 4,
        text: instruction.text,
        returns: instruction.flow == Flow::Return,
        target: instruction.target,
        value: instruction.value,
        page: instruction.page,
        // An unshifted 12-bit immediate: it fits.
        page_offset: instruction
            .page_offset
            .map(|(register, offset)| (register, offset as i64)),
    })
}

/// Every AArch64 instruction is 4 bytes.
fn length(_: &[u8]) -> usize {
    4
}

/// A register in a base (address) field, where 31 is `sp`.
fn base(number: u8) -> Reg {
    match number {
        31 => Reg::Sp,
        number => data(number),
    }
}

/// A register in a data field; 31 there is the zero register, which is none
/// of the frame's.
fn data(number: u8) -> Reg {
    match number {
        29 => Reg::Frame,
        30 => Reg::Link,
        number => Reg::Other(number),
    }
}

/// What the A64 instruction at the start of `bytes`, at `address`, does to its
/// function's frame: `x29` is the frame pointer, `x30` the link register.
fn effect(bytes: &[u8], address: u64) -> Effect {
    let Some(word) = word(bytes) else {
        return Effect::Other;
    };
    let rd = (word & 31) as u8;
    let rn = ((word >> 5) & 31) as u8;
    let rt2 = ((word >> 10) & 31) as u8;
    // ADD/SUB (immediate), 64-bit: sf=1, S=0.
    if word & 0x3f80_0000 == 0x1100_0000 && word >> 31 == 1 {
        let subtract = word & 0x4000_0000 != 0;
        let shift = if word & 0x0040_0000 != 0 { 12 } else { 0 };
        let immediate = i64::from((word >> 10) & 0xfff) << shift;
        let signed = if subtract { -immediate } else { immediate };
        return match (rd, rn) {
            (31, 31) => Effect::AdjustSp(signed),
            (31, 29) => Effect::SpFromFp { offset: signed },
            (31, _) => Effect::SpUnknown,
            (register, 31) => Effect::SpAddress {
                register: data(register),
                offset: signed,
            },
            _ => Effect::Other,
        };
    }
    // Any other write to `sp` (`mov sp, x9`, `sub sp, sp, x16`): ADD/SUB
    // (extended register) with Rd = 31.
    if word & 0x1fe0_0000 == 0x0b20_0000 && rd == 31 {
        return Effect::SpUnknown;
    }
    // LDP/STP, 64-bit integer or SIMD: opc and V select the size.
    let pair = word & 0x3a00_0000 == 0x2800_0000;
    if pair {
        let opc = word >> 30;
        let simd = word & 0x0400_0000 != 0;
        let scale = match (simd, opc) {
            (false, 2) => 8,
            (true, 1) => 16,
            (true, 2) => 32,
            _ => 0,
        };
        if scale != 0 {
            let load = word & 0x0040_0000 != 0;
            let mode = (word >> 23) & 3;
            let offset = sign_extend((word >> 15) & 0x7f, 7) * scale;
            // SIMD registers are never `fp` or `lr`.
            let registers = if simd {
                Vec::new()
            } else {
                vec![data(rd), data(rt2)]
            };
            return pair_effect(load, mode, base(rn), registers, offset, scale);
        }
    }
    // STR/LDR (immediate, unsigned offset), 64-bit integer.
    if word & 0xffc0_0000 == 0xf900_0000 {
        let offset = i64::from((word >> 10) & 0xfff) * 8;
        return Effect::Store {
            base: base(rn),
            pairs: vec![(data(rd), offset)],
        };
    }
    if word & 0xffc0_0000 == 0xf940_0000 {
        return Effect::Load {
            registers: vec![data(rd)],
            sp_delta: None,
        };
    }
    // STR/LDR (immediate, pre- or post-index), 64-bit integer.
    if word & 0xffe0_0400 == 0xf800_0400 {
        let pre = word & 0x800 != 0;
        let delta = sign_extend((word >> 12) & 0x1ff, 9);
        return if pre {
            Effect::StoreWriteback {
                base: base(rn),
                pairs: vec![(data(rd), 0)],
                delta,
            }
        } else {
            Effect::StoreThenMove {
                base: base(rn),
                pairs: vec![(data(rd), 0)],
                delta,
            }
        };
    }
    if word & 0xffe0_0400 == 0xf840_0400 {
        return Effect::Load {
            registers: vec![data(rd)],
            sp_delta: (rn == 31).then(|| sign_extend((word >> 12) & 0x1ff, 9)),
        };
    }
    // STR (immediate, unsigned offset) of a SIMD register to `sp`: a
    // callee-saved `d8`–`d15` save, part of a prologue.
    if word & 0x3f40_0000 == 0x3d00_0000 && rn == 31 {
        return Effect::Store {
            base: Reg::Sp,
            pairs: Vec::new(),
        };
    }
    // HINT: `nop`, `pacibsp`, `bti`, ….
    if word & 0xffff_f01f == 0xd503_201f {
        return Effect::Hint;
    }
    // RET, RETAA, RETAB.
    if word & 0xffff_fc1f == 0xd65f_0000 || word == 0xd65f_0bff || word == 0xd65f_0fff {
        return Effect::Leave { target: None };
    }
    // BR (and its authenticated forms).
    if word & 0xfe9f_f000 == 0xd61f_0000 {
        return Effect::Leave { target: None };
    }
    // B.
    if word & 0xfc00_0000 == 0x1400_0000 {
        let offset = sign_extend(word & 0x03ff_ffff, 26) * 4;
        return Effect::Leave {
            target: Some(address.wrapping_add_signed(offset)),
        };
    }
    Effect::Other
}

/// The effect of an LDP/STP: `mode` 1 is post-index, 2 signed offset, 3
/// pre-index.
fn pair_effect(
    load: bool,
    mode: u32,
    base: Reg,
    registers: Vec<Reg>,
    offset: i64,
    scale: i64,
) -> Effect {
    if load {
        let sp_delta = (base == Reg::Sp && mode != 2).then_some(offset);
        // A pre-index load from `sp` moves it first; every load here restores
        // registers, so only where `sp` ends up matters.
        return Effect::Load {
            registers,
            sp_delta,
        };
    }
    let pairs = registers.into_iter().zip([0, scale]).collect::<Vec<_>>();
    match mode {
        3 => Effect::StoreWriteback {
            base,
            pairs,
            delta: offset,
        },
        1 => Effect::StoreThenMove {
            base,
            pairs,
            delta: offset,
        },
        _ => Effect::Store {
            base,
            pairs: pairs
                .into_iter()
                .map(|(register, at)| (register, at + offset))
                .collect(),
        },
    }
}

fn sign_extend(value: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    (i64::from(value) << shift) >> shift
}

/// The compiler's AArch64 ELF: `R_AARCH64_JUMP_SLOT` and `R_AARCH64_GLOB_DAT`
/// bind its GOT slots, and an import stub is `adrp x16` / `ldr x17, [x16]` /
/// `br x17`.
pub(crate) const ELF: ElfArch = ElfArch {
    machine: 183,
    jump_slot: 1026,
    data_slot: 1025,
    stub_size: 12,
    stub_slot,
};

/// The GOT slot the import stub at `start` loads: `adrp x16, page`; `ldr x17,
/// [x16, #imm12*8]`; `br x17` (`crate::os::linux::link::emit_import_stub`).
fn stub_slot(code: &[u8], start: u64) -> Option<u64> {
    let (adrp, ldr, br) = (word(code)?, word(code.get(4..)?)?, word(code.get(8..)?)?);
    if adrp & 0x9f00_001f != 0x9000_0010 || ldr & 0xffc0_03ff != 0xf940_0211 || br != 0xd61f_0220 {
        return None;
    }
    let immlo = u64::from((adrp >> 29) & 0x3);
    let immhi = u64::from((adrp >> 5) & 0x7_ffff);
    let page = (((immhi << 2) | immlo) << 12) as i64;
    let page = ((page << 31) >> 31) as u64; // sign-extend the 33-bit offset
    Some(
        (start & !0xfff)
            .wrapping_add(page)
            .wrapping_add(u64::from((ldr >> 10) & 0xfff) * 8),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every register the descriptor names is one `list reg` shows, a word
    /// that names a register, and (but for `pc`) one `jump` sets.
    #[test]
    fn the_descriptor_names_the_registers_the_backends_read() {
        let shown = registers(&[0; 34], &Float::zeroed())
            .into_iter()
            .map(|register| register.name)
            .collect::<Vec<_>>();
        let named = [AARCH64.pc, AARCH64.sp, AARCH64.link, AARCH64.frame]
            .into_iter()
            .chain(AARCH64.arguments);
        for name in named {
            assert!(
                shown.iter().any(|shown| shown == name),
                "{name} is not listed"
            );
            assert!((AARCH64.is_register)(name), "{name} is not a register");
            assert_eq!(
                (AARCH64.settable)(name).is_ok(),
                name != AARCH64.pc,
                "{name}"
            );
        }
    }
}
