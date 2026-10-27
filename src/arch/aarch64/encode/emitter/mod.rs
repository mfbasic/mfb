use super::operand::{
    field, immediate, is_stack_pointer, reg, scratch_excluding, shift, shifted_reg, vreg,
};
use super::sizing::{
    add_sub_chunk_count, branch_imm14, branch_imm19, branch_imm26, checked_imm12,
    encode_add_sub_imm, next_add_sub_chunk, MAX_ADD_SUB_CHUNKS,
};
use super::*;
use crate::codegen::engine::types::RelocIntent;

mod memory;
mod scalar;
mod vector;

// Load-Acquire / Store-Release, the "exclusive" encoding class
// `size 0010000 L 0 Rs=11111 o0=1 Rt2=11111 Rn Rt` (bug-564). `STLR` is L=0 and
// `LDAR` L=1; size 11 is the X form and 10 the W form. `Rn` is the base and 31
// there is SP. Pinned against clang's assembler in `encodes_load_acquire_store_release`.
const STLR_X: u32 = 0xc89f_fc00;
const LDAR_X: u32 = 0xc8df_fc00;
/// plan-183-A: `mrs Xt, DCZID_EL0` (op0 3, op1 3, CRn 0, CRm 0, op2 7).
const MRS_DCZID_EL0: u32 = 0xd53b_00e0;
/// plan-183-A: `dc zva, Xt` = `sys #3, C7, C4, #1, Xt`.
const DC_ZVA: u32 = 0xd50b_7420;
const LDAR_W: u32 = 0x88df_fc00;

pub(super) struct Encoder {
    pub(super) text: Vec<u8>,
    pub(super) data: Vec<u8>,
    pub(super) symbols: Vec<EncodedSymbol>,
    pub(super) relocations: Vec<EncodedRelocation>,
    pub(super) imports: HashMap<String, String>,
    pub(super) labels: HashMap<String, usize>,
    pub(super) patches: Vec<LabelPatch>,
}

pub(super) struct LabelPatch {
    offset: usize,
    target: String,
    kind: String,
}

