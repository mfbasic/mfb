//! The vector (`V`) forms the compiler emits (`src/arch/riscv64/encode`'s
//! `emit_rv_vop`): configuration, unit-stride loads and stores, and the
//! arithmetic, compare, shift, move and conversion forms, masked or not.
//! They are spelled as `objdump -M max` spells them; a plain `objdump` on the
//! riscv64 boxes does not enable `V` and prints them as `.insn`.

use super::{funct3, instruction, rd, rs1, rs2, Instruction, X};

/// A vector register's name.
fn v(number: u8) -> String {
    format!("v{number}")
}

/// `,v0.t` for a masked instruction (`vm`, bit 25, clear), else nothing.
fn mask(word: u32) -> &'static str {
    if (word >> 25) & 1 == 0 {
        ",v0.t"
    } else {
        ""
    }
}

/// Whether `vm` is set: the instruction is not masked.
fn unmasked(word: u32) -> bool {
    (word >> 25) & 1 == 1
}

/// A unit-stride load or store (`vle<w>.v`, `vse<w>.v`) in `LOAD-FP` or
/// `STORE-FP`, or `None` for every other form those opcodes hold.
pub(super) fn load_store(word: u32) -> Option<Instruction> {
    let width = match funct3(word) {
        0 => 8,
        5 => 16,
        6 => 32,
        7 => 64,
        _ => return None,
    };
    // nf, mew and mop zero; lumop / sumop zero.
    if word >> 26 != 0 || rs2(word) != 0 {
        return None;
    }
    let store = word & 0x7f == 0x27;
    let mnemonic = format!("v{}e{width}.v", if store { "s" } else { "l" });
    Some(instruction(
        word,
        &mnemonic,
        format!(
            "{},({}){}",
            v(rd(word)),
            X[usize::from(rs1(word))],
            mask(word)
        ),
    ))
}

/// An `OP-V` instruction the compiler emits, or `None`.
pub(super) fn op_v(word: u32) -> Option<Instruction> {
    let funct6 = word >> 26;
    let (vd, vs2, vs1) = (v(rd(word)), v(rs2(word)), v(rs1(word)));
    let (xd, x1) = (X[usize::from(rd(word))], X[usize::from(rs1(word))]);
    let simm = super::sign_extend(u32::from(rs1(word)), 5);
    let uimm = rs1(word);
    let m = mask(word);
    let (mnemonic, operands): (&str, String) = match funct3(word) {
        // OPIVV
        0 => {
            let name = match funct6 {
                0b000000 => "vadd.vv",
                0b000010 => "vsub.vv",
                0b001001 => "vand.vv",
                0b001010 => "vor.vv",
                0b001011 => "vxor.vv",
                0b011000 => "vmseq.vv",
                0b011101 => "vmsle.vv",
                0b011011 => "vmslt.vv",
                0b100101 => "vsll.vv",
                0b101001 => "vsra.vv",
                0b101000 => "vsrl.vv",
                _ => return None,
            };
            (name, format!("{vd},{vs2},{vs1}{m}"))
        }
        // OPFVV
        1 => match funct6 {
            0b001001 if rs1(word) == rs2(word) => ("vfneg.v", format!("{vd},{vs2}{m}")),
            0b001010 if rs1(word) == rs2(word) => ("vfabs.v", format!("{vd},{vs2}{m}")),
            0b101100 => ("vfmacc.vv", format!("{vd},{vs1},{vs2}{m}")),
            0b101111 => ("vfnmsac.vv", format!("{vd},{vs1},{vs2}{m}")),
            0b010011 if rs1(word) == 0 => ("vfsqrt.v", format!("{vd},{vs2}{m}")),
            0b010010 => {
                let name = match rs1(word) {
                    0b00001 => "vfcvt.x.f.v",
                    0b00111 => "vfcvt.rtz.x.f.v",
                    0b00011 => "vfcvt.f.x.v",
                    _ => return None,
                };
                (name, format!("{vd},{vs2}{m}"))
            }
            _ => {
                let name = match funct6 {
                    0b000000 => "vfadd.vv",
                    0b000010 => "vfsub.vv",
                    0b100100 => "vfmul.vv",
                    0b100000 => "vfdiv.vv",
                    0b000100 => "vfmin.vv",
                    0b000110 => "vfmax.vv",
                    0b001001 => "vfsgnjn.vv",
                    0b001010 => "vfsgnjx.vv",
                    0b011000 => "vmfeq.vv",
                    0b011001 => "vmfle.vv",
                    0b011011 => "vmflt.vv",
                    _ => return None,
                };
                (name, format!("{vd},{vs2},{vs1}{m}"))
            }
        },
        // OPMVV
        2 if funct6 == 0b010000 && rs1(word) == 0 && unmasked(word) => {
            ("vmv.x.s", format!("{xd},{vs2}"))
        }
        // OPIVI
        3 => match funct6 {
            0b010111 if unmasked(word) && rs2(word) == 0 => ("vmv.v.i", format!("{vd},{simm}")),
            0b010111 if !unmasked(word) => ("vmerge.vim", format!("{vd},{vs2},{simm},v0")),
            _ => {
                let name = match funct6 {
                    0b100101 => "vsll.vi",
                    0b101001 => "vsra.vi",
                    0b101000 => "vsrl.vi",
                    0b001111 => "vslidedown.vi",
                    _ => return None,
                };
                (name, format!("{vd},{vs2},{uimm}{m}"))
            }
        },
        // OPIVX
        4 => match funct6 {
            0b000011 if rs1(word) == 0 => ("vneg.v", format!("{vd},{vs2}{m}")),
            0b010111 if unmasked(word) && rs2(word) == 0 => ("vmv.v.x", format!("{vd},{x1}")),
            _ => {
                let name = match funct6 {
                    0b000011 => "vrsub.vx",
                    0b100101 => "vsll.vx",
                    0b101001 => "vsra.vx",
                    0b101000 => "vsrl.vx",
                    _ => return None,
                };
                (name, format!("{vd},{vs2},{x1}{m}"))
            }
        },
        // OPMVX
        6 if funct6 == 0b010000 && rs2(word) == 0 && unmasked(word) => {
            ("vmv.s.x", format!("{vd},{x1}"))
        }
        // OPCFG
        7 if word >> 31 == 0 => (
            "vsetvli",
            format!("{xd},{x1},{}", vtype((word >> 20) & 0x7ff)?),
        ),
        7 if word >> 30 == 0b11 => (
            "vsetivli",
            format!("{xd},{uimm},{}", vtype((word >> 20) & 0x3ff)?),
        ),
        _ => return None,
    };
    Some(instruction(word, mnemonic, operands))
}

/// A `vtype` as `objdump` spells it: the element width, the group multiplier,
/// and the tail and mask policies (`e64,m1,ta,ma`); `None` for a reserved
/// setting.
fn vtype(bits: u32) -> Option<String> {
    if bits >> 8 != 0 {
        return None;
    }
    let lmul = match bits & 7 {
        0 => "m1",
        1 => "m2",
        2 => "m4",
        3 => "m8",
        5 => "mf8",
        6 => "mf4",
        7 => "mf2",
        _ => return None,
    };
    let sew = match (bits >> 3) & 7 {
        width @ 0..=3 => 8 << width,
        _ => return None,
    };
    let tail = if bits & 0x40 != 0 { "ta" } else { "tu" };
    let masked = if bits & 0x80 != 0 { "ma" } else { "mu" };
    Some(format!("e{sew},{lmul},{tail},{masked}"))
}
