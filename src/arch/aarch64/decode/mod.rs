//! AArch64 instruction decoder for `mfb dbg`'s listings.
//!
//! It knows every form the encoder emits — `tests.rs` holds the two sides
//! together by encoding each form and decoding it back — and, beyond them, the
//! A64 base instruction set, scalar floating point, Advanced SIMD, the AES and
//! SHA-1/SHA-256 instructions, LSE atomics and pointer authentication, so code
//! in dyld and the system libraries a program stops in reads as instructions
//! too. SVE, SME and memory tagging are not decoded; a word it does not know
//! decodes to `None`.
//!
//! Text follows the LLVM disassembler that lldb uses on macOS: the mnemonic
//! padded to six columns, aliases where LLVM prints one (`mov`, `cmp`, `neg`,
//! `cset`, `lsl`, `tst`…), immediates in hex, register 31 as `sp` or `xzr` by
//! what the form means, and NEON in Apple's syntax — the arrangement on the
//! mnemonic (`fadd.2d v1, v0, v2`), not on each register.

mod control;
mod float;
mod integer;
mod memory;
mod vector;

#[cfg(test)]
mod tests;

/// What an instruction does to the flow of control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Falls through to the next instruction.
    Next,
    /// `ret`, `retaa`, `retab`: leaves the function.
    Return,
    /// A jump: `b`, `b.<cond>`, `cbz`, `tbz` to `target`, or `br` to a
    /// register.
    Branch,
    /// `bl` to `target`, or `blr` (and its authenticating forms).
    Call,
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Instruction {
    /// `mnemonic operands`, the mnemonic padded to six columns.
    pub(crate) text: String,
    pub(crate) flow: Flow,
    /// The address a branch, call, `adr` or literal load names.
    pub(crate) target: Option<u64>,
    /// The value a `mov` of an immediate sets, for a comment
    /// (`mov x8, #0xde83`, `mov w0, #-0x2`).
    pub(crate) value: Option<i128>,
    /// `adrp`: the register it sets and the page address it sets it to.
    pub(crate) page: Option<(u32, u64)>,
    /// `add Xd, Xn, #imm` (unshifted): `Xn` and the immediate, so a listing can
    /// follow an `adrp` into the address the pair forms.
    pub(crate) page_offset: Option<(u32, u64)>,
}

fn instruction(mnemonic: &str, operands: String) -> Instruction {
    Instruction {
        text: if operands.is_empty() {
            mnemonic.to_string()
        } else {
            format!("{mnemonic:<6} {operands}")
        },
        flow: Flow::Next,
        target: None,
        value: None,
        page: None,
        page_offset: None,
    }
}

/// A branch or call (or, with [`Flow::Next`], an address-forming instruction)
/// to `target`, printed as an absolute address after `operands`.
fn to_target(mnemonic: &str, operands: &str, target: u64, flow: Flow) -> Instruction {
    let mut jump = instruction(mnemonic, format!("{operands}{target:#x}"));
    jump.flow = flow;
    jump.target = Some(target);
    jump
}

/// `bits` bits of `word` from bit `low` up.
fn field(word: u32, low: u32, bits: u32) -> u32 {
    (word >> low) & ((1 << bits) - 1)
}

fn bit(word: u32, at: u32) -> bool {
    word & (1 << at) != 0
}

fn rd(word: u32) -> u32 {
    word & 0x1f
}

fn rn(word: u32) -> u32 {
    (word >> 5) & 0x1f
}

fn rm(word: u32) -> u32 {
    (word >> 16) & 0x1f
}

fn ra(word: u32) -> u32 {
    (word >> 10) & 0x1f
}

/// `x<n>`, with 31 as `xzr` (register forms) or `sp` (immediate, address forms).
fn x(number: u32, sp: bool) -> String {
    register(number, true, sp)
}

fn w(number: u32) -> String {
    register(number, false, false)
}

/// A general register: `x<n>` when `wide`, else `w<n>`; 31 is the stack pointer
/// (`sp`/`wsp`) when `sp`, else the zero register (`xzr`/`wzr`).
fn register(number: u32, wide: bool, sp: bool) -> String {
    match (number, wide, sp) {
        (31, true, true) => "sp".to_string(),
        (31, false, true) => "wsp".to_string(),
        (31, true, false) => "xzr".to_string(),
        (31, false, false) => "wzr".to_string(),
        (n, true, _) => format!("x{n}"),
        (n, false, _) => format!("w{n}"),
    }
}

fn hex(value: u64) -> String {
    format!("#{value:#x}")
}

/// A signed immediate in hex: `#0x10`, `#-0x10`.
fn signed_hex(value: i64) -> String {
    if value < 0 {
        format!("#-{:#x}", value.unsigned_abs())
    } else {
        format!("#{value:#x}")
    }
}

/// An immediate LLVM prints as `#0` when it is zero and in hex otherwise
/// (`svc`, `brk`, `hlt`).
fn hex_or_zero(value: u64) -> String {
    if value == 0 {
        "#0".to_string()
    } else {
        hex(value)
    }
}

/// `[base]` or `[base, #offset]`.
fn address(base: u32, offset: i64) -> String {
    if offset == 0 {
        format!("[{}]", x(base, true))
    } else {
        format!("[{}, {}]", x(base, true), signed_hex(offset))
    }
}

/// The condition names of `b.<cond>`, `csel` and `ccmp`, by their 4-bit code.
const CONDITIONS: [&str; 16] = [
    "eq", "ne", "hs", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "al", "nv",
];

fn condition(code: u32) -> &'static str {
    CONDITIONS[(code & 0xf) as usize]
}

fn sign_extend(value: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((value as i64) << shift) >> shift
}

/// `pc` plus a word offset of `bits` bits found at bit `low` of `word`.
fn relative(word: u32, pc: u64, low: u32, bits: u32) -> u64 {
    pc.wrapping_add((sign_extend(field(word, low, bits), bits) * 4) as u64)
}

/// Decode `word`, found at `pc`; `None` for a word it does not know.
pub(crate) fn decode(word: u32, pc: u64) -> Option<Instruction> {
    match field(word, 25, 4) {
        0b1000 | 0b1001 => integer::immediate(word, pc),
        0b1010 | 0b1011 => control::decode(word, pc),
        0b0100 | 0b0110 | 0b1100 | 0b1110 => memory::decode(word, pc),
        0b0101 | 0b1101 => integer::registers(word),
        0b0111 | 0b1111 => float::decode(word).or_else(|| vector::decode(word)),
        0b0000 if field(word, 16, 16) == 0 => {
            Some(instruction("udf", hex(u64::from(word & 0xffff))))
        }
        _ => None,
    }
}
