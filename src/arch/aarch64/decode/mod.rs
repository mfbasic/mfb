//! AArch64 instruction decoder for the forms the encoder emits.
//!
//! The counterpart of [`super::encode`], and deliberately no wider: it knows
//! each bit pattern `encode::emitter` writes (plus the linker-filled fields of
//! `adrp`, `bl` and branches) and nothing else, so a word it does not know is
//! simply not one of ours. `tests.rs` holds the two sides together by encoding
//! every form and decoding it back.
//!
//! Text follows the LLVM disassembler that lldb uses on macOS: the mnemonic
//! padded to six columns, aliases where LLVM prints one (`mov`, `cmp`, `neg`,
//! `cset`, `lsl`…), immediates in hex, register 31 as `sp` or `xzr` by what the
//! form means, and NEON in Apple's syntax — the arrangement on the mnemonic
//! (`fadd.2d v1, v0, v2`), not on each register.

#[cfg(test)]
mod tests;

/// What an instruction does to the flow of control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Falls through to the next instruction.
    Next,
    /// `ret`: leaves the function.
    Return,
    /// `b`/`b.<cond>` to `target`.
    Branch,
    /// `bl` to `target`, or `blr`.
    Call,
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Instruction {
    /// `mnemonic operands`, the mnemonic padded to six columns.
    pub(crate) text: String,
    pub(crate) flow: Flow,
    /// The address a branch, call or `adrp` names.
    pub(crate) target: Option<u64>,
    /// The value of a wide immediate, for a comment (`mov x8, #0xde83`).
    pub(crate) value: Option<u64>,
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
    match (number, sp) {
        (31, true) => "sp".to_string(),
        (31, false) => "xzr".to_string(),
        (n, _) => format!("x{n}"),
    }
}

fn w(number: u32) -> String {
    if number == 31 {
        "wzr".to_string()
    } else {
        format!("w{number}")
    }
}

fn hex(value: u64) -> String {
    format!("#{value:#x}")
}

/// `[base]` or `[base, #offset]`.
fn address(base: u32, offset: u64) -> String {
    if offset == 0 {
        format!("[{}]", x(base, true))
    } else {
        format!("[{}, {}]", x(base, true), hex(offset))
    }
}

/// The condition names of `b.<cond>` and `cset`, by their 4-bit code.
const CONDITIONS: [&str; 16] = [
    "eq", "ne", "hs", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "al", "nv",
];

fn sign_extend(value: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((value as i64) << shift) >> shift
}