impl Encoder {
    pub(super) fn emit_instruction(&mut self, instruction: &CodeInstruction) -> Result<(), String> {
        match instruction.op.mnemonic() {
            "label" => Ok(()),
            "mov" => self.emit_mov(&field(instruction, "dst")?, &field(instruction, "src")?),
            "mov_imm" => self.emit_mov_imm(
                reg(field(instruction, "dst")?)?,
                immediate(field(instruction, "value")?)?,
            ),
            "add" => self.emit_add(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "adds" => self.emit_adds(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "sub" => self.emit_sub(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "subs" => self.emit_subs(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "and" => self.emit_and(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "orr" => self.emit_orr(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "eor" => self.emit_eor(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "mvn" => self.emit_mvn(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "src")?)?,
            ),
            "mul" => self.emit_mul(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "smulh" => self.emit_smulh(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "umulh" => self.emit_umulh(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "add_carry" => self.emit_add_carry(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "carry_out")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
                reg(field(instruction, "carry_in")?)?,
            ),
            "sub_borrow" => self.emit_sub_borrow(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "borrow_out")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
                reg(field(instruction, "borrow_in")?)?,
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
            "sxtw" => self.emit_sxtw(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "sdiv" => self.emit_sdiv(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "udiv" => self.emit_udiv(
                shifted_reg(field(instruction, "dst")?)?,
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
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
            "cmp_imm" => {
                // The spelling, not just the number, has to reach the encoder:
                // `cmp_imm` picks its form from the immediate's magnitude, and only
                // the immediate form reads register 31 as SP (bug-284 C1).
                let lhs = field(instruction, "lhs")?;
                let lhs_is_sp = is_stack_pointer(&lhs);
                self.emit_cmp_imm(reg(lhs)?, immediate(field(instruction, "rhs")?)?, lhs_is_sp)
            }
            "cmp" => self.emit_cmp(
                shifted_reg(field(instruction, "lhs")?)?,
                shifted_reg(field(instruction, "rhs")?)?,
            ),
            "b.eq" => self.emit_label_branch("b.eq", field(instruction, "target")?.into_owned()),
            "b.ne" => self.emit_label_branch("b.ne", field(instruction, "target")?.into_owned()),
            "b.ge" => self.emit_label_branch("b.ge", field(instruction, "target")?.into_owned()),
            "b.lt" => self.emit_label_branch("b.lt", field(instruction, "target")?.into_owned()),
            "b.gt" => self.emit_label_branch("b.gt", field(instruction, "target")?.into_owned()),
            "b.le" => self.emit_label_branch("b.le", field(instruction, "target")?.into_owned()),
            "b.vc" => self.emit_label_branch("b.vc", field(instruction, "target")?.into_owned()),
            "b.vs" => self.emit_label_branch("b.vs", field(instruction, "target")?.into_owned()),
            "b.hi" => self.emit_label_branch("b.hi", field(instruction, "target")?.into_owned()),
            "b.lo" => self.emit_label_branch("b.lo", field(instruction, "target")?.into_owned()),
            "b.mi" => self.emit_label_branch("b.mi", field(instruction, "target")?.into_owned()),
            "b.ls" => self.emit_label_branch("b.ls", field(instruction, "target")?.into_owned()),
            "b" => self.emit_label_branch("b", field(instruction, "target")?.into_owned()),
            "cbz" | "cbnz" | "tbz" | "tbnz" => self.emit_zero_or_bit_branch(instruction),
            "bl" => self.emit_bl(field(instruction, "target")?.into_owned()),
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
            "str_u16" => self.emit_str_u16(
                reg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_u8" => self.emit_str_u8(
                reg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            // bug-564: load-acquire / store-release. No offset form exists in
            // A64, so the address is materialised by the caller.
            "stlr_u64" => self.emit_word(
                STLR_X
                    | (u32::from(reg(field(instruction, "base")?)?) << 5)
                    | u32::from(reg(field(instruction, "src")?)?),
            ),
            // plan-183-A: the arena zero fill's block-size probe and block zero.
            "mrs_dczid" => {
                self.emit_word(MRS_DCZID_EL0 | u32::from(reg(field(instruction, "dst")?)?))
            }
            "dc_zva" => self.emit_word(DC_ZVA | (u32::from(reg(field(instruction, "base")?)?))),
            "ldar_u64" => self.emit_word(
                LDAR_X
                    | (u32::from(reg(field(instruction, "base")?)?) << 5)
                    | u32::from(reg(field(instruction, "dst")?)?),
            ),
            "ldar_u32" => self.emit_word(
                LDAR_W
                    | (u32::from(reg(field(instruction, "base")?)?) << 5)
                    | u32::from(reg(field(instruction, "dst")?)?),
            ),
            // plan-171-A: the store/load pair, X or Q, all three addressing modes.
            "stp" => self.emit_pair(instruction, false),
            "ldp" => self.emit_pair(instruction, true),
            "ldr_d" => self.emit_ldr_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_d" => self.emit_str_d(
                reg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "adrp" => self.emit_symbol_ref(
                "adrp",
                reg(field(instruction, "dst")?)?,
                field(instruction, "symbol")?.into_owned(),
            ),
            "add_pageoff" => self.emit_symbol_ref(
                "add_pageoff",
                reg(field(instruction, "dst")?)?,
                field(instruction, "symbol")?.into_owned(),
            ),
            "fmov_x_from_d" => self.emit_fmov_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fmov_d_from_x" => self.emit_fmov_d_from_x(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fmov_d_from_d" => self.emit_fmov_d_from_d(
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
            "fminnm_d" => self.emit_fminnm_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fmaxnm_d" => self.emit_fmaxnm_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fneg_d" => self.emit_fneg_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fabs_d" => self.emit_fabs_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fsqrt_d" => self.emit_fsqrt_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            // FCVT Sd, Dn / FCVT Dd, Sn (FP data-processing, 1 source).
            "fcvt_s_from_d" => self.emit_rr(
                0x1e62_4000,
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvt_d_from_s" => self.emit_rr(
                0x1e22_c000,
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
            | "cmge_v" | "cmeq_v" | "sshl_v" | "ushl_v" | "and_v" | "orr_v" | "eor_v" | "bsl_v"
            | "bit_v" => self.emit_v_three_same(
                instruction.op,
                vreg(field(instruction, "dst")?)?,
                vreg(field(instruction, "lhs")?)?,
                vreg(field(instruction, "rhs")?)?,
            ),
            "fabs_v" | "fneg_v" | "fsqrt_v" | "frintp_v" | "frintm_v" | "frinta_v" | "frintn_v"
            | "frintz_v" | "fcvtzs_v" | "fcvtas_v" | "scvtf_v" | "neg_v" | "abs_v"
            | "fcmgt_zero_v" | "fcmge_zero_v" | "fcmeq_zero_v" | "fcmlt_zero_v"
            | "fcmle_zero_v" | "cnt8b_v" | "addv8b_v" => self.emit_v_two_misc(
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
            "fmadd_d" | "fmsub_d" | "fnmsub_d" | "fnmadd_d" => self.emit_fma_d(
                instruction.op,
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "addend")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            other => Err(format!(
                "AArch64 encoder does not support instruction '{other}'"
            )),
        }
    }

    pub(super) fn patch_labels(&mut self) -> Result<(), String> {
        for patch in &self.patches {
            let Some(&target) = self.labels.get(&patch.target) else {
                return Err(format!(
                    "AArch64 branch target label '{}' does not resolve",
                    patch.target
                ));
            };
            // Reject an out-of-range displacement instead of masking it to a
            // wrong target (bug-124): unconditional `b` reaches ±128 MiB (imm26
            // words), conditional `b.*` and `cbz`/`cbnz` only ±1 MiB (imm19
            // words), `tbz`/`tbnz` ±32 KiB (imm14 words, plan-171-B).
            let delta = target as isize - patch.offset as isize;
            let (limit, span) = match patch.kind.as_str() {
                "b" => (1_isize << 27, "±128 MiB"),
                "tbz" | "tbnz" => (1_isize << 15, "±32 KiB"),
                _ => (1_isize << 20, "±1 MiB"),
            };
            if delta < -limit || delta >= limit {
                return Err(format!(
                    "AArch64 branch '{}' displacement {delta} to '{}' exceeds {span}",
                    patch.kind, patch.target
                ));
            }
            let word = match patch.kind.as_str() {
                "b" => 0x1400_0000 | branch_imm26(patch.offset, target)?,
                "b.eq" => 0x5400_0000 | (branch_imm19(patch.offset, target)? << 5),
                "b.ne" => 0x5400_0001 | (branch_imm19(patch.offset, target)? << 5),
                "b.ge" => 0x5400_000a | (branch_imm19(patch.offset, target)? << 5),
                "b.lt" => 0x5400_000b | (branch_imm19(patch.offset, target)? << 5),
                "b.gt" => 0x5400_000c | (branch_imm19(patch.offset, target)? << 5),
                "b.le" => 0x5400_000d | (branch_imm19(patch.offset, target)? << 5),
                "b.vc" => 0x5400_0007 | (branch_imm19(patch.offset, target)? << 5),
                "b.vs" => 0x5400_0006 | (branch_imm19(patch.offset, target)? << 5),
                "b.hi" => 0x5400_0008 | (branch_imm19(patch.offset, target)? << 5),
                "b.lo" => 0x5400_0003 | (branch_imm19(patch.offset, target)? << 5),
                "b.mi" => 0x5400_0004 | (branch_imm19(patch.offset, target)? << 5),
                "b.ls" => 0x5400_0009 | (branch_imm19(patch.offset, target)? << 5),
                // plan-171-B: the register (and bit) were written at emission.
                "cbz" | "cbnz" => {
                    self.word_at(patch.offset) | (branch_imm19(patch.offset, target)? << 5)
                }
                "tbz" | "tbnz" => {
                    self.word_at(patch.offset) | (branch_imm14(patch.offset, target)? << 5)
                }
                other => return Err(format!("unknown AArch64 branch patch '{other}'")),
            };
            self.text[patch.offset..patch.offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        Ok(())
    }
}

impl Encoder {
    /// The word already emitted at `offset` (a label patch ORs into it).
    fn word_at(&self, offset: usize) -> u32 {
        u32::from_le_bytes(
            self.text[offset..offset + 4]
                .try_into()
                .expect("a patch offset names one emitted word"),
        )
    }
}

impl crate::arch::encode_plan::InstructionEncoder for Encoder {
    fn new(data: Vec<u8>, imports: HashMap<String, String>) -> Self {
        Encoder {
            text: Vec::new(),
            data,
            symbols: Vec::new(),
            relocations: Vec::new(),
            imports,
            labels: HashMap::new(),
            patches: Vec::new(),
        }
    }

    fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String> {
        super::sizing::instruction_size(instruction)
    }

    fn label_name(instruction: &CodeInstruction) -> Result<String, String> {
        super::operand::field(instruction, "name").map(|c| c.into_owned())
    }

    fn emit_one(&mut self, instruction: &CodeInstruction) -> Result<(), String> {
        self.emit_instruction(instruction)
    }

    fn resolve_patches(&mut self) -> Result<(), String> {
        self.patch_labels()
    }

    fn text_len(&self) -> usize {
        self.text.len()
    }

    fn reserve_text(&mut self, len: usize) {
        self.text.resize(len, 0);
    }

    fn truncate_text(&mut self, len: usize) {
        self.text.truncate(len);
    }

    fn push_symbol(&mut self, symbol: EncodedSymbol) {
        self.symbols.push(symbol);
    }

    fn insert_label(&mut self, name: String, offset: usize) -> Option<usize> {
        self.labels.insert(name, offset)
    }

    fn clear_labels(&mut self) {
        self.labels.clear();
    }

    fn clear_patches(&mut self) {
        self.patches.clear();
    }

    fn into_parts(self) -> (Vec<u8>, Vec<u8>, Vec<EncodedSymbol>, Vec<EncodedRelocation>) {
        (self.text, self.data, self.symbols, self.relocations)
    }
}
