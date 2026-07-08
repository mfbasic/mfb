//! RV64GC instruction emission (plan-99). Every method encodes one CodeOp into
//! little-endian 32-bit RISC-V words. The scratch registers `t0`/`t1`/`t2`
//! (x5/x6/x7) are reserved from allocation (`regmodel`) and are reused here for
//! the multi-instruction expansions (immediate materialization, out-of-range
//! memory offsets, `msub`, …); every expansion is self-contained, so a value in
//! `t0` never needs to survive to the next instruction.

use super::operand::{field, freg, immediate, reg, shift};
use super::sizing::{li_steps, LiStep};
use super::*;
use crate::target::shared::code::RelocIntent;

// Base opcodes.
const OP: u32 = 0x33;
const OP_IMM: u32 = 0x13;
const OP_32: u32 = 0x3b;
const OP_IMM_32: u32 = 0x1b;
const LOAD: u32 = 0x03;
const STORE: u32 = 0x23;
const LUI: u32 = 0x37;
const AUIPC: u32 = 0x17;
const JAL: u32 = 0x6f;
const JALR: u32 = 0x67;
const BRANCH: u32 = 0x63;
const SYSTEM: u32 = 0x73;
const LOAD_FP: u32 = 0x07;
const STORE_FP: u32 = 0x27;
const OP_FP: u32 = 0x53;
const MADD: u32 = 0x43;
const MSUB: u32 = 0x47;
const NMSUB: u32 = 0x4b;
const NMADD: u32 = 0x4f;

// Reserved lowering scratch (see module comment).
const T0: u8 = 5;
const T1: u8 = 6;
const T2: u8 = 7;
const ZERO: u8 = 0;
const RA: u8 = 1;
const FT0: u8 = 0;

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
    /// Byte offset of the `jal` word to patch.
    offset: usize,
    target: String,
}

// --- Instruction-format encoders (little-endian 32-bit words) ----------------

