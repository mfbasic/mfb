//! Branches, exception generation and system instructions.

use super::{condition, field, hex, hex_or_zero, instruction, relative, rn, x, Flow, Instruction};

pub(super) fn decode(word: u32, pc: u64) -> Option<Instruction> {
    let pattern = |mask: u32, bits: u32| word & mask == bits;
    if pattern(0x7c00_0000, 0x1400_0000) {
        let call = word & 0x8000_0000 != 0;
        let target = relative(word, pc, 0, 26);
        return Some(if call {
            super::to_target("bl", "", target, Flow::Call)
        } else {
            super::to_target("b", "", target, Flow::Branch)
        });
    }
    if pattern(0x7e00_0000, 0x3400_0000) {
        let wide = word & 0x8000_0000 != 0;
        let mnemonic = if word & 0x0100_0000 != 0 {
            "cbnz"
        } else {
            "cbz"
        };
        let register = super::register(super::rd(word), wide, false);
        let target = relative(word, pc, 5, 19);
        return Some(super::to_target(
            mnemonic,
            &format!("{register}, "),
            target,
            Flow::Branch,
        ));
    }
    if pattern(0x7e00_0000, 0x3600_0000) {
        let wide = word & 0x8000_0000 != 0;
        let mnemonic = if word & 0x0100_0000 != 0 {
            "tbnz"
        } else {
            "tbz"
        };
        let number = (u32::from(wide) << 5) | field(word, 19, 5);
        let register = super::register(super::rd(word), wide, false);
        let target = relative(word, pc, 5, 14);
        return Some(super::to_target(
            mnemonic,
            &format!("{register}, {}, ", hex(u64::from(number))),
            target,
            Flow::Branch,
        ));
    }
    if pattern(0xff00_0000, 0x5400_0000) {
        let target = relative(word, pc, 5, 19);
        let consistent = if word & 0x10 != 0 { "bc" } else { "b" };
        return Some(super::to_target(
            &format!("{consistent}.{}", condition(word)),
            "",
            target,
            Flow::Branch,
        ));
    }
    if pattern(0xff00_0000, 0xd400_0000) {
        return exception(word);
    }
    if pattern(0xffc0_0000, 0xd500_0000) {
        return system(word);
    }
    if pattern(0xfe00_0000, 0xd600_0000) {
        return branch_register(word);
    }
    None
}

/// `svc`, `hvc`, `smc`, `brk`, `hlt`.
fn exception(word: u32) -> Option<Instruction> {
    let immediate = hex_or_zero(u64::from(field(word, 5, 16)));
    let mnemonic = match (field(word, 21, 3), field(word, 2, 3), field(word, 0, 2)) {
        (0b000, 0, 0b01) => "svc",
        (0b000, 0, 0b10) => "hvc",
        (0b000, 0, 0b11) => "smc",
        (0b001, 0, 0b00) => "brk",
        (0b010, 0, 0b00) => "hlt",
        _ => return None,
    };
    Some(instruction(mnemonic, immediate))
}

/// `br`, `blr`, `ret`, `eret` and their pointer-authenticating forms.
fn branch_register(word: u32) -> Option<Instruction> {
    let opc = field(word, 21, 4);
    let op2 = field(word, 16, 5);
    let op3 = field(word, 10, 6);
    let target = rn(word);
    let modifier = word & 0x1f;
    if op2 != 0x1f {
        return None;
    }
    let flow = match opc {
        0b0000 | 0b1000 => Flow::Branch,
        0b0001 | 0b1001 => Flow::Call,
        0b0010 => Flow::Return,
        _ => Flow::Next,
    };
    let mut decoded = match (opc, op3, modifier) {
        (0b0000, 0, 0) => instruction("br", x(target, false)),
        (0b0000, 0b10, 0x1f) => instruction("braaz", x(target, false)),
        (0b0000, 0b11, 0x1f) => instruction("brabz", x(target, false)),
        (0b0001, 0, 0) => instruction("blr", x(target, false)),
        (0b0001, 0b10, 0x1f) => instruction("blraaz", x(target, false)),
        (0b0001, 0b11, 0x1f) => instruction("blrabz", x(target, false)),
        (0b0010, 0, 0) if target == 30 => instruction("ret", String::new()),
        (0b0010, 0, 0) => instruction("ret", x(target, false)),
        (0b0010, 0b10, 0x1f) if target == 0x1f => instruction("retaa", String::new()),
        (0b0010, 0b11, 0x1f) if target == 0x1f => instruction("retab", String::new()),
        (0b0100, 0, 0) if target == 0x1f => instruction("eret", String::new()),
        (0b0100, 0b10, 0x1f) if target == 0x1f => instruction("eretaa", String::new()),
        (0b0100, 0b11, 0x1f) if target == 0x1f => instruction("eretab", String::new()),
        (0b0101, 0, 0) if target == 0x1f => instruction("drps", String::new()),
        (0b1000 | 0b1001, 0b10 | 0b11, _) => {
            let mnemonic = match (opc, op3) {
                (0b1000, 0b10) => "braa",
                (0b1000, _) => "brab",
                (_, 0b10) => "blraa",
                _ => "blrab",
            };
            instruction(
                mnemonic,
                format!("{}, {}", x(target, false), x(modifier, true)),
            )
        }
        _ => return None,
    };
    decoded.flow = flow;
    Some(decoded)
}

