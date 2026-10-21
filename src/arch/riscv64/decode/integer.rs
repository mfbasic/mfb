//! RV64I, `M`, `Zicsr` and `Zifencei`: every base opcode but `AMO`, the
//! floating-point ones and the vector ones.

use super::{
    address, b_imm, funct3, funct7, i_imm, instruction, j_imm, rd, rs1, rs2, s_imm, u_imm, Flow,
    Instruction, X,
};

/// The base instruction `word` at `pc`, or `None` when it is none of RV64I,
/// `M`, `Zicsr`, `Zifencei`.
pub(super) fn decode(word: u32, pc: u64) -> Option<Instruction> {
    match word & 0x7f {
        0x03 => load(word),
        0x0f => misc_mem(word),
        0x13 => op_imm(word),
        0x17 => Some(upper(word, "auipc", pc)),
        0x1b => op_imm_32(word),
        0x23 => store(word),
        0x33 => op(word),
        0x37 => Some(upper(word, "lui", pc)),
        0x3b => op_32(word),
        0x63 => branch(word, pc),
        0x67 => jalr(word),
        0x6f => Some(jal(word, pc)),
        0x73 => system(word),
        _ => None,
    }
}

/// `offset(register)`, the address operand of a load, a store and `jalr`.
pub(super) fn memory(offset: i64, base: u8) -> String {
    format!("{offset}({})", X[usize::from(base)])
}

fn load(word: u32) -> Option<Instruction> {
    let mnemonic = match funct3(word) {
        0 => "lb",
        1 => "lh",
        2 => "lw",
        3 => "ld",
        4 => "lbu",
        5 => "lhu",
        6 => "lwu",
        _ => return None,
    };
    let offset = i_imm(word);
    let mut decoded = instruction(
        word,
        mnemonic,
        format!("{},{}", X[usize::from(rd(word))], memory(offset, rs1(word))),
    );
    decoded.page_offset = Some((rs1(word), offset));
    Some(decoded)
}

fn store(word: u32) -> Option<Instruction> {
    let mnemonic = match funct3(word) {
        0 => "sb",
        1 => "sh",
        2 => "sw",
        3 => "sd",
        _ => return None,
    };
    let offset = s_imm(word);
    let mut decoded = instruction(
        word,
        mnemonic,
        format!(
            "{},{}",
            X[usize::from(rs2(word))],
            memory(offset, rs1(word))
        ),
    );
    decoded.page_offset = Some((rs1(word), offset));
    Some(decoded)
}

/// `fence` and `fence.i`.
fn misc_mem(word: u32) -> Option<Instruction> {
    match funct3(word) {
        0 => {
            let fm = word >> 28;
            let pred = (word >> 24) & 0xf;
            let succ = (word >> 20) & 0xf;
            if fm == 0b1000 && pred == 0b0011 && succ == 0b0011 {
                return Some(instruction(word, "fence.tso", String::new()));
            }
            // `pause` (`fence w,0`) prints as a plain `fence`, since `objdump`
            // does not know Zihintpause by default.
            if pred == 0b1111 && succ == 0b1111 {
                return Some(instruction(word, "fence", String::new()));
            }
            Some(instruction(
                word,
                "fence",
                format!("{},{}", fence_set(pred), fence_set(succ)),
            ))
        }
        1 => Some(instruction(word, "fence.i", String::new())),
        _ => None,
    }
}

/// A `fence`'s predecessor or successor set: the letters of `iorw` it holds,
/// or `unknown` for an empty set, as `objdump` prints it.
fn fence_set(bits: u32) -> String {
    let set = "iorw"
        .chars()
        .enumerate()
        .filter(|(index, _)| bits & (0b1000 >> index) != 0)
        .map(|(_, letter)| letter)
        .collect::<String>();
    if set.is_empty() {
        "unknown".to_string()
    } else {
        set
    }
}

