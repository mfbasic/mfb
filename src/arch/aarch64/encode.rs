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
    /// Internal text symbols run, in order, after dynamic relocations and before
    /// the program entry (plan-linker.md §5.3). Materialized as ELF
    /// `DT_INIT_ARRAY` / Mach-O `S_MOD_INIT_FUNC_POINTERS`.
    pub(crate) initializers: Vec<String>,
    pub(crate) signing_metadata: Option<Vec<u8>>,
}

/// Whether an imported symbol names a function (called through a stub) or a data
/// global (addressed through the GOT). Makes linker layout deterministic without
/// scanning relocations (plan-linker.md §5.1). `Data` is produced by a
/// `tls`/app-mode consumer (and the linker tests) once one exists; the built-in
/// surface is function-only, so allow it to be otherwise-unconstructed for now.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ImportKind {
    Function,
    Data,
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
    /// Function (stub) vs data global (GOT-only) (plan-linker.md §5.1).
    pub(crate) kind: ImportKind,
    /// glibc symbol version this reference requires, e.g. `Some("GLIBC_2.17")`
    /// (plan-linker.md §5.2). `None` emits an unversioned reference. Ignored on
    /// Mach-O, which selects by dylib ordinal.
    pub(crate) version: Option<String>,
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
            // The built-in surface is function-only and unversioned; a versioned
            // or data import is supplied by a `tls`/app-mode consumer once one
            // exists (plan-linker.md §3.1). Default accordingly.
            kind: ImportKind::Function,
            version: None,
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
        initializers: Vec::new(),
        signing_metadata: None,
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
            "umulh" => self.emit_umulh(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "adc" => self.emit_adc(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "rorv" => self.emit_rorv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "rorv_w" => self.emit_rorv_w(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "lslv" => self.emit_lslv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "lsrv" => self.emit_lsrv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "asrv" => self.emit_asrv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "clz" => self.emit_clz(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "rbit" => self.emit_rbit(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "rev_w" => self.emit_rev_w(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "rev_x" => self.emit_rev_x(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
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
            "str_u32" => self.emit_str_u32(
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
            "fcmp_d" => self.emit_fcmp_d(
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
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
            "ldr_q" => self.emit_ldr_q(
                vreg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_q" => self.emit_str_q(
                vreg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "fadd_v" | "fsub_v" | "fmul_v" | "fdiv_v" | "fmla_v" | "fmls_v" | "fmin_v"
            | "fmax_v" | "fcmgt_v" | "fcmge_v" | "fcmeq_v" | "add_v" | "sub_v" | "cmgt_v"
            | "cmge_v" | "cmeq_v" | "sshl_v" | "ushl_v" | "and_v" | "orr_v" | "eor_v"
            | "bsl_v" | "bit_v" => self.emit_v_three_same(
                instruction.op,
                vreg(field(instruction, "dst")?)?,
                vreg(field(instruction, "lhs")?)?,
                vreg(field(instruction, "rhs")?)?,
            ),
            "fabs_v" | "fneg_v" | "fsqrt_v" | "frintp_v" | "frintm_v" | "frinta_v"
            | "frintn_v" | "frintz_v" | "fcvtzs_v" | "fcvtas_v" | "scvtf_v" | "neg_v"
            | "abs_v" | "fcmgt_zero_v" | "fcmge_zero_v" | "fcmeq_zero_v" | "fcmlt_zero_v"
            | "fcmle_zero_v" => self.emit_v_two_misc(
                instruction.op,
                vreg(field(instruction, "dst")?)?,
                vreg(field(instruction, "src")?)?,
            ),
            "shl_v" | "sshr_v" | "ushr_v" => self.emit_v_shift_imm(
                instruction.op,
                vreg(field(instruction, "dst")?)?,
                vreg(field(instruction, "src")?)?,
                shift(field(instruction, "shift")?)?,
            ),
            "dup_v_from_x" => self.emit_dup_v_from_x(
                vreg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "umov_x_from_v" => self.emit_umov_x_from_v(
                reg(field(instruction, "dst")?)?,
                vreg(field(instruction, "src")?)?,
                immediate(field(instruction, "index")?)?,
            ),
            other => Err(format!(
                "AArch64 encoder does not support instruction '{other}'"
            )),
        }
    }

    /// 128-bit `LDR Qt, [Xn, #offset]` — offset must be a multiple of 16.
    fn emit_ldr_q(&mut self, vt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 16 != 0 || offset / 16 > 4095 {
            return Err(format!("AArch64 ldr q offset {offset} is not encodable"));
        }
        let imm12 = (offset / 16) as u32;
        self.emit_word(0x3dc0_0000 | (imm12 << 10) | ((rn as u32) << 5) | vt as u32)
    }

    /// 128-bit `STR Qt, [Xn, #offset]` — offset must be a multiple of 16.
    fn emit_str_q(&mut self, vt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 16 != 0 || offset / 16 > 4095 {
            return Err(format!("AArch64 str q offset {offset} is not encodable"));
        }
        let imm12 = (offset / 16) as u32;
        self.emit_word(0x3d80_0000 | (imm12 << 10) | ((rn as u32) << 5) | vt as u32)
    }

    /// Three-same NEON ops: `op Vd.<T>, Vn.<T>, Vm.<T>`. The arrangement (`.2d`
    /// for numeric lanes, `.16b` for the bitwise ops) is baked into the base word.
    fn emit_v_three_same(&mut self, op: CodeOp, vd: u8, vn: u8, vm: u8) -> Result<(), String> {
        let base = match op {
            CodeOp::FAddV => 0x4e60_d400,
            CodeOp::FSubV => 0x4ee0_d400,
            CodeOp::FMulV => 0x6e60_dc00,
            CodeOp::FDivV => 0x6e60_fc00,
            CodeOp::FMlaV => 0x4e60_cc00,
            CodeOp::FMlsV => 0x4ee0_cc00,
            CodeOp::FMinV => 0x4ee0_f400,
            CodeOp::FMaxV => 0x4e60_f400,
            CodeOp::FCmGtV => 0x6ee0_e400,
            CodeOp::FCmGeV => 0x6e60_e400,
            CodeOp::FCmEqV => 0x4e60_e400,
            CodeOp::AddV => 0x4ee0_8400,
            CodeOp::SubV => 0x6ee0_8400,
            CodeOp::CmGtV => 0x4ee0_3400,
            CodeOp::CmGeV => 0x4ee0_3c00,
            CodeOp::CmEqV => 0x6ee0_8c00,
            CodeOp::SshlV => 0x4ee0_4400,
            CodeOp::UshlV => 0x6ee0_4400,
            CodeOp::AndV => 0x4e20_1c00,
            CodeOp::OrrV => 0x4ea0_1c00,
            CodeOp::EorV => 0x6e20_1c00,
            CodeOp::BslV => 0x6e60_1c00,
            CodeOp::BitV => 0x6ea0_1c00,
            other => return Err(format!("{} is not a three-same NEON op", other.mnemonic())),
        };
        self.emit_word(base | ((vm as u32) << 16) | ((vn as u32) << 5) | vd as u32)
    }

    /// Two-register-misc NEON ops: `op Vd.<T>, Vn.<T>` (the compare-zero forms
    /// compare each lane against 0.0/0 implicitly).
    fn emit_v_two_misc(&mut self, op: CodeOp, vd: u8, vn: u8) -> Result<(), String> {
        let base = match op {
            CodeOp::FAbsV => 0x4ee0_f800,
            CodeOp::FNegV => 0x6ee0_f800,
            CodeOp::FSqrtV => 0x6ee1_f800,
            CodeOp::FRintpV => 0x4ee1_8800,
            CodeOp::FRintmV => 0x4e61_9800,
            CodeOp::FRintaV => 0x6e61_8800,
            CodeOp::FRintnV => 0x4e61_8800,
            CodeOp::FRintzV => 0x4ee1_9800,
            CodeOp::FCvtzsV => 0x4ee1_b800,
            CodeOp::FCvtasV => 0x4e61_c800,
            CodeOp::ScvtfV => 0x4e61_d800,
            CodeOp::NegV => 0x6ee0_b800,
            CodeOp::AbsV => 0x4ee0_b800,
            CodeOp::FCmGtZeroV => 0x4ee0_c800,
            CodeOp::FCmGeZeroV => 0x6ee0_c800,
            CodeOp::FCmEqZeroV => 0x4ee0_d800,
            CodeOp::FCmLtZeroV => 0x4ee0_e800,
            CodeOp::FCmLeZeroV => 0x6ee0_d800,
            other => return Err(format!("{} is not a two-reg-misc NEON op", other.mnemonic())),
        };
        self.emit_word(base | ((vn as u32) << 5) | vd as u32)
    }

    /// Shifted-immediate NEON ops on `.2d` lanes (64-bit element). `shl` takes a
    /// left-shift 0..=63; `sshr`/`ushr` take a right-shift 1..=64.
    fn emit_v_shift_imm(&mut self, op: CodeOp, vd: u8, vn: u8, amount: u8) -> Result<(), String> {
        let (base, immhb) = match op {
            CodeOp::ShlV => {
                if amount > 63 {
                    return Err(format!("AArch64 shl.2d shift {amount} is out of range"));
                }
                (0x4f00_5400, 64 + amount as u32)
            }
            CodeOp::SshrV => {
                if amount == 0 || amount > 64 {
                    return Err(format!("AArch64 sshr.2d shift {amount} is out of range"));
                }
                (0x4f00_0400, 128 - amount as u32)
            }
            CodeOp::UshrV => {
                if amount == 0 || amount > 64 {
                    return Err(format!("AArch64 ushr.2d shift {amount} is out of range"));
                }
                (0x6f00_0400, 128 - amount as u32)
            }
            other => return Err(format!("{} is not a NEON shift-imm op", other.mnemonic())),
        };
        self.emit_word(base | (immhb << 16) | ((vn as u32) << 5) | vd as u32)
    }

    /// `DUP Vd.2d, Xn` — broadcast a 64-bit GPR into both lanes.
    fn emit_dup_v_from_x(&mut self, vd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x4e08_0c00 | ((rn as u32) << 5) | vd as u32)
    }

    /// `UMOV Xd, Vn.d[index]` — extract lane `index` (0 or 1) into a GPR.
    fn emit_umov_x_from_v(&mut self, rd: u8, vn: u8, index: u64) -> Result<(), String> {
        if index > 1 {
            return Err(format!("AArch64 umov .d lane index {index} is out of range"));
        }
        let imm5 = 0x8 | ((index as u32) << 4);
        self.emit_word(0x4e00_3c00 | (imm5 << 16) | ((vn as u32) << 5) | rd as u32)
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

    fn emit_umulh(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9bc0_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_adc(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9a00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_rorv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_2c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 32-bit `RORV Wd, Wn, Wm` — rotate right by `Wm mod 32`; zero-extends Wd.
    fn emit_rorv_w(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x1ac0_2c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `LSLV Xd, Xn, Xm` — logical shift left by `Xm mod 64`.
    fn emit_lslv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_2000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `LSRV Xd, Xn, Xm` — logical shift right by `Xm mod 64`.
    fn emit_lsrv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_2400 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `ASRV Xd, Xn, Xm` — arithmetic shift right by `Xm mod 64`.
    fn emit_asrv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_2800 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `CLZ Xd, Xn` — count leading zeros.
    fn emit_clz(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_1000 | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `RBIT Xd, Xn` — reverse bit order.
    fn emit_rbit(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_0000 | ((rn as u32) << 5) | rd as u32)
    }

    /// 32-bit `REV Wd, Wn` — reverse the four bytes of Wn; zero-extends Wd.
    fn emit_rev_w(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x5ac0_0800 | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `REV Xd, Xn` — reverse all eight bytes of Xn.
    fn emit_rev_x(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_0c00 | ((rn as u32) << 5) | rd as u32)
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

    fn emit_add_imm_chunk(
        &mut self,
        rd: u8,
        rn: u8,
        imm12: u32,
        shift12: bool,
    ) -> Result<(), String> {
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

    fn emit_sub_imm_chunk(
        &mut self,
        rd: u8,
        rn: u8,
        imm12: u32,
        shift12: bool,
    ) -> Result<(), String> {
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

    fn emit_fcmp_d(&mut self, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_2000 | ((dm as u32) << 16) | ((dn as u32) << 5))
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

    fn emit_str_u32(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 4 != 0 {
            return Err(format!("unaligned AArch64 str u32 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 4) {
            return self.emit_word(0xb900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xb900_0000 | ((scratch as u32) << 5) | rt as u32)
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
            return Ok(sized_add_sub_imm(immediate(field(
                instruction,
                if instruction.op == CodeOp::CmpImm {
                    "rhs"
                } else {
                    "imm"
                },
            )?)?));
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
        "sp" | "raw_sp" | "x31" | "xzr" => Ok(31),
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

/// Parse a NEON vector register operand. Accepts `v0`..`v31` and the `q0`..`q31`
/// load/store spelling (the arrangement suffix, e.g. `.2d`, is implied by the op,
/// so only the register number is decoded here).
fn vreg(name: String) -> Result<u8, String> {
    let digits = name
        .strip_prefix('v')
        .or_else(|| name.strip_prefix('q'))
        .ok_or_else(|| format!("unknown AArch64 vector register '{name}'"))?;
    let number = digits
        .parse::<u8>()
        .map_err(|_| format!("unknown AArch64 vector register '{name}'"))?;
    if number > 31 {
        return Err(format!("AArch64 vector register '{name}' out of range"));
    }
    Ok(number)
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
    fn encodes_umulh_adc_and_rorv() {
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
            .emit_instruction(
                &CodeInstruction::new("umulh")
                    .field("dst", "x14")
                    .field("lhs", "x11")
                    .field("rhs", "x9"),
            )
            .unwrap();
        encoder
            .emit_instruction(
                &CodeInstruction::new("adc")
                    .field("dst", "x10")
                    .field("lhs", "x14")
                    .field("rhs", "x12"),
            )
            .unwrap();
        encoder
            .emit_instruction(
                &CodeInstruction::new("rorv")
                    .field("dst", "x0")
                    .field("lhs", "x12")
                    .field("rhs", "x11"),
            )
            .unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&0x9bc9_7d6e_u32.to_le_bytes());
        expected.extend_from_slice(&0x9a0c_01ca_u32.to_le_bytes());
        expected.extend_from_slice(&0x9acb_2d80_u32.to_le_bytes());
        assert_eq!(encoder.text, expected);
    }

    fn fresh_encoder() -> Encoder {
        Encoder {
            text: Vec::new(),
            data: Vec::new(),
            symbols: Vec::new(),
            relocations: Vec::new(),
            imports: HashMap::new(),
            labels: HashMap::new(),
            patches: Vec::new(),
        }
    }

    fn encode_one(instruction: &CodeInstruction) -> u32 {
        let mut encoder = fresh_encoder();
        encoder.emit_instruction(instruction).unwrap();
        assert_eq!(encoder.text.len(), 4, "expected a single 4-byte word");
        u32::from_le_bytes(encoder.text[..4].try_into().unwrap())
    }

    /// Each new NEON op's encoding is checked against the exact little-endian word
    /// produced by the system assembler (`as -arch arm64`) for the same mnemonic
    /// and operands. Operands deliberately use distinct register numbers
    /// (dst=v5/x5, lhs/src=v9/x9, rhs=v17/x17) so a misplaced Rd/Rn/Rm bit-field
    /// is caught, not just a wrong base constant.
    #[test]
    fn encodes_neon_vector_ops() {
        let three = |op: &str| {
            CodeInstruction::new(op)
                .field("dst", "v5")
                .field("lhs", "v9")
                .field("rhs", "v17")
        };
        let two = |op: &str| CodeInstruction::new(op).field("dst", "v5").field("src", "v17");

        // Three-same .2d / .16b.
        assert_eq!(encode_one(&three("fadd_v")), 0x4e71_d525);
        assert_eq!(encode_one(&three("fsub_v")), 0x4ef1_d525);
        assert_eq!(encode_one(&three("fmul_v")), 0x6e71_dd25);
        assert_eq!(encode_one(&three("fdiv_v")), 0x6e71_fd25);
        assert_eq!(encode_one(&three("fmla_v")), 0x4e71_cd25);
        assert_eq!(encode_one(&three("fmls_v")), 0x4ef1_cd25);
        assert_eq!(encode_one(&three("fmin_v")), 0x4ef1_f525);
        assert_eq!(encode_one(&three("fmax_v")), 0x4e71_f525);
        assert_eq!(encode_one(&three("fcmgt_v")), 0x6ef1_e525);
        assert_eq!(encode_one(&three("fcmge_v")), 0x6e71_e525);
        assert_eq!(encode_one(&three("fcmeq_v")), 0x4e71_e525);
        assert_eq!(encode_one(&three("add_v")), 0x4ef1_8525);
        assert_eq!(encode_one(&three("sub_v")), 0x6ef1_8525);
        assert_eq!(encode_one(&three("cmgt_v")), 0x4ef1_3525);
        assert_eq!(encode_one(&three("cmge_v")), 0x4ef1_3d25);
        assert_eq!(encode_one(&three("cmeq_v")), 0x6ef1_8d25);
        assert_eq!(encode_one(&three("sshl_v")), 0x4ef1_4525);
        assert_eq!(encode_one(&three("ushl_v")), 0x6ef1_4525);
        assert_eq!(encode_one(&three("and_v")), 0x4e31_1d25);
        assert_eq!(encode_one(&three("orr_v")), 0x4eb1_1d25);
        assert_eq!(encode_one(&three("eor_v")), 0x6e31_1d25);
        assert_eq!(encode_one(&three("bsl_v")), 0x6e71_1d25);
        assert_eq!(encode_one(&three("bit_v")), 0x6eb1_1d25);

        // Two-reg-misc .2d.
        assert_eq!(encode_one(&two("fabs_v")), 0x4ee0_fa25);
        assert_eq!(encode_one(&two("fneg_v")), 0x6ee0_fa25);
        assert_eq!(encode_one(&two("fsqrt_v")), 0x6ee1_fa25);
        assert_eq!(encode_one(&two("frintp_v")), 0x4ee1_8a25);
        assert_eq!(encode_one(&two("frintm_v")), 0x4e61_9a25);
        assert_eq!(encode_one(&two("frinta_v")), 0x6e61_8a25);
        assert_eq!(encode_one(&two("frintn_v")), 0x4e61_8a25);
        assert_eq!(encode_one(&two("frintz_v")), 0x4ee1_9a25);
        assert_eq!(encode_one(&two("fcvtzs_v")), 0x4ee1_ba25);
        assert_eq!(encode_one(&two("fcvtas_v")), 0x4e61_ca25);
        assert_eq!(encode_one(&two("scvtf_v")), 0x4e61_da25);
        assert_eq!(encode_one(&two("neg_v")), 0x6ee0_ba25);
        assert_eq!(encode_one(&two("abs_v")), 0x4ee0_ba25);
        assert_eq!(encode_one(&two("fcmgt_zero_v")), 0x4ee0_ca25);
        assert_eq!(encode_one(&two("fcmge_zero_v")), 0x6ee0_ca25);
        assert_eq!(encode_one(&two("fcmeq_zero_v")), 0x4ee0_da25);
        assert_eq!(encode_one(&two("fcmlt_zero_v")), 0x4ee0_ea25);
        assert_eq!(encode_one(&two("fcmle_zero_v")), 0x6ee0_da25);

        // Shifted-immediate .2d.
        assert_eq!(
            encode_one(
                &CodeInstruction::new("shl_v")
                    .field("dst", "v5")
                    .field("src", "v17")
                    .field("shift", "32")
            ),
            0x4f60_5625
        );
        assert_eq!(
            encode_one(
                &CodeInstruction::new("sshr_v")
                    .field("dst", "v5")
                    .field("src", "v17")
                    .field("shift", "32")
            ),
            0x4f60_0625
        );
        assert_eq!(
            encode_one(
                &CodeInstruction::new("ushr_v")
                    .field("dst", "v5")
                    .field("src", "v17")
                    .field("shift", "20")
            ),
            0x6f6c_0625
        );

        // Lane broadcast / extract.
        assert_eq!(
            encode_one(
                &CodeInstruction::new("dup_v_from_x")
                    .field("dst", "v5")
                    .field("src", "x17")
            ),
            0x4e08_0e25
        );
        assert_eq!(
            encode_one(
                &CodeInstruction::new("umov_x_from_v")
                    .field("dst", "x5")
                    .field("src", "v17")
                    .field("index", "0")
            ),
            0x4e08_3e25
        );
        assert_eq!(
            encode_one(
                &CodeInstruction::new("umov_x_from_v")
                    .field("dst", "x5")
                    .field("src", "v17")
                    .field("index", "1")
            ),
            0x4e18_3e25
        );

        // 128-bit load/store, with and without an offset.
        assert_eq!(
            encode_one(
                &CodeInstruction::new("ldr_q")
                    .field("dst", "v5")
                    .field("base", "x9")
                    .field("offset", "0")
            ),
            0x3dc0_0125
        );
        assert_eq!(
            encode_one(
                &CodeInstruction::new("str_q")
                    .field("src", "v5")
                    .field("base", "x9")
                    .field("offset", "16")
            ),
            0x3d80_0525
        );
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
