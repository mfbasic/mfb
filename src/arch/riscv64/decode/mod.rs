//! A riscv64 instruction decoder for `mfb dbg`: what an instruction is, where
//! control goes after it, and the text GNU `objdump` prints for it.
//!
//! The text is `objdump -d`'s with its default aliases (`li`, `mv`, `ret`, `j`,
//! `beqz`, `sext.w`, `frflags`, …) and ABI register names: the mnemonic, one
//! tab, then the operands, with the `# …` comment and `<symbol>` annotations
//! left out. A compressed (`C`) instruction prints as the base instruction it
//! expands to, as `objdump` prints it. `tools/oracles/riscv64-decode` holds the
//! oracle: `objdump_words.txt` is checked on every test run.
//!
//! The decoder knows RV64I, `M`, `A`, `F`, `D`, `C`, `Zicsr`, `Zifencei` and the
//! vector forms the compiler emits; anything else answers `None`, though
//! [`length`] still gives its size so a listing can step over it.

mod atomic;
mod compressed;
mod float;
mod integer;
mod vector;

#[cfg(test)]
mod round_trip;
#[cfg(test)]
mod tests;

/// Where control goes after an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Falls through to the next instruction.
    Next,
    /// A conditional branch: to `target`, or the next instruction.
    Branch,
    /// An unconditional jump to `target` that links nothing (`j`).
    Jump,
    /// A jump to `target` that leaves the return address in a register (`jal`).
    Call,
    /// A jump to `(register + offset) & !1`; `link` when it leaves the return
    /// address in a register (`jalr`), not when it does not (`jr`).
    Indirect {
        register: u8,
        offset: i64,
        link: bool,
    },
    /// `ret`: `jalr zero, 0(ra)`.
    Return,
    /// `ecall` or `ebreak`.
    Trap,
    /// `lr.w` or `lr.d`: the start of a load-reserved / store-conditional
    /// sequence, which must run without interruption to succeed.
    AtomicStart,
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Instruction {
    /// Its size: 2 or 4 bytes.
    pub(crate) length: usize,
    /// The base instruction, as a 32-bit word: the instruction itself, or the
    /// one a compressed instruction expands to. [`rd`], [`rs1`] and the other
    /// field readers take it.
    pub(crate) word: u32,
    /// `mnemonic\toperands`, as `objdump` prints it.
    pub(crate) text: String,
    pub(crate) flow: Flow,
    /// The address a branch or a direct jump goes to.
    pub(crate) target: Option<u64>,
    /// The value `li` sets.
    pub(crate) value: Option<i64>,
    /// `auipc`: the register it sets and the address it sets it to.
    pub(crate) page: Option<(u8, u64)>,
    /// An `addi`, load, store or `jalr` through a register: the register and
    /// the offset, so a listing can follow an `auipc` into the address the pair
    /// forms.
    pub(crate) page_offset: Option<(u8, i64)>,
}

/// The integer registers by number, as `objdump` names them.
pub(crate) const X: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

/// The floating-point registers by number, as `objdump` names them.
pub(crate) const F: [&str; 32] = [
    "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
    "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
    "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
];

/// The size of the instruction whose first bytes are `bytes`, from the low
/// bits of its first 16-bit parcel: 2 for a compressed instruction, 4 for a
/// base one, and the longer encodings' sizes. 2, the smallest step, when
/// `bytes` is too short to tell or the encoding is reserved.
pub(crate) fn length(bytes: &[u8]) -> usize {
    let Some(&low) = bytes.first() else {
        return 2;
    };
    if low & 0b11 != 0b11 {
        return 2;
    }
    if low & 0b1_1100 != 0b1_1100 {
        return 4;
    }
    if low & 0b11_1111 == 0b01_1111 {
        return 6;
    }
    if low & 0b111_1111 == 0b011_1111 {
        return 8;
    }
    // 80 bits and up: bits 12-14 of the parcel give the size, 7 is reserved.
    match bytes.get(1).map(|high| (high >> 4) & 0b111) {
        Some(size) if size != 0b111 => 10 + 2 * usize::from(size),
        _ => 2,
    }
}