fn op_imm(word: u32) -> Option<Instruction> {
    let (rd, rs1, imm) = (rd(word), rs1(word), i_imm(word));
    let (d, s) = (X[usize::from(rd)], X[usize::from(rs1)]);
    let shamt = (word >> 20) & 0x3f;
    let decoded = match funct3(word) {
        0 if rd == 0 && rs1 == 0 && imm == 0 => instruction(word, "nop", String::new()),
        0 if rs1 == 0 => {
            let mut decoded = instruction(word, "li", format!("{d},{imm}"));
            decoded.value = Some(imm);
            decoded
        }
        0 if imm == 0 => instruction(word, "mv", format!("{d},{s}")),
        0 => {
            let mut decoded = instruction(word, "addi", format!("{d},{s},{imm}"));
            decoded.page_offset = Some((rs1, imm));
            decoded
        }
        1 if word >> 26 == 0 => instruction(word, "slli", format!("{d},{s},{shamt:#x}")),
        2 => instruction(word, "slti", format!("{d},{s},{imm}")),
        3 if imm == 1 => instruction(word, "seqz", format!("{d},{s}")),
        3 => instruction(word, "sltiu", format!("{d},{s},{imm}")),
        4 if imm == -1 => instruction(word, "not", format!("{d},{s}")),
        4 => instruction(word, "xori", format!("{d},{s},{imm}")),
        5 if word >> 26 == 0 => instruction(word, "srli", format!("{d},{s},{shamt:#x}")),
        5 if word >> 26 == 0b01_0000 => instruction(word, "srai", format!("{d},{s},{shamt:#x}")),
        6 => instruction(word, "ori", format!("{d},{s},{imm}")),
        7 if imm == 255 => instruction(word, "zext.b", format!("{d},{s}")),
        7 => instruction(word, "andi", format!("{d},{s},{imm}")),
        _ => return None,
    };
    Some(decoded)
}

fn op_imm_32(word: u32) -> Option<Instruction> {
    let (d, s, imm) = (
        X[usize::from(rd(word))],
        X[usize::from(rs1(word))],
        i_imm(word),
    );
    let shamt = (word >> 20) & 0x1f;
    let decoded = match (funct3(word), funct7(word)) {
        (0, _) if imm == 0 => instruction(word, "sext.w", format!("{d},{s}")),
        (0, _) => instruction(word, "addiw", format!("{d},{s},{imm}")),
        (1, 0) => instruction(word, "slliw", format!("{d},{s},{shamt:#x}")),
        (5, 0) => instruction(word, "srliw", format!("{d},{s},{shamt:#x}")),
        (5, 0b010_0000) => instruction(word, "sraiw", format!("{d},{s},{shamt:#x}")),
        _ => return None,
    };
    Some(decoded)
}

/// `lui` and `auipc`: the 20-bit immediate in hex; `auipc` also records the
/// address it forms.
fn upper(word: u32, mnemonic: &str, pc: u64) -> Instruction {
    let mut decoded = instruction(
        word,
        mnemonic,
        format!("{},{:#x}", X[usize::from(rd(word))], word >> 12),
    );
    if mnemonic == "auipc" {
        decoded.page = Some((rd(word), pc.wrapping_add_signed(u_imm(word))));
    }
    decoded
}

fn op(word: u32) -> Option<Instruction> {
    let (rd, rs1, rs2) = (rd(word), rs1(word), rs2(word));
    let (d, a, b) = (X[usize::from(rd)], X[usize::from(rs1)], X[usize::from(rs2)]);
    let mnemonic = match (funct7(word), funct3(word)) {
        (0, 0) => "add",
        (0, 1) => "sll",
        (0, 2) if rs1 == 0 => return Some(instruction(word, "sgtz", format!("{d},{b}"))),
        (0, 2) if rs2 == 0 => return Some(instruction(word, "sltz", format!("{d},{a}"))),
        (0, 2) => "slt",
        (0, 3) if rs1 == 0 => return Some(instruction(word, "snez", format!("{d},{b}"))),
        (0, 3) => "sltu",
        (0, 4) => "xor",
        (0, 5) => "srl",
        (0, 6) => "or",
        (0, 7) => "and",
        (0b010_0000, 0) if rs1 == 0 => return Some(instruction(word, "neg", format!("{d},{b}"))),
        (0b010_0000, 0) => "sub",
        (0b010_0000, 5) => "sra",
        (1, 0) => "mul",
        (1, 1) => "mulh",
        (1, 2) => "mulhsu",
        (1, 3) => "mulhu",
        (1, 4) => "div",
        (1, 5) => "divu",
        (1, 6) => "rem",
        (1, 7) => "remu",
        _ => return None,
    };
    Some(instruction(word, mnemonic, format!("{d},{a},{b}")))
}

