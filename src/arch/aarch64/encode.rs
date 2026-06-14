use std::collections::HashMap;

use crate::arch::aarch64::ops::CodeOp;
use crate::target::shared::code::{CodeInstruction, NativeCodePlan};

pub(crate) struct EncodedImage {
    pub(crate) text: Vec<u8>,
    pub(crate) data: Vec<u8>,
    pub(crate) symbols: Vec<EncodedSymbol>,
    pub(crate) relocations: Vec<EncodedRelocation>,
    pub(crate) imports: Vec<EncodedImport>,
    pub(crate) entry: String,
}

pub(crate) struct EncodedSymbol {
    pub(crate) name: String,
    pub(crate) section: EncodedSection,
    pub(crate) offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EncodedSection {
    Text,
    Data,
}

pub(crate) struct EncodedRelocation {
    pub(crate) offset: usize,
    pub(crate) target: String,
    pub(crate) kind: String,
    pub(crate) binding: String,
    pub(crate) library: Option<String>,
}

pub(crate) struct EncodedImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
}

pub(crate) fn encode(plan: &NativeCodePlan) -> Result<EncodedImage, String> {
    let mut encoder = Encoder {
        text: Vec::new(),
        data: encode_data(plan)?,
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: plan
            .imports
            .iter()
            .map(|import| (import.symbol.clone(), import.library.clone()))
            .collect(),
        labels: HashMap::new(),
        patches: Vec::new(),
    };

    let mut data_offset = 0;
    for object in &plan.data_objects {
        data_offset = align(data_offset, object.align);
        encoder.symbols.push(EncodedSymbol {
            name: object.symbol.clone(),
            section: EncodedSection::Data,
            offset: data_offset,
        });
        data_offset += object.size;
    }

    let mut text_offset = 0;
    for function in &plan.functions {
        encoder.symbols.push(EncodedSymbol {
            name: function.symbol.clone(),
            section: EncodedSection::Text,
            offset: text_offset,
        });
        for instruction in &function.instructions {
            text_offset += instruction_size(instruction)?;
        }
    }

    for function in &plan.functions {
        encoder.labels.clear();
        let function_start = encoder.text.len();
        for instruction in &function.instructions {
            if instruction.op == CodeOp::Label {
                encoder
                    .labels
                    .insert(field(instruction, "name")?, encoder.text.len());
            } else {
                encoder
                    .text
                    .resize(encoder.text.len() + instruction_size(instruction)?, 0);
            }
        }
        encoder.text.truncate(function_start);
        for instruction in &function.instructions {
            encoder.emit_instruction(instruction)?;
        }
        encoder.patch_labels()?;
        encoder.patches.clear();
    }

    let imports = plan
        .imports
        .iter()
        .map(|import| EncodedImport {
            library: import.library.clone(),
            symbol: import.symbol.clone(),
        })
        .collect();

    Ok(EncodedImage {
        text: encoder.text,
        data: encoder.data,
        symbols: encoder.symbols,
        relocations: encoder.relocations,
        imports,
        entry: plan
            .entry_symbol
            .clone()
            .ok_or_else(|| "encoded image requires entry symbol".to_string())?,
    })
}

struct Encoder {
    text: Vec<u8>,
    data: Vec<u8>,
    symbols: Vec<EncodedSymbol>,
    relocations: Vec<EncodedRelocation>,
    imports: HashMap<String, String>,
    labels: HashMap<String, usize>,
    patches: Vec<LabelPatch>,
}

struct LabelPatch {
    offset: usize,
    target: String,
    kind: String,
}