fn r_type(funct7: u32, rs2: u32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

fn i_type(imm: i32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    ((imm as u32 & 0xfff) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

fn s_type(imm: i32, rs2: u32, rs1: u32, funct3: u32, opcode: u32) -> u32 {
    let imm = imm as u32;
    let hi = (imm >> 5) & 0x7f;
    let lo = imm & 0x1f;
    (hi << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (lo << 7) | opcode
}

fn b_type(imm: i32, rs2: u32, rs1: u32, funct3: u32, opcode: u32) -> u32 {
    let imm = imm as u32;
    let b12 = (imm >> 12) & 0x1;
    let b11 = (imm >> 11) & 0x1;
    let b10_5 = (imm >> 5) & 0x3f;
    let b4_1 = (imm >> 1) & 0xf;
    (b12 << 31) | (b10_5 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (b4_1 << 8) | (b11 << 7) | opcode
}

fn u_type(imm20: u32, rd: u32, opcode: u32) -> u32 {
    ((imm20 & 0xfffff) << 12) | (rd << 7) | opcode
}

fn j_type(imm: i32, rd: u32, opcode: u32) -> u32 {
    let imm = imm as u32;
    let b20 = (imm >> 20) & 0x1;
    let b10_1 = (imm >> 1) & 0x3ff;
    let b11 = (imm >> 11) & 0x1;
    let b19_12 = (imm >> 12) & 0xff;
    (b20 << 31) | (b10_1 << 21) | (b11 << 20) | (b19_12 << 12) | (rd << 7) | opcode
}

impl Encoder {
    fn emit_word(&mut self, word: u32) -> Result<(), String> {
        self.text.extend_from_slice(&word.to_le_bytes());
        Ok(())
    }

    /// Materialize a 64-bit immediate into `rd` via the `li` sequence.
    fn emit_li(&mut self, rd: u8, value: u64) -> Result<(), String> {
        let rd = rd as u32;
        for step in li_steps(value as i64) {
            let word = match step {
                LiStep::Lui(hi20) => u_type(hi20, rd, LUI),
                LiStep::Addi(imm) => i_type(imm, ZERO as u32, 0, rd, OP_IMM),
                LiStep::Slli(sh) => i_type(sh as i32, rd, 1, rd, OP_IMM), // slli funct3=001, funct6=0
                LiStep::AddiFrom(imm) => i_type(imm, rd, 0, rd, OP_IMM),
            };
            self.emit_word(word)?;
        }
        Ok(())
    }

    pub(super) fn emit_instruction(&mut self, instruction: &CodeInstruction) -> Result<(), String> {
        let r = |name: &str| -> Result<u8, String> { reg(field(instruction, name)?) };
        let f = |name: &str| -> Result<u8, String> { freg(field(instruction, name)?) };
        let imm = |name: &str| -> Result<u64, String> { immediate(field(instruction, name)?) };
        match instruction.op.mnemonic() {
            "label" => Ok(()),
            "mov" => {
                // mv rd, rs → addi rd, rs, 0.
                let (rd, rs) = (r("dst")?, r("src")?);
                self.emit_word(i_type(0, rs as u32, 0, rd as u32, OP_IMM))
            }
            "mov_imm" => self.emit_li(r("dst")?, imm("value")?),
            "add" => self.emit_r(OP, 0b000, 0, r("dst")?, r("lhs")?, r("rhs")?),
            "sub" => self.emit_r(OP, 0b000, 0b0100000, r("dst")?, r("lhs")?, r("rhs")?),
            "and" => self.emit_r(OP, 0b111, 0, r("dst")?, r("lhs")?, r("rhs")?),
            "orr" => self.emit_r(OP, 0b110, 0, r("dst")?, r("lhs")?, r("rhs")?),
            "eor" => self.emit_r(OP, 0b100, 0, r("dst")?, r("lhs")?, r("rhs")?),
            "mul" => self.emit_r(OP, 0b000, 0b0000001, r("dst")?, r("lhs")?, r("rhs")?),
            "smulh" => self.emit_r(OP, 0b001, 0b0000001, r("dst")?, r("lhs")?, r("rhs")?), // mulh
            "umulh" => self.emit_r(OP, 0b011, 0b0000001, r("dst")?, r("lhs")?, r("rhs")?), // mulhu
            "sdiv" => self.emit_r(OP, 0b100, 0b0000001, r("dst")?, r("lhs")?, r("rhs")?),
            "udiv" => self.emit_r(OP, 0b101, 0b0000001, r("dst")?, r("lhs")?, r("rhs")?),
            "add_carry" => self.emit_add_carry(
                r("dst")?,
                r("carry_out")?,
                r("lhs")?,
                r("rhs")?,
                r("carry_in")?,
            ),
            "sub_borrow" => self.emit_sub_borrow(
                r("dst")?,
                r("borrow_out")?,
                r("lhs")?,
                r("rhs")?,
                r("borrow_in")?,
            ),
            "rv.slt" => self.emit_r(OP, 0b010, 0, r("dst")?, r("lhs")?, r("rhs")?),
            "rv.sltu" => self.emit_r(OP, 0b011, 0, r("dst")?, r("lhs")?, r("rhs")?),
            "lslv" => self.emit_r(OP, 0b001, 0, r("dst")?, r("lhs")?, r("rhs")?),
            "lsrv" => self.emit_r(OP, 0b101, 0, r("dst")?, r("lhs")?, r("rhs")?),
            "asrv" => self.emit_r(OP, 0b101, 0b0100000, r("dst")?, r("lhs")?, r("rhs")?),
            "rorv" => {
                // Base-ISA rotate-right (no Zbb): ror = (x >> s) | (x << (64-s)).
                // RISC-V shifts mask the amount to 6 bits, so `-s` gives (64-s)&63
                // and s==0 degenerates to the identity. Zbb `ror` is a later opt.
                let (dst, lhs, rhs) = (r("dst")?, r("lhs")?, r("rhs")?);
                self.emit_r(OP, 0b101, 0, T0, lhs, rhs)?; // srl t0, lhs, rhs
                self.emit_r(OP, 0b000, 0b0100000, T1, ZERO, rhs)?; // sub t1, zero, rhs
                self.emit_r(OP, 0b001, 0, T2, lhs, T1)?; // sll t2, lhs, t1
                self.emit_r(OP, 0b110, 0, dst, T0, T2) // or dst, t0, t2
            }
            "rorv_w" => {
                // 32-bit rotate-right, zero-extended: word shifts (srlw/sllw) then
                // zero-extend the low 32 bits.
                let (dst, lhs, rhs) = (r("dst")?, r("lhs")?, r("rhs")?);
                self.emit_r(OP_32, 0b101, 0, T0, lhs, rhs)?; // srlw t0, lhs, rhs
                self.emit_r(OP, 0b000, 0b0100000, T1, ZERO, rhs)?; // sub t1, zero, rhs
                self.emit_r(OP_32, 0b001, 0, T2, lhs, T1)?; // sllw t2, lhs, t1
                self.emit_r(OP, 0b110, 0, T0, T0, T2)?; // or t0, t0, t2
                self.emit_word(i_type(32, T0 as u32, 0b001, T0 as u32, OP_IMM))?; // slli t0, t0, 32
                self.emit_word(i_type(32, T0 as u32, 0b101, dst as u32, OP_IMM)) // srli dst, t0, 32
            }
            "mvn" => {
                // not rd, rs → xori rd, rs, -1.
                let (rd, rs) = (r("dst")?, r("src")?);
                self.emit_word(i_type(-1, rs as u32, 0b100, rd as u32, OP_IMM))
            }
            "sxtw" => {
                // sext.w rd, rs → addiw rd, rs, 0 (sign-extends the low 32 bits).
                let (rd, rs) = (r("dst")?, r("src")?);
                self.emit_word(i_type(0, rs as u32, 0, rd as u32, OP_IMM_32))
            }
            "msub" => {
                // rd = minuend - lhs*rhs → mul t0, lhs, rhs ; sub rd, minuend, t0.
                let (rd, lhs, rhs, minuend) =
                    (r("dst")?, r("lhs")?, r("rhs")?, r("minuend")?);
                self.emit_r(OP, 0b000, 0b0000001, T0, lhs, rhs)?;
                self.emit_r(OP, 0b000, 0b0100000, rd, minuend, T0)
            }
            "lsl_imm" => {
                let (rd, rs, sh) = (r("dst")?, r("src")?, shift(field(instruction, "shift")?)?);
                self.emit_word(i_type(sh as i32, rs as u32, 0b001, rd as u32, OP_IMM))
            }
            "lsr_imm" => {
                let (rd, rs, sh) = (r("dst")?, r("src")?, shift(field(instruction, "shift")?)?);
                self.emit_word(i_type(sh as i32, rs as u32, 0b101, rd as u32, OP_IMM))
            }
            "asr_imm" => {
                let (rd, rs, sh) = (r("dst")?, r("src")?, shift(field(instruction, "shift")?)?);
                // srai: funct6 = 010000 in the top of the shamt field.
                self.emit_word(i_type((0b0100000 << 5) | sh as i32, rs as u32, 0b101, rd as u32, OP_IMM))
            }
            // Base-ISA bit manipulation (no Zbb): parallel masked swaps, and a
            // SWAR popcount of the down-smeared value for `clz` (plan-99).
            "clz" => self.emit_clz(r("dst")?, r("src")?),
            "rbit" => self.emit_reversal(r("dst")?, r("src")?, super::sizing::RBIT_LEVELS),
            "rev_x" => self.emit_reversal(r("dst")?, r("src")?, super::sizing::REV_X_LEVELS),
            "rev_w" => self.emit_rev_w(r("dst")?, r("src")?),
            "add_imm" => self.emit_add_imm(r("dst")?, r("src")?, imm("imm")?),
            "sub_imm" => self.emit_sub_imm(r("dst")?, r("src")?, imm("imm")?),
            "sub_sp" => self.emit_sub_imm(2, 2, imm("imm")?),
            "add_sp" => self.emit_add_imm(2, 2, imm("imm")?),
            "ldr_u64" => self.emit_load(0b011, r("dst")?, r("base")?, imm("offset")?),
            "ldr_u32" => self.emit_load(0b110, r("dst")?, r("base")?, imm("offset")?), // lwu
            "ldr_u16" => self.emit_load(0b101, r("dst")?, r("base")?, imm("offset")?), // lhu
            "ldr_u8" => self.emit_load(0b100, r("dst")?, r("base")?, imm("offset")?),  // lbu
            "str_u64" => self.emit_store(0b011, r("src")?, r("base")?, imm("offset")?),
            "str_u32" => self.emit_store(0b010, r("src")?, r("base")?, imm("offset")?),
            "str_u8" => self.emit_store(0b000, r("src")?, r("base")?, imm("offset")?),
            "ldr_d" => self.emit_load_fp(0b011, f("dst")?, r("base")?, imm("offset")?),
            "str_d" => self.emit_store_fp(0b011, f("src")?, r("base")?, imm("offset")?),
            "b" => self.emit_jal_label(ZERO, field(instruction, "target")?),
            "bl" => self.emit_call(field(instruction, "target")?),
            "blr" => {
                // jalr ra, 0(rs).
                let rs = r("register")?;
                self.emit_word(i_type(0, rs as u32, 0, RA as u32, JALR))
            }
            "branch_self" => self.emit_word(j_type(0, ZERO as u32, JAL)),
            "svc" => self.emit_word(SYSTEM), // ecall (imm=0, funct3=0)
            "ret" => self.emit_word(i_type(0, RA as u32, 0, ZERO as u32, JALR)),
            "rv.br" => self.emit_rv_br(instruction),
            "adrp" => self.emit_auipc_ref(r("dst")?, field(instruction, "symbol")?),
            "add_pageoff" => self.emit_pageoff(r("dst")?, field(instruction, "symbol")?),
            // --- FP scalar (Phase 2) ---
            "fadd_d" => self.emit_fp_r(0b0000001, f("dst")?, f("lhs")?, f("rhs")?, 0b000),
            "fsub_d" => self.emit_fp_r(0b0000101, f("dst")?, f("lhs")?, f("rhs")?, 0b000),
            "fmul_d" => self.emit_fp_r(0b0001001, f("dst")?, f("lhs")?, f("rhs")?, 0b000),
            "fdiv_d" => self.emit_fp_r(0b0001101, f("dst")?, f("lhs")?, f("rhs")?, 0b000),
            // fmin.d / fmax.d — funct7=0010101, funct3 selects min(000)/max(001).
            // RISC-V fmin.d/fmax.d implement the IEEE number semantics (a finite
            // operand wins over a NaN), matching AArch64 `fminnm`/`fmaxnm`.
            "fminnm_d" => self.emit_fp_r(0b0010101, f("dst")?, f("lhs")?, f("rhs")?, 0b000),
            "fmaxnm_d" => self.emit_fp_r(0b0010101, f("dst")?, f("lhs")?, f("rhs")?, 0b001),
            "fmov_d_from_d" => {
                // fmv.d rd, rs → fsgnj.d rd, rs, rs.
                let (rd, rs) = (f("dst")?, f("src")?);
                self.emit_fp_r(0b0010001, rd, rs, rs, 0b000)
            }
            "fneg_d" => {
                // fneg.d rd, rs → fsgnjn.d rd, rs, rs.
                let (rd, rs) = (f("dst")?, f("src")?);
                self.emit_fp_r(0b0010001, rd, rs, rs, 0b001)
            }
            "fabs_d" => {
                // fabs.d rd, rs → fsgnjx.d rd, rs, rs.
                let (rd, rs) = (f("dst")?, f("src")?);
                self.emit_fp_r(0b0010001, rd, rs, rs, 0b010)
            }
            "fsqrt_d" => {
                let (rd, rs) = (f("dst")?, f("src")?);
                // fsqrt.d: funct7=0101101, rs2=0, rm=000 (RNE).
                self.emit_word(r_type(0b0101101, 0, rs as u32, 0b000, rd as u32, OP_FP))
            }
            "fmov_x_from_d" => {
                // fmv.x.d rd(gpr), rs(fp): funct7=1110001, rs2=0, funct3=000.
                let (rd, rs) = (r("dst")?, f("src")?);
                self.emit_word(r_type(0b1110001, 0, rs as u32, 0b000, rd as u32, OP_FP))
            }
            "fmov_d_from_x" => {
                // fmv.d.x rd(fp), rs(gpr): funct7=1111001, rs2=0, funct3=000.
                let (rd, rs) = (f("dst")?, r("src")?);
                self.emit_word(r_type(0b1111001, 0, rs as u32, 0b000, rd as u32, OP_FP))
            }
            "scvtf_d_from_x" => {
                // fcvt.d.l rd(fp), rs(gpr): funct7=1101001, rs2=00010(L), rm=000.
                let (rd, rs) = (f("dst")?, r("src")?);
                self.emit_word(r_type(0b1101001, 0b00010, rs as u32, 0b000, rd as u32, OP_FP))
            }
            "fcvtzs_x_from_d" => self.emit_fcvt_l_d(r("dst")?, f("src")?, 0b001), // RTZ (toward zero)
            "fcvtms_x_from_d" => self.emit_fcvt_l_d(r("dst")?, f("src")?, 0b010), // RDN (toward -inf)
            "fcvtps_x_from_d" => self.emit_fcvt_l_d(r("dst")?, f("src")?, 0b011), // RUP (toward +inf)
            "fcvtas_x_from_d" => self.emit_fcvt_l_d(r("dst")?, f("src")?, 0b100), // RMM (nearest ties away)
            "rv.fcmp" => {
                let (rd, lhs, rhs) = (r("dst")?, f("lhs")?, f("rhs")?);
                let funct3 = match field(instruction, "cmp")?.as_str() {
                    "eq" => 0b010, // feq.d
                    "lt" => 0b001, // flt.d
                    "le" => 0b000, // fle.d
                    other => return Err(format!("rv64 unknown fcmp kind '{other}'")),
                };
                self.emit_word(r_type(0b1010001, rhs as u32, lhs as u32, funct3, rd as u32, OP_FP))
            }
            // Scalar fused multiply-add family. RISC-V's native fmadd/fmsub/
            // fnmsub/fnmadd.d follow the same result naming as our neutral MIR ops
            // (rs1=lhs, rs2=rhs, rs3=addend), so the mapping is 1:1 by mnemonic:
            //   fmadd_d  → MADD   rs1*rs2 + rs3 = lhs*rhs + addend
            //   fmsub_d  → MSUB   rs1*rs2 - rs3 = lhs*rhs - addend
            //   fnmsub_d → NMSUB  -(rs1*rs2) + rs3 = addend - lhs*rhs
            //   fnmadd_d → NMADD  -(rs1*rs2) - rs3 = -(lhs*rhs) - addend
            "fmadd_d" | "fmsub_d" | "fnmsub_d" | "fnmadd_d" => {
                let opcode = match instruction.op.mnemonic() {
                    "fmadd_d" => MADD,
                    "fmsub_d" => MSUB,
                    "fnmsub_d" => NMSUB,
                    _ => NMADD,
                };
                let (rd, addend, lhs, rhs) = (f("dst")?, f("addend")?, f("lhs")?, f("rhs")?);
                // R4-type: rs3<<27 | fmt(D=01)<<25 | rs2<<20 | rs1<<15 | rm<<12 | rd<<7 | opcode.
                self.emit_word(
                    ((addend as u32) << 27)
                        | (0b01 << 25)
                        | ((rhs as u32) << 20)
                        | ((lhs as u32) << 15)
                        | (0b000 << 12)
                        | ((rd as u32) << 7)
                        | opcode,
                )
            }
            other => Err(format!(
                "rv64 encoder does not yet support instruction '{other}' (deferred to a later phase)"
            )),
        }
    }

    fn emit_r(&mut self, opcode: u32, funct3: u32, funct7: u32, rd: u8, rs1: u8, rs2: u8) -> Result<(), String> {
        self.emit_word(r_type(funct7, rs2 as u32, rs1 as u32, funct3, rd as u32, opcode))
    }

    /// `slli`/`srli rd, rs, shift` (logical shift by a constant).
    fn emit_shift_imm(&mut self, rd: u8, rs: u8, shift: u32, left: bool) -> Result<(), String> {
        let funct3 = if left { 0b001 } else { 0b101 };
        self.emit_word(i_type(shift as i32, rs as u32, funct3, rd as u32, OP_IMM))
    }

    /// One parallel-swap level in `T0`, with `T1`/`T2` as scratch:
    /// `T0 = ((T0 & mask) << shift) | ((T0 >> shift) & mask)`.
    fn emit_swap_level(&mut self, shift: u32, mask: u64) -> Result<(), String> {
        self.emit_li(T2, mask)?; // li   t2, mask
        self.emit_shift_imm(T1, T0, shift, false)?; // srli t1, t0, shift
        self.emit_r(OP, 0b111, 0, T1, T1, T2)?; // and  t1, t1, t2
        self.emit_r(OP, 0b111, 0, T0, T0, T2)?; // and  t0, t0, t2
        self.emit_shift_imm(T0, T0, shift, true)?; // slli t0, t0, shift
        self.emit_r(OP, 0b110, 0, T0, T0, T1) // or   t0, t0, t1
    }

    /// Swap the two 32-bit halves of `T0` in place (`T0 = (T0 << 32) | (T0 >> 32)`),
    /// the final level shared by `rev_x`/`rbit` (no mask needed).
    fn emit_swap_halves(&mut self, rd: u8) -> Result<(), String> {
        self.emit_shift_imm(T1, T0, 32, false)?; // srli t1, t0, 32
        self.emit_shift_imm(T0, T0, 32, true)?; // slli t0, t0, 32
        self.emit_r(OP, 0b110, 0, rd, T0, T1) // or   rd, t0, t1
    }

    /// `rev_x`/`rbit`: reverse bytes / bits of a 64-bit value via parallel masked
    /// swaps at each granularity, finishing with the 32-bit half swap. `src` is
    /// copied to `T0` first so `rd == src` is safe.
    fn emit_reversal(&mut self, rd: u8, src: u8, levels: &[(u32, u64)]) -> Result<(), String> {
        self.emit_word(i_type(0, src as u32, 0, T0 as u32, OP_IMM))?; // mv t0, src
        for &(shift, mask) in levels {
            self.emit_swap_level(shift, mask)?;
        }
        self.emit_swap_halves(rd)
    }

    /// `rev_w`: reverse the bytes of the low 32 bits, zero-extending the result.
    fn emit_rev_w(&mut self, rd: u8, src: u8) -> Result<(), String> {
        self.emit_shift_imm(T0, src, 32, true)?; // slli t0, src, 32  \ zero-extend
        self.emit_shift_imm(T0, T0, 32, false)?; // srli t0, t0, 32   / low 32 bits
        self.emit_swap_level(8, super::sizing::REV_W_MASK)?; // swap adjacent bytes
        self.emit_shift_imm(T1, T0, 16, true)?; // slli t1, t0, 16    \ swap the two
        self.emit_shift_imm(T0, T0, 16, false)?; // srli t0, t0, 16   / 16-bit halves
        self.emit_r(OP, 0b110, 0, T0, T0, T1)?; // or   t0, t0, t1
        self.emit_shift_imm(T0, T0, 32, true)?; // slli t0, t0, 32    \ zero-extend
        self.emit_shift_imm(rd, T0, 32, false) // srli rd, t0, 32     / to 32 bits
    }

    /// `clz`: count leading zeros of a 64-bit value. Smear the highest set bit
    /// down to bit 0, then `64 - popcount` counts the leading zeros (`64` when the
    /// input is zero). Runs entirely in `T0`/`T1`/`T2`, writing `rd` only at the
    /// end so `rd == src` is safe.
    fn emit_clz(&mut self, rd: u8, src: u8) -> Result<(), String> {
        use super::sizing::CLZ_POPCOUNT_MASKS as M;
        self.emit_word(i_type(0, src as u32, 0, T0 as u32, OP_IMM))?; // mv t0, src
        for sh in [1u32, 2, 4, 8, 16, 32] {
            self.emit_shift_imm(T1, T0, sh, false)?; // srli t1, t0, sh
            self.emit_r(OP, 0b110, 0, T0, T0, T1)?; // or   t0, t0, t1
        }
        // popcount(T0) — SWAR, result in T0.
        self.emit_li(T2, M[0])?; // t0 = t0 - ((t0>>1) & 0x5555…)
        self.emit_shift_imm(T1, T0, 1, false)?;
        self.emit_r(OP, 0b111, 0, T1, T1, T2)?;
        self.emit_r(OP, 0b000, 0b0100000, T0, T0, T1)?;
        self.emit_li(T2, M[1])?; // t0 = (t0 & 0x3333…) + ((t0>>2) & 0x3333…)
        self.emit_r(OP, 0b111, 0, T1, T0, T2)?;
        self.emit_shift_imm(T0, T0, 2, false)?;
        self.emit_r(OP, 0b111, 0, T0, T0, T2)?;
        self.emit_r(OP, 0b000, 0, T0, T0, T1)?;
        self.emit_shift_imm(T1, T0, 4, false)?; // t0 = (t0 + (t0>>4)) & 0x0F0F…
        self.emit_r(OP, 0b000, 0, T0, T0, T1)?;
        self.emit_li(T2, M[2])?;
        self.emit_r(OP, 0b111, 0, T0, T0, T2)?;
        self.emit_li(T2, M[3])?; // t0 = (t0 * 0x0101…) >> 56
        self.emit_r(OP, 0b000, 0b0000001, T0, T0, T2)?;
        self.emit_shift_imm(T0, T0, 56, false)?;
        self.emit_li(T1, 64)?; // clz = 64 - popcount
        self.emit_r(OP, 0b000, 0b0100000, rd, T1, T0)
    }

    /// FP three-register op (double): `funct7` selects the operation, `rm` the
    /// rounding mode (in funct3).
    fn emit_fp_r(&mut self, funct7: u32, rd: u8, rs1: u8, rs2: u8, rm: u32) -> Result<(), String> {
        self.emit_word(r_type(funct7, rs2 as u32, rs1 as u32, rm, rd as u32, OP_FP))
    }

    /// `fcvt.l.d rd(gpr), rs(fp)` with rounding mode `rm`: funct7=1100001,
    /// rs2=00010 (L = signed 64-bit).
    fn emit_fcvt_l_d(&mut self, rd: u8, rs: u8, rm: u32) -> Result<(), String> {
        self.emit_word(r_type(0b1100001, 0b00010, rs as u32, rm, rd as u32, OP_FP))
    }

    /// Explicit-carry add (plan-00-G §4): `dst = lhs + rhs + carry_in`,
    /// `carry_out` = the unsigned carry as a value. RISC-V has no carry flag, so
    /// the carry is computed from `sltu` comparisons. A fixed 7-instruction
    /// sequence (correct even when `carry_in`/`carry_out` are `zero`, so the size
    /// is deterministic):
    ///   add  t0, lhs, rhs          ; sum1
    ///   sltu t1, t0, lhs           ; c1 = sum1 < lhs (carry of first add)
    ///   add  t2, t0, carry_in      ; sum = sum1 + carry_in
    ///   sltu t0, t2, t0            ; c2 = sum < sum1 (carry of adding carry_in)
    ///   or   t1, t1, t0            ; carry_out = c1 | c2
    ///   mv   dst, t2 ; mv carry_out, t1
    fn emit_add_carry(
        &mut self,
        dst: u8,
        carry_out: u8,
        lhs: u8,
        rhs: u8,
        carry_in: u8,
    ) -> Result<(), String> {
        self.emit_r(OP, 0b000, 0, T0, lhs, rhs)?;
        self.emit_r(OP, 0b011, 0, T1, T0, lhs)?;
        self.emit_r(OP, 0b000, 0, T2, T0, carry_in)?;
        self.emit_r(OP, 0b011, 0, T0, T2, T0)?;
        self.emit_r(OP, 0b110, 0, T1, T1, T0)?;
        self.emit_word(i_type(0, T2 as u32, 0, dst as u32, OP_IMM))?;
        self.emit_word(i_type(0, T1 as u32, 0, carry_out as u32, OP_IMM))
    }

    /// Explicit-borrow subtract (plan-00-G §4): `dst = lhs - rhs - borrow_in`,
    /// `borrow_out` = the borrow as a value. Mirror of [`Self::emit_add_carry`]:
    ///   sub  t0, lhs, rhs          ; diff1
    ///   sltu t1, lhs, rhs          ; b1 = lhs < rhs (borrow of first sub)
    ///   sltu t2, t0, borrow_in     ; b2 = diff1 < borrow_in
    ///   sub  t0, t0, borrow_in     ; diff = diff1 - borrow_in
    ///   or   t1, t1, t2            ; borrow_out = b1 | b2
    ///   mv   dst, t0 ; mv borrow_out, t1
    fn emit_sub_borrow(
        &mut self,
        dst: u8,
        borrow_out: u8,
        lhs: u8,
        rhs: u8,
        borrow_in: u8,
    ) -> Result<(), String> {
        self.emit_r(OP, 0b000, 0b0100000, T0, lhs, rhs)?;
        self.emit_r(OP, 0b011, 0, T1, lhs, rhs)?;
        self.emit_r(OP, 0b011, 0, T2, T0, borrow_in)?;
        self.emit_r(OP, 0b000, 0b0100000, T0, T0, borrow_in)?;
        self.emit_r(OP, 0b110, 0, T1, T1, T2)?;
        self.emit_word(i_type(0, T0 as u32, 0, dst as u32, OP_IMM))?;
        self.emit_word(i_type(0, T1 as u32, 0, borrow_out as u32, OP_IMM))
    }

    fn emit_add_imm(&mut self, rd: u8, rs: u8, value: u64) -> Result<(), String> {
        if value <= 2047 {
            return self.emit_word(i_type(value as i32, rs as u32, 0, rd as u32, OP_IMM));
        }
        self.emit_li(T0, value)?;
        self.emit_r(OP, 0b000, 0, rd, rs, T0)
    }

    fn emit_sub_imm(&mut self, rd: u8, rs: u8, value: u64) -> Result<(), String> {
        // addi rd, rs, -value when -value fits the 12-bit signed field.
        if value <= 2048 {
            return self.emit_word(i_type(-(value as i32), rs as u32, 0, rd as u32, OP_IMM));
        }
        self.emit_li(T0, value)?;
        self.emit_r(OP, 0b000, 0b0100000, rd, rs, T0)
    }

    fn emit_load(&mut self, funct3: u32, rd: u8, base: u8, offset: u64) -> Result<(), String> {
        if offset <= 2047 {
            return self.emit_word(i_type(offset as i32, base as u32, funct3, rd as u32, LOAD));
        }
        // Materialize the address in `rd` itself (it is overwritten by the load),
        // never in `t0` — a large frame's spill/reload can land amid a scalarized
        // `v128` sequence that holds live lanes in the `t0`/`t1` scratch.
        self.emit_li(rd, offset)?;
        self.emit_r(OP, 0b000, 0, rd, base, rd)?; // add rd, base, rd
        self.emit_word(i_type(0, rd as u32, funct3, rd as u32, LOAD))
    }

    fn emit_store(&mut self, funct3: u32, src: u8, base: u8, offset: u64) -> Result<(), String> {
        if offset <= 2047 {
            return self.emit_word(s_type(offset as i32, src as u32, base as u32, funct3, STORE));
        }
        self.emit_li(T0, offset)?;
        self.emit_r(OP, 0b000, 0, T0, base, T0)?;
        self.emit_word(s_type(0, src as u32, T0 as u32, funct3, STORE))
    }

    fn emit_load_fp(&mut self, funct3: u32, rd: u8, base: u8, offset: u64) -> Result<(), String> {
        if offset <= 2047 {
            return self.emit_word(i_type(offset as i32, base as u32, funct3, rd as u32, LOAD_FP));
        }
        self.emit_li(T0, offset)?;
        self.emit_r(OP, 0b000, 0, T0, base, T0)?;
        self.emit_word(i_type(0, T0 as u32, funct3, rd as u32, LOAD_FP))
    }

    fn emit_store_fp(&mut self, funct3: u32, src: u8, base: u8, offset: u64) -> Result<(), String> {
        if offset <= 2047 {
            return self.emit_word(s_type(offset as i32, src as u32, base as u32, funct3, STORE_FP));
        }
        self.emit_li(T0, offset)?;
        self.emit_r(OP, 0b000, 0, T0, base, T0)?;
        self.emit_word(s_type(0, src as u32, T0 as u32, funct3, STORE_FP))
    }

    /// `rv.br` — the flagless compare-and-branch, always emitted in the 8-byte
    /// long form so its size is deterministic and it reaches ±1 MiB: an inverted
    /// conditional branch over an unconditional `jal` to the target.
    fn emit_rv_br(&mut self, instruction: &CodeInstruction) -> Result<(), String> {
        let rs1 = reg(field(instruction, "lhs")?)?;
        let rs2 = reg(field(instruction, "rhs")?)?;
        let cond = field(instruction, "cond")?;
        let target = field(instruction, "target")?;
        // Inverted-condition funct3 (so the short branch skips the jal when the
        // real condition is false).
        let inv_funct3 = match cond.as_str() {
            "eq" => 0b001, // bne
            "ne" => 0b000, // beq
            "lt" => 0b101, // bge
            "ge" => 0b100, // blt
            "ltu" => 0b111, // bgeu
            "geu" => 0b110, // bltu
            other => return Err(format!("rv64 unknown branch condition '{other}'")),
        };
        // b<inv> rs1, rs2, +8 (skip the jal).
        self.emit_word(b_type(8, rs2 as u32, rs1 as u32, inv_funct3, BRANCH))?;
        // jal zero, target (patched later).
        self.emit_jal_label(ZERO, target)
    }

    /// Emit a `jal rd, <label>` whose displacement is patched once label offsets
    /// are known.
    fn emit_jal_label(&mut self, rd: u8, target: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(j_type(0, rd as u32, JAL))?;
        self.patches.push(LabelPatch { offset, target });
        Ok(())
    }

    /// `bl <symbol>` → the `call` pseudo: `auipc ra, %pcrel_hi(sym); jalr ra,
    /// %pcrel_lo(sym)(ra)`. One `riscv_call` relocation at the `auipc`; the linker
    /// patches both words.
    fn emit_call(&mut self, target: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(u_type(0, RA as u32, AUIPC))?;
        self.emit_word(i_type(0, RA as u32, 0, RA as u32, JALR))?;
        let call_kind = crate::arch::riscv64::reloc::reloc_kind(RelocIntent::Call).to_string();
        if self.symbols.iter().any(|symbol| symbol.name == target) {
            self.relocations.push(EncodedRelocation {
                offset,
                target,
                kind: call_kind,
                binding: "internal".to_string(),
                library: None,
            });
        } else if let Some(library) = self.imports.get(&target) {
            self.relocations.push(EncodedRelocation {
                offset,
                target,
                kind: call_kind,
                binding: "external".to_string(),
                library: Some(library.clone()),
            });
        } else {
            return Err(format!("rv64 call target symbol '{target}' does not resolve"));
        }
        Ok(())
    }

    /// `auipc rd, %pcrel_hi(sym)` (or `%got_pcrel_hi` for an imported symbol) —
    /// the high half of a PC-relative address. Records the hi20 relocation.
    fn emit_auipc_ref(&mut self, rd: u8, symbol: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(u_type(0, rd as u32, AUIPC))?;
        if let Some(library) = self.imports.get(&symbol) {
            self.relocations.push(EncodedRelocation {
                offset,
                target: symbol,
                kind: crate::arch::riscv64::reloc::reloc_kind(RelocIntent::GotLoadHi).to_string(),
                binding: "external".to_string(),
                library: Some(library.clone()),
            });
        } else {
            self.relocations.push(EncodedRelocation {
                offset,
                target: symbol,
                kind: crate::arch::riscv64::reloc::reloc_kind(RelocIntent::DataAddrHi).to_string(),
                binding: "data".to_string(),
                library: None,
            });
        }
        Ok(())
    }

    /// The low half of a PC-relative address. For an internal symbol this is
    /// `addi rd, rd, %pcrel_lo(sym)`; for an imported (GOT) symbol it is a load
    /// `ld rd, %pcrel_lo(sym)(rd)` (the GOT slot holds the resolved address).
    fn emit_pageoff(&mut self, rd: u8, symbol: String) -> Result<(), String> {
        let offset = self.text.len();
        let imported = self.imports.contains_key(&symbol);
        if imported {
            // ld rd, 0(rd) — funct3=011, opcode LOAD.
            self.emit_word(i_type(0, rd as u32, 0b011, rd as u32, LOAD))?;
            let library = self.imports.get(&symbol).cloned();
            self.relocations.push(EncodedRelocation {
                offset,
                target: symbol,
                kind: crate::arch::riscv64::reloc::reloc_kind(RelocIntent::GotLoadLo).to_string(),
                binding: "external".to_string(),
                library,
            });
        } else {
            // addi rd, rd, 0.
            self.emit_word(i_type(0, rd as u32, 0, rd as u32, OP_IMM))?;
            self.relocations.push(EncodedRelocation {
                offset,
                target: symbol,
                kind: crate::arch::riscv64::reloc::reloc_kind(RelocIntent::DataAddrLo).to_string(),
                binding: "data".to_string(),
                library: None,
            });
        }
        Ok(())
    }

    pub(super) fn patch_labels(&mut self) -> Result<(), String> {
        for patch in &self.patches {
            let Some(&target) = self.labels.get(&patch.target) else {
                return Err(format!("rv64 branch target label '{}' does not resolve", patch.target));
            };
            let delta = target as i64 - patch.offset as i64;
            if delta < -(1 << 20) || delta >= (1 << 20) {
                return Err(format!(
                    "rv64 jal displacement {delta} to '{}' exceeds ±1 MiB",
                    patch.target
                ));
            }
            // Preserve the rd field already in the word; re-encode the J immediate.
            let existing = u32::from_le_bytes(
                self.text[patch.offset..patch.offset + 4]
                    .try_into()
                    .expect("slice length"),
            );
            let rd = (existing >> 7) & 0x1f;
            let word = j_type(delta as i32, rd, JAL);
            self.text[patch.offset..patch.offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        Ok(())
    }
}

// Scratch registers reserved for later phases (referenced to keep them named).
const _: (u8, u8, u8) = (T1, T2, FT0);