fn op_32(word: u32) -> Option<Instruction> {
    let (rd, rs1, rs2) = (rd(word), rs1(word), rs2(word));
    let (d, a, b) = (X[usize::from(rd)], X[usize::from(rs1)], X[usize::from(rs2)]);
    let mnemonic = match (funct7(word), funct3(word)) {
        (0, 0) => "addw",
        (0, 1) => "sllw",
        (0, 5) => "srlw",
        (0b010_0000, 0) if rs1 == 0 => return Some(instruction(word, "negw", format!("{d},{b}"))),
        (0b010_0000, 0) => "subw",
        (0b010_0000, 5) => "sraw",
        (1, 0) => "mulw",
        (1, 4) => "divw",
        (1, 5) => "divuw",
        (1, 6) => "remw",
        (1, 7) => "remuw",
        _ => return None,
    };
    Some(instruction(word, mnemonic, format!("{d},{a},{b}")))
}

/// A conditional branch, with `objdump`'s aliases for a comparison with
/// `zero`.
fn branch(word: u32, pc: u64) -> Option<Instruction> {
    let (rs1, rs2) = (rs1(word), rs2(word));
    let (a, b) = (X[usize::from(rs1)], X[usize::from(rs2)]);
    let target = pc.wrapping_add_signed(b_imm(word));
    let to = address(target);
    let (mnemonic, operands) = match funct3(word) {
        0 if rs2 == 0 => ("beqz", format!("{a},{to}")),
        0 => ("beq", format!("{a},{b},{to}")),
        1 if rs2 == 0 => ("bnez", format!("{a},{to}")),
        1 => ("bne", format!("{a},{b},{to}")),
        4 if rs2 == 0 => ("bltz", format!("{a},{to}")),
        4 if rs1 == 0 => ("bgtz", format!("{b},{to}")),
        4 => ("blt", format!("{a},{b},{to}")),
        5 if rs2 == 0 => ("bgez", format!("{a},{to}")),
        5 if rs1 == 0 => ("blez", format!("{b},{to}")),
        5 => ("bge", format!("{a},{b},{to}")),
        6 => ("bltu", format!("{a},{b},{to}")),
        7 => ("bgeu", format!("{a},{b},{to}")),
        _ => return None,
    };
    let mut decoded = instruction(word, mnemonic, operands);
    decoded.flow = Flow::Branch;
    decoded.target = Some(target);
    Some(decoded)
}

/// `jal`: `j` when it links nothing, the bare target when it links `ra`.
fn jal(word: u32, pc: u64) -> Instruction {
    let target = pc.wrapping_add_signed(j_imm(word));
    let (mnemonic, operands, flow) = match rd(word) {
        0 => ("j", address(target), Flow::Jump),
        1 => ("jal", address(target), Flow::Call),
        rd => (
            "jal",
            format!("{},{}", X[usize::from(rd)], address(target)),
            Flow::Call,
        ),
    };
    let mut decoded = instruction(word, mnemonic, operands);
    decoded.flow = flow;
    decoded.target = Some(target);
    decoded
}

/// `jalr`: `ret`, `jr` when it links nothing, the bare address when it links
/// `ra`; a zero offset is left out.
fn jalr(word: u32) -> Option<Instruction> {
    if funct3(word) != 0 {
        return None;
    }
    let (rd, rs1, offset) = (rd(word), rs1(word), i_imm(word));
    let through = if offset == 0 {
        X[usize::from(rs1)].to_string()
    } else {
        memory(offset, rs1)
    };
    let mut decoded = match rd {
        0 if rs1 == 1 && offset == 0 => {
            let mut decoded = instruction(word, "ret", String::new());
            decoded.flow = Flow::Return;
            return Some(decoded);
        }
        0 => instruction(word, "jr", through),
        1 => instruction(word, "jalr", through),
        rd => instruction(word, "jalr", format!("{},{through}", X[usize::from(rd)])),
    };
    decoded.flow = Flow::Indirect {
        register: rs1,
        offset,
        link: rd != 0,
    };
    decoded.page_offset = Some((rs1, offset));
    Some(decoded)
}

/// `ecall`, `ebreak`, the privileged instructions a program may show, and the
/// `Zicsr` forms with `objdump`'s aliases.
fn system(word: u32) -> Option<Instruction> {
    let decoded = match word {
        0x0000_0073 => trap(word, "ecall"),
        0x0010_0073 => trap(word, "ebreak"),
        0x1050_0073 => instruction(word, "wfi", String::new()),
        0x1020_0073 => instruction(word, "sret", String::new()),
        0x3020_0073 => instruction(word, "mret", String::new()),
        // `csrrw zero, cycle, zero`: the canonical illegal instruction.
        0xc000_1073 => instruction(word, "unimp", String::new()),
        _ => return csr(word),
    };
    Some(decoded)
}