/// The instruction at the start of `bytes`, at address `pc`, or `None` when the
/// decoder does not know it or `bytes` holds less than all of it.
pub(crate) fn decode(bytes: &[u8], pc: u64) -> Option<Instruction> {
    match length(bytes) {
        2 => {
            let parcel = u16::from_le_bytes(bytes.get(..2)?.try_into().ok()?);
            compressed::decode(parcel, pc)
        }
        4 => {
            let word = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
            base(word, pc, 4)
        }
        _ => None,
    }
}

/// The base instruction `word` at `pc`, `length` bytes long in the code (2
/// for a compressed instruction it stands for).
fn base(word: u32, pc: u64, length: usize) -> Option<Instruction> {
    let mut instruction = match word & 0x7f {
        0x07 | 0x27 => float::load_store(word).or_else(|| vector::load_store(word))?,
        0x2f => atomic::decode(word)?,
        0x43 | 0x47 | 0x4b | 0x4f => float::fused(word)?,
        0x53 => float::op_fp(word)?,
        0x57 => vector::op_v(word)?,
        _ => integer::decode(word, pc)?,
    };
    instruction.length = length;
    Some(instruction)
}

/// An instruction of the given text that falls through; [`base`] sets its
/// length.
fn instruction(word: u32, mnemonic: &str, operands: String) -> Instruction {
    Instruction {
        length: 4,
        word,
        text: if operands.is_empty() {
            mnemonic.to_string()
        } else {
            format!("{mnemonic}\t{operands}")
        },
        flow: Flow::Next,
        target: None,
        value: None,
        page: None,
        page_offset: None,
    }
}

/// `rd`, bits 7-11.
pub(crate) fn rd(word: u32) -> u8 {
    ((word >> 7) & 31) as u8
}

/// `rs1`, bits 15-19.
pub(crate) fn rs1(word: u32) -> u8 {
    ((word >> 15) & 31) as u8
}

/// `rs2`, bits 20-24.
pub(crate) fn rs2(word: u32) -> u8 {
    ((word >> 20) & 31) as u8
}

/// `funct3`, bits 12-14.
pub(crate) fn funct3(word: u32) -> u32 {
    (word >> 12) & 7
}

/// `funct7`, bits 25-31.
pub(crate) fn funct7(word: u32) -> u32 {
    word >> 25
}

/// The I-type immediate: bits 20-31, sign-extended.
pub(crate) fn i_imm(word: u32) -> i64 {
    i64::from((word as i32) >> 20)
}

/// The S-type immediate: bits 25-31 and 7-11, sign-extended.
pub(crate) fn s_imm(word: u32) -> i64 {
    i64::from(((word as i32) >> 25) << 5 | ((word >> 7) & 31) as i32)
}

/// The B-type immediate: a branch's offset, sign-extended.
pub(crate) fn b_imm(word: u32) -> i64 {
    let value = ((word >> 31) & 1) << 12
        | ((word >> 7) & 1) << 11
        | ((word >> 25) & 0x3f) << 5
        | ((word >> 8) & 0xf) << 1;
    sign_extend(value, 13)
}

/// The U-type immediate: bits 12-31 in place, sign-extended.
pub(crate) fn u_imm(word: u32) -> i64 {
    i64::from((word & 0xffff_f000) as i32)
}

/// The J-type immediate: a `jal`'s offset, sign-extended.
pub(crate) fn j_imm(word: u32) -> i64 {
    let value = ((word >> 31) & 1) << 20
        | ((word >> 12) & 0xff) << 12
        | ((word >> 20) & 1) << 11
        | ((word >> 21) & 0x3ff) << 1;
    sign_extend(value, 21)
}

/// The low `bits` bits of `value`, sign-extended.
pub(crate) fn sign_extend(value: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    (i64::from(value) << shift) >> shift
}

/// An address as `objdump` prints a branch's or jump's target: hex, no `0x`.
fn address(value: u64) -> String {
    format!("{value:x}")
}