/// Decode `word`, found at `pc`; `None` for a word the encoder never emits.
pub(crate) fn decode(word: u32, pc: u64) -> Option<Instruction> {
    let pattern = |mask: u32, bits: u32| word & mask == bits;

    // Fixed words.
    match word {
        0xd65f_03c0 => {
            let mut ret = instruction("ret", String::new());
            ret.flow = Flow::Return;
            return Some(ret);
        }
        0xd400_0001 => return Some(instruction("svc", "#0".to_string())),
        _ => {}
    }

    // Branches.
    if pattern(0x7c00_0000, 0x1400_0000) {
        let target = pc.wrapping_add((sign_extend(word & 0x03ff_ffff, 26) * 4) as u64);
        let call = word & 0x8000_0000 != 0;
        let mut branch = instruction(if call { "bl" } else { "b" }, format!("{target:#x}"));
        branch.flow = if call { Flow::Call } else { Flow::Branch };
        branch.target = Some(target);
        return Some(branch);
    }
    if pattern(0xff00_0010, 0x5400_0000) {
        let target = pc.wrapping_add((sign_extend((word >> 5) & 0x7ffff, 19) * 4) as u64);
        let condition = CONDITIONS[(word & 0xf) as usize];
        let mut branch = instruction(&format!("b.{condition}"), format!("{target:#x}"));
        branch.flow = Flow::Branch;
        branch.target = Some(target);
        return Some(branch);
    }
    if pattern(0xffff_fc1f, 0xd63f_0000) {
        let mut call = instruction("blr", x(rn(word), false));
        call.flow = Flow::Call;
        return Some(call);
    }

    // `adrp`: a 4 KiB page relative to this instruction's page, printed as
    // that count of pages (as LLVM prints it).
    if pattern(0x9f00_0000, 0x9000_0000) {
        let immediate = (((word >> 5) & 0x7ffff) << 2) | ((word >> 29) & 0x3);
        let pages = sign_extend(immediate, 21);
        let page = (pc & !0xfff).wrapping_add((pages << 12) as u64);
        let mut adrp = instruction("adrp", format!("{}, {pages}", x(rd(word), false)));
        adrp.page = Some((rd(word), page));
        return Some(adrp);
    }

    // Moves of a wide immediate.
    if pattern(0xffe0_0000, 0xd280_0000) {
        let value = u64::from((word >> 5) & 0xffff);
        let mut mov = instruction("mov", format!("{}, {}", x(rd(word), false), hex(value)));
        mov.value = Some(value);
        return Some(mov);
    }
    if pattern(0xff80_0000, 0xf280_0000) {
        let value = u64::from((word >> 5) & 0xffff);
        let shift = ((word >> 21) & 0x3) * 16;
        return Some(instruction(
            "movk",
            format!("{}, {}, lsl #{shift}", x(rd(word), false), hex(value)),
        ));
    }

    // Add/subtract, immediate (register 31 is `sp`).
    if pattern(0x9f80_0000, 0x9100_0000) {
        let subtract = word & 0x4000_0000 != 0;
        let set_flags = word & 0x2000_0000 != 0;
        let imm12 = u64::from((word >> 10) & 0xfff);
        let shifted = word & 0x0040_0000 != 0;
        let immediate = if shifted {
            format!("{}, lsl #12", hex(imm12))
        } else {
            hex(imm12)
        };
        let (dst, src) = (rd(word), rn(word));
        return Some(match (subtract, set_flags) {
            (false, false) if imm12 == 0 && !shifted && (dst == 31 || src == 31) => {
                instruction("mov", format!("{}, {}", x(dst, true), x(src, true)))
            }
            (false, false) => {
                let mut add = instruction(
                    "add",
                    format!("{}, {}, {immediate}", x(dst, true), x(src, true)),
                );
                if !shifted {
                    add.page_offset = Some((src, imm12));
                }
                add
            }
            (true, false) => instruction(
                "sub",
                format!("{}, {}, {immediate}", x(dst, true), x(src, true)),
            ),
            (true, true) if dst == 31 => {
                instruction("cmp", format!("{}, {immediate}", x(src, true)))
            }
            _ => return None,
        });
    }

    // Logical and add/subtract, shifted register (unshifted; 31 is `xzr`).
    let three = |mnemonic: &str| {
        instruction(
            mnemonic,
            format!(
                "{}, {}, {}",
                x(rd(word), false),
                x(rn(word), false),
                x(rm(word), false)
            ),
        )
    };
    match word & 0xffe0_fc00 {
        0xaa00_0000 if rn(word) == 31 => {
            return Some(instruction(
                "mov",
                format!("{}, {}", x(rd(word), false), x(rm(word), false)),
            ))
        }
        0xaa20_0000 if rn(word) == 31 => {
            return Some(instruction(
                "mvn",
                format!("{}, {}", x(rd(word), false), x(rm(word), false)),
            ))
        }
        0xcb00_0000 if rn(word) == 31 => {
            return Some(instruction(
                "neg",
                format!("{}, {}", x(rd(word), false), x(rm(word), false)),
            ))
        }
        0xeb00_0000 if rd(word) == 31 => {
            return Some(instruction(
                "cmp",
                format!("{}, {}", x(rn(word), false), x(rm(word), false)),
            ))
        }
        0x8a00_0000 => return Some(three("and")),
        0xaa00_0000 => return Some(three("orr")),
        0xca00_0000 => return Some(three("eor")),
        0x8b00_0000 => return Some(three("add")),
        0xab00_0000 => return Some(three("adds")),
        0xcb00_0000 => return Some(three("sub")),
        0xeb00_0000 => return Some(three("subs")),
        0xba00_0000 => return Some(three("adcs")),
        0xfa00_0000 => return Some(three("sbcs")),
        0x9b00_7c00 => return Some(three("mul")),
        0x9b40_7c00 => return Some(three("smulh")),
        0x9bc0_7c00 => return Some(three("umulh")),
        0x9ac0_0c00 => return Some(three("sdiv")),
        0x9ac0_0800 => return Some(three("udiv")),
        0x9ac0_2000 => return Some(three("lsl")),
        0x9ac0_2400 => return Some(three("lsr")),
        0x9ac0_2800 => return Some(three("asr")),
        0x9ac0_2c00 => return Some(three("ror")),
        0x1ac0_2c00 => {
            return Some(instruction(
                "ror",
                format!("{}, {}, {}", w(rd(word)), w(rn(word)), w(rm(word))),
            ))
        }
        _ => {}
    }
    if pattern(0xffe0_8000, 0x9b00_8000) {
        return Some(instruction(
            "msub",
            format!(
                "{}, {}, {}, {}",
                x(rd(word), false),
                x(rn(word), false),
                x(rm(word), false),
                x(ra(word), false)
            ),
        ));
    }

    // `cset` (`csinc Xd, xzr, xzr, <inverted cond>`).
    if pattern(0xffff_0fe0, 0x9a9f_07e0) {
        let condition = CONDITIONS[(((word >> 12) & 0xf) ^ 1) as usize];
        return Some(instruction(
            "cset",
            format!("{}, {condition}", x(rd(word), false)),
        ));
    }

    // Data processing, one source.
    let two = |mnemonic: &str| {
        instruction(
            mnemonic,
            format!("{}, {}", x(rd(word), false), x(rn(word), false)),
        )
    };
    match word & 0xffff_fc00 {
        0xdac0_1000 => return Some(two("clz")),
        0xdac0_0000 => return Some(two("rbit")),
        0xdac0_0c00 => return Some(two("rev")),
        0x5ac0_0800 => {
            return Some(instruction(
                "rev",
                format!("{}, {}", w(rd(word)), w(rn(word))),
            ))
        }
        0x9340_7c00 => {
            return Some(instruction(
                "sxtw",
                format!("{}, {}", x(rd(word), false), w(rn(word))),
            ))
        }
        _ => {}
    }

    // Shifts by an immediate (`ubfm`/`sbfm` aliases).
    if pattern(0xffc0_0000, 0xd340_0000) || pattern(0xffc0_0000, 0x9340_0000) {
        let immr = (word >> 16) & 0x3f;
        let imms = (word >> 10) & 0x3f;
        let unsigned = word & 0x4000_0000 != 0;
        let operands =
            |amount: u32| format!("{}, {}, #{amount}", x(rd(word), false), x(rn(word), false));
        return match (unsigned, imms) {
            (true, 63) => Some(instruction("lsr", operands(immr))),
            (false, 63) => Some(instruction("asr", operands(immr))),
            (true, _) if (imms + 1) & 63 == immr => Some(instruction("lsl", operands(63 - imms))),
            _ => None,
        };
    }

    // Loads and stores, unsigned scaled offset (base 31 is `sp`).
    let scaled = |scale: u64| u64::from((word >> 10) & 0xfff) * scale;
    let memory = |mnemonic: &str, register: String, scale: u64| {
        instruction(
            mnemonic,
            format!("{register}, {}", address(rn(word), scaled(scale))),
        )
    };
    let gpr = x(rd(word), false);
    match word & 0xffc0_0000 {
        0xf940_0000 => return Some(memory("ldr", gpr, 8)),
        0xf900_0000 => return Some(memory("str", gpr, 8)),
        0xb940_0000 => return Some(memory("ldr", w(rd(word)), 4)),
        0xb900_0000 => return Some(memory("str", w(rd(word)), 4)),
        0x7940_0000 => return Some(memory("ldrh", w(rd(word)), 2)),
        0x7900_0000 => return Some(memory("strh", w(rd(word)), 2)),
        0x3940_0000 => return Some(memory("ldrb", w(rd(word)), 1)),
        0x3900_0000 => return Some(memory("strb", w(rd(word)), 1)),
        0xfd40_0000 => return Some(memory("ldr", format!("d{}", rd(word)), 8)),
        0xfd00_0000 => return Some(memory("str", format!("d{}", rd(word)), 8)),
        0x3dc0_0000 => return Some(memory("ldr", format!("q{}", rd(word)), 16)),
        0x3d80_0000 => return Some(memory("str", format!("q{}", rd(word)), 16)),
        _ => {}
    }

    // Load-acquire / store-release.
    match word & 0xffff_fc00 {
        0xc89f_fc00 => return Some(memory("stlr", x(rd(word), false), 0)),
        0xc8df_fc00 => return Some(memory("ldar", x(rd(word), false), 0)),
        0x88df_fc00 => return Some(memory("ldar", w(rd(word)), 0)),
        _ => {}
    }

    decode_floating(word)
}