/// Hints, barriers, `msr` of a processor-state field, `sys` (as `dc`/`ic`)
/// and `mrs`/`msr` of a system register.
fn system(word: u32) -> Option<Instruction> {
    let read = word & 0x0020_0000 != 0;
    let op0 = field(word, 19, 2);
    let op1 = field(word, 16, 3);
    let crn = field(word, 12, 4);
    let crm = field(word, 8, 4);
    let op2 = field(word, 5, 3);
    let rt = word & 0x1f;
    match (read, op0) {
        (false, 0b00) if rt == 0x1f => match crn {
            0b0010 if op1 == 0b011 => Some(hint((crm << 3) | op2)),
            0b0011 if op1 == 0b011 => barrier(crm, op2),
            0b0100 => pstate(op1, op2, crm),
            _ => None,
        },
        (false, 0b01) => cache(op1, crn, crm, op2, rt),
        (_, 0b10 | 0b11) => {
            let name = system_register(op0, op1, crn, crm, op2);
            Some(if read {
                instruction("mrs", format!("{}, {name}", x(rt, false)))
            } else {
                instruction("msr", format!("{name}, {}", x(rt, false)))
            })
        }
        _ => None,
    }
}

fn hint(number: u32) -> Instruction {
    let name = match number {
        0x00 => "nop",
        0x01 => "yield",
        0x02 => "wfe",
        0x03 => "wfi",
        0x04 => "sev",
        0x05 => "sevl",
        0x06 => "dgh",
        0x07 => "xpaclri",
        0x08 => "pacia1716",
        0x0a => "pacib1716",
        0x0c => "autia1716",
        0x0e => "autib1716",
        0x10 => "esb",
        0x11 => "psb csync",
        0x12 => "tsb csync",
        0x13 => "gcsb dsync",
        0x14 => "csdb",
        0x16 => "clrbhb",
        0x18 => "paciaz",
        0x19 => "paciasp",
        0x1a => "pacibz",
        0x1b => "pacibsp",
        0x1c => "autiaz",
        0x1d => "autiasp",
        0x1e => "autibz",
        0x1f => "autibsp",
        0x20 => "bti",
        0x22 => "bti c",
        0x24 => "bti j",
        0x26 => "bti jc",
        0x27 => "pacm",
        0x28 => "chkfeat x16",
        _ => return instruction("hint", hex(u64::from(number))),
    };
    match name.split_once(' ') {
        Some((mnemonic, operand)) => instruction(mnemonic, operand.to_string()),
        None => instruction(name, String::new()),
    }
}

/// The option names of `dmb` and `dsb`, by their `CRm`.
const BARRIER_OPTIONS: [&str; 16] = [
    "", "oshld", "oshst", "osh", "", "nshld", "nshst", "nsh", "", "ishld", "ishst", "ish", "",
    "ld", "st", "sy",
];

fn barrier(crm: u32, op2: u32) -> Option<Instruction> {
    let option = || match BARRIER_OPTIONS[crm as usize] {
        "" => format!("#{crm}"),
        name => name.to_string(),
    };
    Some(match op2 {
        0b010 if crm == 0xf => instruction("clrex", String::new()),
        0b010 => instruction("clrex", hex(u64::from(crm))),
        0b100 if crm == 0b0000 => instruction("ssbb", String::new()),
        0b100 if crm == 0b0100 => instruction("pssbb", String::new()),
        0b100 if crm == 0b1100 => instruction("dfb", String::new()),
        0b100 => instruction("dsb", option()),
        0b101 => instruction("dmb", option()),
        0b110 if crm == 0xf => instruction("isb", String::new()),
        0b110 => instruction("isb", format!("#{crm}")),
        0b111 if crm == 0 => instruction("sb", String::new()),
        _ => return None,
    })
}

/// `msr <field>, #imm` of a processor-state field.
fn pstate(op1: u32, op2: u32, crm: u32) -> Option<Instruction> {
    let name = match (op1, op2) {
        (0b000, 0b011) => "UAO",
        (0b000, 0b100) => "PAN",
        (0b000, 0b101) => "SPSel",
        (0b011, 0b001) => "SSBS",
        (0b011, 0b010) => "DIT",
        (0b011, 0b100) => "TCO",
        (0b011, 0b110) => "DAIFSet",
        (0b011, 0b111) => "DAIFClr",
        _ => return None,
    };
    Some(instruction(
        "msr",
        format!("{name}, {}", hex(u64::from(crm))),
    ))
}