impl Encoder {
    fn emit_instruction(&mut self, instruction: &CodeInstruction) -> Result<(), String> {
        match instruction.op.mnemonic() {
            "label" => Ok(()),
            "mov" => self.emit_mov(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "mov_imm" => self.emit_mov_imm(
                reg(field(instruction, "dst")?)?,
                immediate(field(instruction, "value")?)?,
            ),
            "add" => self.emit_add(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "adds" => self.emit_adds(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "sub" => self.emit_sub(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "subs" => self.emit_subs(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "and" => self.emit_and(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "orr" => self.emit_orr(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "eor" => self.emit_eor(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "mvn" => self.emit_mvn(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "mul" => self.emit_mul(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "smulh" => self.emit_smulh(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "sdiv" => self.emit_sdiv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "udiv" => self.emit_udiv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "msub" => self.emit_msub(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
                reg(field(instruction, "minuend")?)?,
            ),
            "lsl_imm" => self.emit_lsl_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                shift(field(instruction, "shift")?)?,
            ),
            "lsr_imm" => self.emit_lsr_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                shift(field(instruction, "shift")?)?,
            ),
            "asr_imm" => self.emit_asr_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                shift(field(instruction, "shift")?)?,
            ),
            "add_imm" => self.emit_add_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                immediate(field(instruction, "imm")?)?,
            ),
            "sub_imm" => self.emit_sub_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                immediate(field(instruction, "imm")?)?,
            ),
            "sub_sp" => self.emit_sub_sp(immediate(field(instruction, "imm")?)?),
            "add_sp" => self.emit_add_sp(immediate(field(instruction, "imm")?)?),
            "cmp_imm" => self.emit_cmp_imm(
                reg(field(instruction, "lhs")?)?,
                immediate(field(instruction, "rhs")?)?,
            ),
            "cmp" => self.emit_cmp(
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "b.eq" => self.emit_label_branch("b.eq", field(instruction, "target")?),
            "b.ne" => self.emit_label_branch("b.ne", field(instruction, "target")?),
            "b.ge" => self.emit_label_branch("b.ge", field(instruction, "target")?),
            "b.lt" => self.emit_label_branch("b.lt", field(instruction, "target")?),
            "b.gt" => self.emit_label_branch("b.gt", field(instruction, "target")?),
            "b.le" => self.emit_label_branch("b.le", field(instruction, "target")?),
            "b.vc" => self.emit_label_branch("b.vc", field(instruction, "target")?),
            "b.hi" => self.emit_label_branch("b.hi", field(instruction, "target")?),
            "b.lo" => self.emit_label_branch("b.lo", field(instruction, "target")?),
            "b" => self.emit_label_branch("b", field(instruction, "target")?),
            "bl" => self.emit_bl(field(instruction, "target")?),
            "blr" => self.emit_blr(reg(field(instruction, "register")?)?),
            "svc" => self.emit_word(0xd400_0001),
            "branch_self" => self.emit_word(0x1400_0000),
            "ret" => self.emit_word(0xd65f_03c0),
            "ldr_u64" => self.emit_ldr_u64(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "ldr_u32" => self.emit_ldr_u32(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "ldr_u16" => self.emit_ldr_u16(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "ldr_u8" => self.emit_ldr_u8(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_u64" => self.emit_str_u64(
                reg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_u8" => self.emit_str_u8(
                reg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "adrp" => self.emit_symbol_ref(
                "adrp",
                reg(field(instruction, "dst")?)?,
                field(instruction, "symbol")?,
            ),
            "add_pageoff" => self.emit_symbol_ref(
                "add_pageoff",
                reg(field(instruction, "dst")?)?,
                field(instruction, "symbol")?,
            ),
            "fmov_x_from_d" => self.emit_fmov_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fmov_d_from_x" => self.emit_fmov_d_from_x(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fadd_d" => self.emit_fadd_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fsub_d" => self.emit_fsub_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fmul_d" => self.emit_fmul_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fdiv_d" => self.emit_fdiv_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fneg_d" => self.emit_fneg_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fsqrt_d" => self.emit_fsqrt_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcmp_zero_d" => self.emit_fcmp_zero_d(reg(field(instruction, "src")?)?),
            "scvtf_d_from_x" => self.emit_scvtf_d_from_x(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvtzs_x_from_d" => self.emit_fcvtzs_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvtms_x_from_d" => self.emit_fcvtms_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvtps_x_from_d" => self.emit_fcvtps_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvtas_x_from_d" => self.emit_fcvtas_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            other => Err(format!(
                "AArch64 encoder does not support instruction '{other}'"
            )),
        }
    }

    fn emit_word(&mut self, word: u32) -> Result<(), String> {
        self.text.extend_from_slice(&word.to_le_bytes());
        Ok(())
    }

    fn emit_mov(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xaa00_03e0 | ((rn as u32) << 16) | rd as u32)
    }

    fn emit_mov_imm(&mut self, rd: u8, value: u64) -> Result<(), String> {
        let mut emitted = false;
        for (index, shift) in [0, 16, 32, 48].into_iter().enumerate() {
            let part = ((value >> shift) & 0xffff) as u32;
            if index == 0 {
                self.emit_word(0xd280_0000 | (part << 5) | rd as u32)?;
                emitted = true;
            } else if part != 0 {
                self.emit_word(
                    0xf280_0000 | (((shift / 16) as u32) << 21) | (part << 5) | rd as u32,
                )?;
            }
        }
        if !emitted {
            self.emit_word(0xd280_0000 | rd as u32)?;
        }
        Ok(())
    }

    fn emit_add(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x8b00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_adds(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xab00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_sub(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xcb00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_subs(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xeb00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_and(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x8a00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_orr(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xaa00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_eor(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xca00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_mvn(&mut self, rd: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xaa20_03e0 | ((rm as u32) << 16) | rd as u32)
    }

    fn emit_mul(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9b00_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_smulh(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9b40_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_sdiv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_0c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_udiv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_0800 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_msub(&mut self, rd: u8, rn: u8, rm: u8, ra: u8) -> Result<(), String> {
        self.emit_word(
            0x9b00_8000
                | ((rm as u32) << 16)
                | ((ra as u32) << 10)
                | ((rn as u32) << 5)
                | rd as u32,
        )
    }

    fn emit_lsl_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 lsl shift {shift} is out of range"));
        }
        let immr = (64 - shift as u32) & 63;
        let imms = 63 - shift as u32;
        self.emit_word(0xd340_0000 | (immr << 16) | (imms << 10) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_lsr_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 lsr shift {shift} is out of range"));
        }
        self.emit_word(
            0xd340_0000 | ((shift as u32) << 16) | (63 << 10) | ((rn as u32) << 5) | rd as u32,
        )
    }

    fn emit_asr_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 asr shift {shift} is out of range"));
        }
        self.emit_word(
            0x9340_0000 | ((shift as u32) << 16) | (63 << 10) | ((rn as u32) << 5) | rd as u32,
        )
    }

    fn emit_add_imm(&mut self, rd: u8, rn: u8, imm: u64) -> Result<(), String> {
        if let Some((imm12, shift12)) = encode_add_sub_imm(imm) {
            return self.emit_add_imm_chunk(rd, rn, imm12, shift12);
        }
        let mut remaining = imm;
        let mut src = rn;
        while remaining > 0 {
            let (chunk, shift12) = next_add_sub_chunk(remaining);
            self.emit_add_imm_chunk(rd, src, chunk, shift12)?;
            remaining -= if shift12 {
                u64::from(chunk) << 12
            } else {
                u64::from(chunk)
            };
            src = rd;
        }
        Ok(())
    }

    fn emit_add_imm_chunk(&mut self, rd: u8, rn: u8, imm12: u32, shift12: bool) -> Result<(), String> {
        self.emit_word(
            0x9100_0000
                | ((u32::from(shift12)) << 22)
                | (imm12 << 10)
                | ((rn as u32) << 5)
                | rd as u32,
        )
    }

    fn emit_sub_imm(&mut self, rd: u8, rn: u8, imm: u64) -> Result<(), String> {
        if let Some((imm12, shift12)) = encode_add_sub_imm(imm) {
            return self.emit_sub_imm_chunk(rd, rn, imm12, shift12);
        }
        let mut remaining = imm;
        let mut src = rn;
        while remaining > 0 {
            let (chunk, shift12) = next_add_sub_chunk(remaining);
            self.emit_sub_imm_chunk(rd, src, chunk, shift12)?;
            remaining -= if shift12 {
                u64::from(chunk) << 12
            } else {
                u64::from(chunk)
            };
            src = rd;
        }
        Ok(())
    }

    fn emit_sub_imm_chunk(&mut self, rd: u8, rn: u8, imm12: u32, shift12: bool) -> Result<(), String> {
        self.emit_word(
            0xd100_0000
                | ((u32::from(shift12)) << 22)
                | (imm12 << 10)
                | ((rn as u32) << 5)
                | rd as u32,
        )
    }

    fn emit_sub_sp(&mut self, imm: u64) -> Result<(), String> {
        self.emit_sub_imm(31, 31, imm)
    }

    fn emit_add_sp(&mut self, imm: u64) -> Result<(), String> {
        self.emit_add_imm(31, 31, imm)
    }

    fn emit_cmp_imm(&mut self, rn: u8, imm: u64) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(imm) {
            return self.emit_word(0xf100_001f | (imm << 10) | ((rn as u32) << 5));
        }
        let scratch = scratch_excluding(rn, 31);
        self.emit_mov_imm(scratch, imm)?;
        self.emit_cmp(rn, scratch)
    }

    fn emit_cmp(&mut self, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xeb00_001f | ((rm as u32) << 16) | ((rn as u32) << 5))
    }

    fn emit_fmov_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e66_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_fmov_d_from_x(&mut self, dd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x9e67_0000 | ((rn as u32) << 5) | dd as u32)
    }

    fn emit_fadd_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_2800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fsub_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_3800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fmul_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_0800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fdiv_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_1800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fneg_d(&mut self, dd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x1e61_4000 | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fsqrt_d(&mut self, dd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x1e61_c000 | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fcmp_zero_d(&mut self, dn: u8) -> Result<(), String> {
        self.emit_word(0x1e60_2000 | ((dn as u32) << 5) | 0x8)
    }

    fn emit_scvtf_d_from_x(&mut self, dd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x9e62_0000 | ((rn as u32) << 5) | dd as u32)
    }

    fn emit_fcvtzs_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e78_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_fcvtms_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e70_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_fcvtps_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e68_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_fcvtas_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e64_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_ldr_u64(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 8 != 0 {
            return Err(format!("unaligned AArch64 ldr offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 8) {
            return self.emit_word(0xf940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xf940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_ldr_u32(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 4 != 0 {
            return Err(format!("unaligned AArch64 ldr u32 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 4) {
            return self.emit_word(0xb940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xb940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_ldr_u16(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 2 != 0 {
            return Err(format!("unaligned AArch64 ldr u16 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 2) {
            return self.emit_word(0x7940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x7940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_str_u64(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 8 != 0 {
            return Err(format!("unaligned AArch64 str offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 8) {
            return self.emit_word(0xf900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xf900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_ldr_u8(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(offset) {
            return self.emit_word(0x3940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x3940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_str_u8(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(offset) {
            return self.emit_word(0x3900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x3900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_label_branch(&mut self, kind: &str, target: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(0)?;
        self.patches.push(LabelPatch {
            offset,
            target,
            kind: kind.to_string(),
        });
        Ok(())
    }

    fn emit_bl(&mut self, target: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(0x9400_0000)?;
        if self.symbols.iter().any(|symbol| symbol.name == target) {
            self.relocations.push(EncodedRelocation {
                offset,
                target,
                kind: "branch26".to_string(),
                binding: "internal".to_string(),
                library: None,
            });
        } else if let Some(library) = self.imports.get(&target) {
            self.relocations.push(EncodedRelocation {
                offset,
                target,
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some(library.clone()),
            });
        } else {
            return Err(format!(
                "AArch64 branch target symbol '{target}' does not resolve"
            ));
        }
        Ok(())
    }

    fn emit_blr(&mut self, rn: u8) -> Result<(), String> {
        self.emit_word(0xd63f_0000 | ((rn as u32) << 5))
    }

    fn emit_symbol_ref(&mut self, kind: &str, rd: u8, symbol: String) -> Result<(), String> {
        let offset = self.text.len();
        match kind {
            "adrp" => self.emit_word(0x9000_0000 | rd as u32)?,
            "add_pageoff" => self.emit_word(0x9100_0000 | ((rd as u32) << 5) | rd as u32)?,
            _ => unreachable!(),
        }
        let relocation_kind = if kind == "adrp" {
            "page21"
        } else {
            "pageoff12"
        }
        .to_string();
        if let Some(library) = self.imports.get(&symbol) {
            self.relocations.push(EncodedRelocation {
                offset,
                target: symbol,
                kind: relocation_kind,
                binding: "external".to_string(),
                library: Some(library.clone()),
            });
        } else {
            self.relocations.push(EncodedRelocation {
                offset,
                target: symbol,
                kind: relocation_kind,
                binding: "data".to_string(),
                library: None,
            });
        }
        Ok(())
    }

    fn patch_labels(&mut self) -> Result<(), String> {
        for patch in &self.patches {
            let Some(&target) = self.labels.get(&patch.target) else {
                return Err(format!(
                    "AArch64 branch target label '{}' does not resolve",
                    patch.target
                ));
            };
            let word = match patch.kind.as_str() {
                "b" => 0x1400_0000 | branch_imm26(patch.offset, target),
                "b.eq" => 0x5400_0000 | (branch_imm19(patch.offset, target) << 5),
                "b.ne" => 0x5400_0001 | (branch_imm19(patch.offset, target) << 5),
                "b.ge" => 0x5400_000a | (branch_imm19(patch.offset, target) << 5),
                "b.lt" => 0x5400_000b | (branch_imm19(patch.offset, target) << 5),
                "b.gt" => 0x5400_000c | (branch_imm19(patch.offset, target) << 5),
                "b.le" => 0x5400_000d | (branch_imm19(patch.offset, target) << 5),
                "b.vc" => 0x5400_0007 | (branch_imm19(patch.offset, target) << 5),
                "b.hi" => 0x5400_0008 | (branch_imm19(patch.offset, target) << 5),
                "b.lo" => 0x5400_0003 | (branch_imm19(patch.offset, target) << 5),
                other => return Err(format!("unknown AArch64 branch patch '{other}'")),
            };
            self.text[patch.offset..patch.offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        Ok(())
    }
}

fn encode_data(plan: &NativeCodePlan) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    for object in &plan.data_objects {
        data.resize(align(data.len(), object.align), 0);
        if object.kind == "raw" {
            data.extend_from_slice(&decode_hex_bytes(&object.value)?);
        } else {
            put_u64(&mut data, object.value.len() as u64);
            data.extend_from_slice(object.value.as_bytes());
            data.push(0);
        }
        data.resize(align(data.len(), object.align), 0);
    }
    Ok(data)
}

fn decode_hex_bytes(value: &str) -> Result<Vec<u8>, String> {
    let compact = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'_')
        .collect::<Vec<_>>();
    if compact.len() % 2 != 0 {
        return Err("raw data object hex value must have an even digit count".to_string());
    }
    compact
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_digit(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("raw data object contains non-hex digit".to_string()),
    }
}

fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String> {
    match instruction.op {
        CodeOp::Label => return Ok(0),
        CodeOp::MovImm => {
            return Ok(wide_imm_word_count(immediate(field(instruction, "value")?)?) * 4);
        }
        CodeOp::AddImm | CodeOp::SubImm => {
            return Ok(sized_add_sub_imm(immediate(field(instruction, "imm")?)?));
        }
        CodeOp::AddSp | CodeOp::SubSp | CodeOp::CmpImm => {
            return Ok(sized_add_sub_imm(immediate(
                field(instruction, if instruction.op == CodeOp::CmpImm {
                    "rhs"
                } else {
                    "imm"
                })?,
            )?));
        }
        CodeOp::LdrU64 | CodeOp::StrU64 => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                8,
            ));
        }
        CodeOp::LdrU32 => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                4,
            ));
        }
        CodeOp::LdrU16 => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                2,
            ));
        }
        CodeOp::LdrU8 | CodeOp::StrU8 => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                1,
            ));
        }
        _ => {}
    }
    Ok(4)
}

fn wide_imm_word_count(value: u64) -> usize {
    1 + [16, 32, 48]
        .into_iter()
        .filter(|shift| ((value >> shift) & 0xffff) != 0)
        .count()
}

fn field(instruction: &CodeInstruction, name: &str) -> Result<String, String> {
    instruction
        .fields
        .iter()
        .find(|(field, _)| *field == name)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| {
            format!(
                "instruction '{}' missing field '{name}'",
                instruction.op.mnemonic()
            )
        })
}

fn reg(name: String) -> Result<u8, String> {
    match name.as_str() {
        "sp" | "x31" | "xzr" => Ok(31),
        "x0" | "w0" => Ok(0),
        "x1" | "w1" => Ok(1),
        "x2" | "w2" => Ok(2),
        "x3" | "w3" => Ok(3),
        "x4" | "w4" => Ok(4),
        "x5" | "w5" => Ok(5),
        "x6" | "w6" => Ok(6),
        "x7" | "w7" => Ok(7),
        "x8" | "w8" => Ok(8),
        "x9" | "w9" => Ok(9),
        "x10" | "w10" => Ok(10),
        "x11" | "w11" => Ok(11),
        "x12" | "w12" => Ok(12),
        "x13" | "w13" => Ok(13),
        "x14" | "w14" => Ok(14),
        "x15" | "w15" => Ok(15),
        "x16" | "w16" => Ok(16),
        "x17" | "w17" => Ok(17),
        "x19" | "w19" => Ok(19),
        "x20" | "w20" => Ok(20),
        "x21" | "w21" => Ok(21),
        "x22" | "w22" => Ok(22),
        "x23" | "w23" => Ok(23),
        "x24" | "w24" => Ok(24),
        "x25" | "w25" => Ok(25),
        "x26" | "w26" => Ok(26),
        "x27" | "w27" => Ok(27),
        "x28" | "w28" => Ok(28),
        "x30" | "lr" => Ok(30),
        "d0" => Ok(0),
        "d1" => Ok(1),
        "d2" => Ok(2),
        "d3" => Ok(3),
        "d4" => Ok(4),
        "d5" => Ok(5),
        "d6" => Ok(6),
        "d7" => Ok(7),
        other => Err(format!("unknown AArch64 register '{other}'")),
    }
}

fn immediate(value: String) -> Result<u64, String> {
    match value.as_str() {
        "true" => Ok(1),
        "false" => Ok(0),
        _ => value
            .parse::<u64>()
            .map_err(|_| format!("invalid immediate '{value}'")),
    }
}

fn shift(value: String) -> Result<u8, String> {
    let value = value
        .parse::<u8>()
        .map_err(|_| format!("invalid shift immediate '{value}'"))?;
    if value >= 64 {
        return Err(format!("shift immediate {value} is out of range"));
    }
    Ok(value)
}

fn checked_imm12(value: u64) -> Result<u32, String> {
    if value > 4095 {
        return Err(format!("AArch64 immediate {value} exceeds 12-bit encoding"));
    }
    Ok(value as u32)
}

fn encode_add_sub_imm(value: u64) -> Option<(u32, bool)> {
    if value <= 4095 {
        Some((value as u32, false))
    } else if value.is_multiple_of(4096) && (value >> 12) <= 4095 {
        Some(((value >> 12) as u32, true))
    } else {
        None
    }
}

fn scratch_excluding(a: u8, b: u8) -> u8 {
    [17, 16, 15]
        .into_iter()
        .find(|candidate| *candidate != a && *candidate != b)
        .expect("scratch register candidate list is non-empty")
}

fn sized_add_sub_imm(value: u64) -> usize {
    if value == 0 {
        return 4;
    }
    let mut remaining = value;
    let mut words = 0;
    while remaining > 0 {
        let (chunk, shift12) = next_add_sub_chunk(remaining);
        remaining -= if shift12 {
            u64::from(chunk) << 12
        } else {
            u64::from(chunk)
        };
        words += 1;
    }
    words * 4
}

fn sized_memory_imm(offset: u64, scale: u64) -> usize {
    if offset.is_multiple_of(scale) && (offset / scale) <= 4095 {
        4
    } else {
        sized_add_sub_imm(offset) + 4
    }
}

fn next_add_sub_chunk(remaining: u64) -> (u32, bool) {
    if remaining >= 4096 {
        (((remaining / 4096).min(4095)) as u32, true)
    } else {
        (remaining as u32, false)
    }
}

fn branch_imm26(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x03ff_ffff
}

fn branch_imm19(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x0007_ffff
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::ops::CodeOp;
    use crate::target::shared::code::CodeInstruction;

    #[test]
    fn instruction_size_expands_large_stack_and_memory_immediates() {
        let sub_sp = CodeInstruction::new("sub_sp").field("imm", "6400");
        let ldr = CodeInstruction::new("ldr_u64")
            .field("dst", "x0")
            .field("base", "sp")
            .field("offset", "32800");

        assert_eq!(instruction_size(&sub_sp).unwrap(), 8);
        assert_eq!(instruction_size(&ldr).unwrap(), 12);
    }

    #[test]
    fn encoder_accepts_large_immediates_with_fallback_sequences() {
        let mut encoder = Encoder {
            text: Vec::new(),
            data: Vec::new(),
            symbols: Vec::new(),
            relocations: Vec::new(),
            imports: HashMap::new(),
            labels: HashMap::new(),
            patches: Vec::new(),
        };

        encoder
            .emit_instruction(&CodeInstruction::new("sub_sp").field("imm", "6400"))
            .unwrap();
        encoder
            .emit_instruction(
                &CodeInstruction::new("str_u64")
                    .field("src", "x0")
                    .field("base", "sp")
                    .field("offset", "32800"),
            )
            .unwrap();
        encoder
            .emit_instruction(
                &CodeInstruction::new("cmp_imm")
                    .field("lhs", "x1")
                    .field("rhs", "6400"),
            )
            .unwrap();

        assert_eq!(encoder.text.len(), 28);
    }

    #[test]
    fn codeop_sizes_cover_large_add_imm() {
        let add = CodeInstruction {
            op: CodeOp::AddImm,
            fields: vec![
                ("dst", "x0".to_string()),
                ("src", "sp".to_string()),
                ("imm", "6400".to_string()),
            ],
        };

        assert_eq!(instruction_size(&add).unwrap(), 8);
    }
}