/// Scalar floating point and NEON.
fn decode_floating(word: u32) -> Option<Instruction> {
    let d = |number: u32| format!("d{number}");
    let s = |number: u32| format!("s{number}");
    let v = |number: u32| format!("v{number}");

    // FP data processing, two sources (`.d`).
    let fp_two = match word & 0xffe0_fc00 {
        0x1e60_2800 => Some("fadd"),
        0x1e60_3800 => Some("fsub"),
        0x1e60_0800 => Some("fmul"),
        0x1e60_1800 => Some("fdiv"),
        0x1e60_7800 => Some("fminnm"),
        0x1e60_6800 => Some("fmaxnm"),
        _ => None,
    };
    if let Some(mnemonic) = fp_two {
        return Some(instruction(
            mnemonic,
            format!("{}, {}, {}", d(rd(word)), d(rn(word)), d(rm(word))),
        ));
    }
    if word & 0xffe0_fc1f == 0x1e60_2000 {
        return Some(instruction(
            "fcmp",
            format!("{}, {}", d(rn(word)), d(rm(word))),
        ));
    }
    if word & 0xffff_fc1f == 0x1e60_2008 {
        return Some(instruction("fcmp", format!("{}, #0.0", d(rn(word)))));
    }

    // FP one source, and moves and conversions between register files.
    let one = match word & 0xffff_fc00 {
        0x1e60_4000 => Some(("fmov", d(rd(word)), d(rn(word)))),
        0x1e61_4000 => Some(("fneg", d(rd(word)), d(rn(word)))),
        0x1e60_c000 => Some(("fabs", d(rd(word)), d(rn(word)))),
        0x1e61_c000 => Some(("fsqrt", d(rd(word)), d(rn(word)))),
        // bug-788: the single/double conversions a `CFloat` crosses with.
        0x1e62_4000 => Some(("fcvt", s(rd(word)), d(rn(word)))),
        0x1e22_c000 => Some(("fcvt", d(rd(word)), s(rn(word)))),
        0x9e66_0000 => Some(("fmov", x(rd(word), false), d(rn(word)))),
        0x9e67_0000 => Some(("fmov", d(rd(word)), x(rn(word), false))),
        0x9e62_0000 => Some(("scvtf", d(rd(word)), x(rn(word), false))),
        0x9e78_0000 => Some(("fcvtzs", x(rd(word), false), d(rn(word)))),
        0x9e70_0000 => Some(("fcvtms", x(rd(word), false), d(rn(word)))),
        0x9e68_0000 => Some(("fcvtps", x(rd(word), false), d(rn(word)))),
        0x9e64_0000 => Some(("fcvtas", x(rd(word), false), d(rn(word)))),
        0x4e08_0c00 => Some(("dup.2d", v(rd(word)), x(rn(word), false))),
        _ => None,
    };
    if let Some((mnemonic, dst, src)) = one {
        return Some(instruction(mnemonic, format!("{dst}, {src}")));
    }

    // FP three sources (`.d`).
    let fma = match word & 0xffe0_8000 {
        0x1f40_0000 => Some("fmadd"),
        0x1f40_8000 => Some("fmsub"),
        0x1f60_0000 => Some("fnmadd"),
        0x1f60_8000 => Some("fnmsub"),
        _ => None,
    };
    if let Some(mnemonic) = fma {
        return Some(instruction(
            mnemonic,
            format!(
                "{}, {}, {}, {}",
                d(rd(word)),
                d(rn(word)),
                d(rm(word)),
                d(ra(word))
            ),
        ));
    }

    // NEON three same.
    let three_same = match word & 0xffe0_fc00 {
        0x4e60_d400 => Some(("fadd", false)),
        0x4ee0_d400 => Some(("fsub", false)),
        0x6e60_dc00 => Some(("fmul", false)),
        0x6e60_fc00 => Some(("fdiv", false)),
        0x4e60_cc00 => Some(("fmla", false)),
        0x4ee0_cc00 => Some(("fmls", false)),
        0x4ee0_f400 => Some(("fmin", false)),
        0x4e60_f400 => Some(("fmax", false)),
        0x6ee0_e400 => Some(("fcmgt", false)),
        0x6e60_e400 => Some(("fcmge", false)),
        0x4e60_e400 => Some(("fcmeq", false)),
        0x4ee0_8400 => Some(("add", false)),
        0x6ee0_8400 => Some(("sub", false)),
        0x4ee0_3400 => Some(("cmgt", false)),
        0x4ee0_3c00 => Some(("cmge", false)),
        0x6ee0_8c00 => Some(("cmeq", false)),
        0x4ee0_4400 => Some(("sshl", false)),
        0x6ee0_4400 => Some(("ushl", false)),
        0x4e20_1c00 => Some(("and", true)),
        0x4ea0_1c00 => Some(("orr", true)),
        0x6e20_1c00 => Some(("eor", true)),
        0x6e60_1c00 => Some(("bsl", true)),
        0x6ea0_1c00 => Some(("bit", true)),
        _ => None,
    };
    if let Some((mnemonic, bytes)) = three_same {
        let arrangement = if bytes { "16b" } else { "2d" };
        // `orr` of a register with itself is the vector `mov`.
        if mnemonic == "orr" && rn(word) == rm(word) {
            return Some(instruction(
                "mov.16b",
                format!("{}, {}", v(rd(word)), v(rn(word))),
            ));
        }
        return Some(instruction(
            &format!("{mnemonic}.{arrangement}"),
            format!("{}, {}, {}", v(rd(word)), v(rn(word)), v(rm(word))),
        ));
    }

    // NEON two-register misc.
    let two_misc = match word & 0xffff_fc00 {
        0x4ee0_f800 => Some(("fabs", "")),
        0x6ee0_f800 => Some(("fneg", "")),
        0x6ee1_f800 => Some(("fsqrt", "")),
        0x4ee1_8800 => Some(("frintp", "")),
        0x4e61_9800 => Some(("frintm", "")),
        0x6e61_8800 => Some(("frinta", "")),
        0x4e61_8800 => Some(("frintn", "")),
        0x4ee1_9800 => Some(("frintz", "")),
        0x4ee1_b800 => Some(("fcvtzs", "")),
        0x4e61_c800 => Some(("fcvtas", "")),
        0x4e61_d800 => Some(("scvtf", "")),
        0x6ee0_b800 => Some(("neg", "")),
        0x4ee0_b800 => Some(("abs", "")),
        0x4ee0_c800 => Some(("fcmgt", ", #0.0")),
        0x6ee0_c800 => Some(("fcmge", ", #0.0")),
        0x4ee0_d800 => Some(("fcmeq", ", #0.0")),
        0x4ee0_e800 => Some(("fcmlt", ", #0.0")),
        0x6ee0_d800 => Some(("fcmle", ", #0.0")),
        _ => None,
    };
    if let Some((mnemonic, zero)) = two_misc {
        return Some(instruction(
            &format!("{mnemonic}.2d"),
            format!("{}, {}{zero}", v(rd(word)), v(rn(word))),
        ));
    }
    match word & 0xffff_fc00 {
        0x0e20_5800 => {
            return Some(instruction(
                "cnt.8b",
                format!("{}, {}", v(rd(word)), v(rn(word))),
            ))
        }
        0x0e31_b800 => {
            return Some(instruction(
                "addv.8b",
                format!("b{}, {}", rd(word), v(rn(word))),
            ))
        }
        _ => {}
    }

    // NEON shift by immediate (`.2d`: immh is 1xxx).
    let immhb = (word >> 16) & 0x7f;
    let shift = match word & 0xff80_fc00 {
        0x4f00_5400 if immhb >= 64 => Some(("shl", immhb - 64)),
        0x4f00_0400 if immhb >= 64 => Some(("sshr", 128 - immhb)),
        0x6f00_0400 if immhb >= 64 => Some(("ushr", 128 - immhb)),
        _ => None,
    };
    if let Some((mnemonic, amount)) = shift {
        return Some(instruction(
            &format!("{mnemonic}.2d"),
            format!(
                "{}, {}, {}",
                v(rd(word)),
                v(rn(word)),
                hex(u64::from(amount))
            ),
        ));
    }

    // `umov Xd, Vn.d[i]`, which LLVM prints as `mov`.
    if word & 0xffef_fc00 == 0x4e08_3c00 {
        return Some(instruction(
            "mov.d",
            format!(
                "{}, {}[{}]",
                x(rd(word), false),
                v(rn(word)),
                (word >> 20) & 1
            ),
        ));
    }
    None
}