/// `sys` as the cache maintenance operation it names: `dc zva, x0`,
/// `ic ivau, x2`.
fn cache(op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> Option<Instruction> {
    if crn != 7 {
        return None;
    }
    let (mnemonic, operation) = match (op1, crm, op2) {
        (0, 1, 0) => ("ic", "ialluis"),
        (0, 5, 0) => ("ic", "iallu"),
        (3, 5, 1) => ("ic", "ivau"),
        (0, 6, 1) => ("dc", "ivac"),
        (0, 6, 2) => ("dc", "isw"),
        (0, 10, 2) => ("dc", "csw"),
        (0, 14, 2) => ("dc", "cisw"),
        (3, 4, 1) => ("dc", "zva"),
        (3, 10, 1) => ("dc", "cvac"),
        (3, 11, 1) => ("dc", "cvau"),
        (3, 12, 1) => ("dc", "cvap"),
        (3, 13, 1) => ("dc", "cvadp"),
        (3, 14, 1) => ("dc", "civac"),
        _ => return None,
    };
    let takes_register = !(mnemonic == "ic" && operation != "ivau");
    Some(match (takes_register, rt) {
        (true, _) => instruction(mnemonic, format!("{operation}, {}", x(rt, false))),
        (false, 0x1f) => instruction(mnemonic, operation.to_string()),
        (false, _) => return None,
    })
}

/// A system register by name where LLVM has one, else `S<op0>_<op1>_C<n>_C<m>_<op2>`.
fn system_register(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> String {
    let name = match (op0, op1, crn, crm, op2) {
        (3, 0, 0, 0, 0) => "MIDR_EL1",
        (3, 0, 0, 0, 5) => "MPIDR_EL1",
        (3, 0, 0, 0, 6) => "REVIDR_EL1",
        (3, 0, 0, 4, 0) => "ID_AA64PFR0_EL1",
        (3, 0, 0, 4, 1) => "ID_AA64PFR1_EL1",
        (3, 0, 0, 5, 0) => "ID_AA64DFR0_EL1",
        (3, 0, 0, 6, 0) => "ID_AA64ISAR0_EL1",
        (3, 0, 0, 6, 1) => "ID_AA64ISAR1_EL1",
        (3, 0, 0, 6, 2) => "ID_AA64ISAR2_EL1",
        (3, 0, 0, 7, 0) => "ID_AA64MMFR0_EL1",
        (3, 0, 0, 7, 1) => "ID_AA64MMFR1_EL1",
        (3, 0, 0, 7, 2) => "ID_AA64MMFR2_EL1",
        (3, 0, 4, 1, 0) => "SP_EL0",
        (3, 0, 4, 2, 0) => "SPSel",
        (3, 0, 4, 2, 2) => "CurrentEL",
        (3, 0, 4, 2, 3) => "PAN",
        (3, 0, 4, 2, 4) => "UAO",
        (3, 0, 13, 0, 1) => "CONTEXTIDR_EL1",
        (3, 0, 13, 0, 4) => "TPIDR_EL1",
        (3, 1, 0, 0, 0) => "CCSIDR_EL1",
        (3, 1, 0, 0, 1) => "CLIDR_EL1",
        (3, 2, 0, 0, 0) => "CSSELR_EL1",
        (3, 3, 0, 0, 1) => "CTR_EL0",
        (3, 3, 0, 0, 7) => "DCZID_EL0",
        (3, 3, 2, 4, 0) => "RNDR",
        (3, 3, 2, 4, 1) => "RNDRRS",
        (3, 3, 4, 2, 0) => "NZCV",
        (3, 3, 4, 2, 1) => "DAIF",
        (3, 3, 4, 2, 2) => "SVCR",
        (3, 3, 4, 2, 5) => "DIT",
        (3, 3, 4, 2, 6) => "SSBS",
        (3, 3, 4, 2, 7) => "TCO",
        (3, 3, 4, 4, 0) => "FPCR",
        (3, 3, 4, 4, 1) => "FPSR",
        (3, 3, 4, 5, 0) => "DSPSR_EL0",
        (3, 3, 4, 5, 1) => "DLR_EL0",
        (3, 3, 9, 12, 0) => "PMCR_EL0",
        (3, 3, 9, 13, 0) => "PMCCNTR_EL0",
        (3, 3, 13, 0, 2) => "TPIDR_EL0",
        (3, 3, 13, 0, 3) => "TPIDRRO_EL0",
        (3, 3, 13, 0, 5) => "TPIDR2_EL0",
        (3, 3, 14, 0, 0) => "CNTFRQ_EL0",
        (3, 3, 14, 0, 1) => "CNTPCT_EL0",
        (3, 3, 14, 0, 2) => "CNTVCT_EL0",
        (3, 3, 14, 0, 5) => "CNTPCTSS_EL0",
        (3, 3, 14, 0, 6) => "CNTVCTSS_EL0",
        (3, 3, 14, 2, 0) => "CNTP_TVAL_EL0",
        (3, 3, 14, 2, 1) => "CNTP_CTL_EL0",
        (3, 3, 14, 2, 2) => "CNTP_CVAL_EL0",
        (3, 3, 14, 3, 0) => "CNTV_TVAL_EL0",
        (3, 3, 14, 3, 1) => "CNTV_CTL_EL0",
        (3, 3, 14, 3, 2) => "CNTV_CVAL_EL0",
        (3, 4, 13, 0, 2) => "TPIDR_EL2",
        _ => return format!("S{op0}_{op1}_C{crn}_C{crm}_{op2}"),
    };
    name.to_string()
}