/// An instruction that traps into the system or the debugger.
fn trap(word: u32, mnemonic: &str) -> Instruction {
    let mut decoded = instruction(word, mnemonic, String::new());
    decoded.flow = Flow::Trap;
    decoded
}

/// A `Zicsr` instruction: `csrrw`, `csrrs`, `csrrc` and their immediate forms,
/// with the reading and writing aliases `objdump` prints, the floating-point
/// status ones (`frflags`, `fsrm`, …) first.
fn csr(word: u32) -> Option<Instruction> {
    let (rd, rs1) = (rd(word), rs1(word));
    let number = word >> 20;
    let name = csr_name(number);
    let d = X[usize::from(rd)];
    let s = X[usize::from(rs1)];
    // The floating-point status registers' own spellings: read, write, and
    // write with the old value.
    let float = match number {
        1 => Some(("frflags", "fsflags", "fsflagsi")),
        2 => Some(("frrm", "fsrm", "fsrmi")),
        3 => Some(("frcsr", "fscsr", "")),
        _ => None,
    };
    let counter = match number {
        0xc00 => Some("rdcycle"),
        0xc01 => Some("rdtime"),
        0xc02 => Some("rdinstret"),
        _ => None,
    };
    let (mnemonic, operands) = match funct3(word) {
        // csrrw
        1 => match (float, rd) {
            (Some((_, write, _)), 0) => (write.to_string(), s.to_string()),
            (Some((_, write, _)), _) => (write.to_string(), format!("{d},{s}")),
            (None, 0) => ("csrw".to_string(), format!("{name},{s}")),
            (None, _) => ("csrrw".to_string(), format!("{d},{name},{s}")),
        },
        // csrrs
        2 => match (float, counter, rd, rs1) {
            (Some((read, _, _)), _, _, 0) => (read.to_string(), d.to_string()),
            (_, Some(read), _, 0) => (read.to_string(), d.to_string()),
            (_, _, _, 0) => ("csrr".to_string(), format!("{d},{name}")),
            (_, _, 0, _) => ("csrs".to_string(), format!("{name},{s}")),
            _ => ("csrrs".to_string(), format!("{d},{name},{s}")),
        },
        // csrrc
        3 => match rd {
            0 => ("csrc".to_string(), format!("{name},{s}")),
            _ => ("csrrc".to_string(), format!("{d},{name},{s}")),
        },
        // csrrwi
        5 => match (float, rd) {
            (Some((_, _, write)), 0) if !write.is_empty() => (write.to_string(), rs1.to_string()),
            (Some((_, _, write)), _) if !write.is_empty() => {
                (write.to_string(), format!("{d},{rs1}"))
            }
            (_, 0) => ("csrwi".to_string(), format!("{name},{rs1}")),
            _ => ("csrrwi".to_string(), format!("{d},{name},{rs1}")),
        },
        // csrrsi
        6 => match rd {
            0 => ("csrsi".to_string(), format!("{name},{rs1}")),
            _ => ("csrrsi".to_string(), format!("{d},{name},{rs1}")),
        },
        // csrrci
        7 => match rd {
            0 => ("csrci".to_string(), format!("{name},{rs1}")),
            _ => ("csrrci".to_string(), format!("{d},{name},{rs1}")),
        },
        _ => return None,
    };
    Some(instruction(word, &mnemonic, operands))
}

/// A CSR's name as `objdump` prints it: the user-level ones a program
/// touches, else the number in hex.
fn csr_name(number: u32) -> String {
    match number {
        0x001 => "fflags",
        0x002 => "frm",
        0x003 => "fcsr",
        0x008 => "vstart",
        0x009 => "vxsat",
        0x00a => "vxrm",
        0x00f => "vcsr",
        0x015 => "seed",
        0xc00 => "cycle",
        0xc01 => "time",
        0xc02 => "instret",
        0xc20 => "vl",
        0xc21 => "vtype",
        0xc22 => "vlenb",
        _ => return format!("{number:#x}"),
    }
    .to_string()
}
