use crate::target::shared::code::CodeInstruction;

pub(crate) const RETURN_REGISTER: &str = "x0";
pub(crate) const IO_PRINT_CLOBBERS: &[&str] = &["x0", "x1", "x2", "x9", "x16"];

pub(crate) fn argument_register(index: usize) -> Result<String, String> {
    if index < 8 {
        Ok(format!("x{index}"))
    } else {
        Err(format!(
            "aarch64 code plan cannot pass argument {index}; stack arguments are not implemented"
        ))
    }
}

pub(crate) fn temporary_register(allocation: usize) -> Result<String, String> {
    let register = match allocation {
        8..=17 => format!("x{allocation}"),
        18 => "x20".to_string(),
        19 => "x21".to_string(),
        20 => "x22".to_string(),
        21 => "x23".to_string(),
        22 => "x24".to_string(),
        23 => "x25".to_string(),
        24 => "x26".to_string(),
        25 => "x27".to_string(),
        26 => "x28".to_string(),
        other => {
            return Err(format!(
                "aarch64 code plan exhausted physical registers at allocation {other}"
            ));
        }
    };
    Ok(register)
}

/// The eager FP temporary register for the `bump` strategy: `d0`–`d7`, restarting
/// each statement (plan-03 Stage C). The linear-scan default colors FP virtual
/// registers by liveness and never uses this.
pub(crate) fn fp_temporary_register(allocation: usize) -> Result<String, String> {
    if allocation <= 7 {
        Ok(format!("d{allocation}"))
    } else {
        Err(format!(
            "aarch64 code plan exhausted FP temporary registers at allocation {allocation}"
        ))
    }
}

pub(crate) fn return_register() -> &'static str {
    RETURN_REGISTER
}

pub(crate) fn link_register() -> &'static str {
    "x30"
}

pub(crate) fn stack_pointer() -> &'static str {
    "sp"
}

pub(crate) fn raw_stack_pointer() -> &'static str {
    "raw_sp"
}

pub(crate) fn syscall_register() -> &'static str {
    "x8"
}

pub(crate) fn string_length_register() -> &'static str {
    "x2"
}

pub(crate) fn string_data_register() -> &'static str {
    "x1"
}

pub(crate) fn newline_scratch_register() -> &'static str {
    "x9"
}

pub(crate) fn is_callee_saved(register: &str) -> bool {
    matches!(
        register,
        "x19" | "x20" | "x21" | "x22" | "x23" | "x24" | "x25" | "x26" | "x27" | "x28"
    )
}

pub(crate) fn is_stack_pointer(register: &str) -> bool {
    register == stack_pointer()
}

pub(crate) fn label(name: &str) -> CodeInstruction {
    CodeInstruction::new("label").field("name", name)
}

pub(crate) fn move_register(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("mov")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn move_immediate(dst: &str, type_: &str, value: &str) -> CodeInstruction {
    CodeInstruction::new("mov_imm")
        .field("dst", dst)
        .field("type", type_)
        .field("value", value)
}

pub(crate) fn add_immediate(dst: &str, src: &str, imm: usize) -> CodeInstruction {
    CodeInstruction::new("add_imm")
        .field("dst", dst)
        .field("src", src)
        .field("imm", &imm.to_string())
}

pub(crate) fn subtract_immediate(dst: &str, src: &str, imm: usize) -> CodeInstruction {
    CodeInstruction::new("sub_imm")
        .field("dst", dst)
        .field("src", src)
        .field("imm", &imm.to_string())
}

pub(crate) fn add_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("add")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn add_registers_set_flags(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("adds")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn subtract_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("sub")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn subtract_registers_set_flags(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("subs")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn and_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("and")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn or_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("orr")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn exclusive_or_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("eor")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn bitwise_not(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("mvn")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn multiply_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("mul")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn signed_multiply_high_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("smulh")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn unsigned_multiply_high_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("umulh")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

/// `adc dst, lhs, rhs` — add with carry, reading the carry flag left by a prior
/// flag-setting add (`adds`). Used to chain a 128-bit addition across two
/// 64-bit limbs.
pub(crate) fn add_with_carry_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("adc")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

/// `rorv dst, src, amount` — rotate `src` right by the low 6 bits of `amount`.
pub(crate) fn rotate_right_registers(dst: &str, src: &str, amount: &str) -> CodeInstruction {
    CodeInstruction::new("rorv")
        .field("dst", dst)
        .field("lhs", src)
        .field("rhs", amount)
}

/// `rorv Wd, Wn, Wm` — 32-bit rotate right by the low 5 bits of `amount`; the
/// 32-bit result is zero-extended into the upper half of the destination.
pub(crate) fn rotate_right_word_registers(dst: &str, src: &str, amount: &str) -> CodeInstruction {
    CodeInstruction::new("rorv_w")
        .field("dst", dst)
        .field("lhs", src)
        .field("rhs", amount)
}

/// `lslv dst, src, amount` — logical shift `src` left by the low 6 bits of `amount`.
pub(crate) fn shift_left_variable(dst: &str, src: &str, amount: &str) -> CodeInstruction {
    CodeInstruction::new("lslv")
        .field("dst", dst)
        .field("lhs", src)
        .field("rhs", amount)
}

/// `lsrv dst, src, amount` — logical shift `src` right by the low 6 bits of `amount`.
pub(crate) fn shift_right_variable(dst: &str, src: &str, amount: &str) -> CodeInstruction {
    CodeInstruction::new("lsrv")
        .field("dst", dst)
        .field("lhs", src)
        .field("rhs", amount)
}

/// `asrv dst, src, amount` — arithmetic (sign-filling) shift `src` right by the
/// low 6 bits of `amount`.
pub(crate) fn arithmetic_shift_right_variable(
    dst: &str,
    src: &str,
    amount: &str,
) -> CodeInstruction {
    CodeInstruction::new("asrv")
        .field("dst", dst)
        .field("lhs", src)
        .field("rhs", amount)
}

/// `clz dst, src` — count the leading zero bits of the 64-bit `src`.
pub(crate) fn count_leading_zeros(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("clz")
        .field("dst", dst)
        .field("src", src)
}

/// `rbit dst, src` — reverse the bit order of the 64-bit `src`.
pub(crate) fn reverse_bits(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("rbit")
        .field("dst", dst)
        .field("src", src)
}

/// `rev Wd, Wn` — reverse the four bytes of the low 32 bits of `src`; the result
/// is zero-extended into the upper half of the destination.
pub(crate) fn reverse_bytes_word(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("rev_w")
        .field("dst", dst)
        .field("src", src)
}

/// `rev Xd, Xn` — reverse all eight bytes of the 64-bit `src`.
pub(crate) fn reverse_bytes(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("rev_x")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn signed_divide_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("sdiv")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn unsigned_divide_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("udiv")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn multiply_subtract_registers(
    dst: &str,
    lhs: &str,
    rhs: &str,
    minuend: &str,
) -> CodeInstruction {
    CodeInstruction::new("msub")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
        .field("minuend", minuend)
}

pub(crate) fn shift_left_immediate(dst: &str, src: &str, shift: u8) -> CodeInstruction {
    CodeInstruction::new("lsl_imm")
        .field("dst", dst)
        .field("src", src)
        .field("shift", &shift.to_string())
}

pub(crate) fn shift_right_immediate(dst: &str, src: &str, shift: u8) -> CodeInstruction {
    CodeInstruction::new("lsr_imm")
        .field("dst", dst)
        .field("src", src)
        .field("shift", &shift.to_string())
}

pub(crate) fn arithmetic_shift_right_immediate(dst: &str, src: &str, shift: u8) -> CodeInstruction {
    CodeInstruction::new("asr_imm")
        .field("dst", dst)
        .field("src", src)
        .field("shift", &shift.to_string())
}

pub(crate) fn subtract_stack(imm: usize) -> CodeInstruction {
    CodeInstruction::new("sub_sp").field("imm", &imm.to_string())
}

pub(crate) fn add_stack(imm: usize) -> CodeInstruction {
    CodeInstruction::new("add_sp").field("imm", &imm.to_string())
}

pub(crate) fn compare_immediate(lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("cmp_imm")
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn compare_registers(lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("cmp")
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn branch_eq(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.eq").field("target", target)
}

pub(crate) fn branch_ne(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.ne").field("target", target)
}

pub(crate) fn branch_ge(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.ge").field("target", target)
}

pub(crate) fn branch_lt(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.lt").field("target", target)
}

pub(crate) fn branch_gt(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.gt").field("target", target)
}

pub(crate) fn branch_le(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.le").field("target", target)
}

pub(crate) fn branch_vc(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.vc").field("target", target)
}

pub(crate) fn branch_vs(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.vs").field("target", target)
}

pub(crate) fn branch_hi(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.hi").field("target", target)
}

pub(crate) fn branch_lo(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.lo").field("target", target)
}

pub(crate) fn branch(target: &str) -> CodeInstruction {
    CodeInstruction::new("b").field("target", target)
}

pub(crate) fn branch_link(target: &str) -> CodeInstruction {
    CodeInstruction::new("bl").field("target", target)
}

pub(crate) fn branch_link_register(register: &str) -> CodeInstruction {
    CodeInstruction::new("blr").field("register", register)
}

pub(crate) fn branch_self() -> CodeInstruction {
    CodeInstruction::new("branch_self")
}

pub(crate) fn syscall() -> CodeInstruction {
    CodeInstruction::new("svc")
}

pub(crate) fn return_() -> CodeInstruction {
    CodeInstruction::new("ret")
}

pub(crate) fn load_u64(dst: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("ldr_u64")
        .field("dst", dst)
        .field("base", base)
        .field("offset", &offset.to_string())
}

#[allow(dead_code)]
pub(crate) fn load_u32(dst: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("ldr_u32")
        .field("dst", dst)
        .field("base", base)
        .field("offset", &offset.to_string())
}

#[allow(dead_code)]
pub(crate) fn load_u16(dst: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("ldr_u16")
        .field("dst", dst)
        .field("base", base)
        .field("offset", &offset.to_string())
}

pub(crate) fn load_u8(dst: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("ldr_u8")
        .field("dst", dst)
        .field("base", base)
        .field("offset", &offset.to_string())
}

pub(crate) fn store_u64(src: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("str_u64")
        .field("src", src)
        .field("base", base)
        .field("offset", &offset.to_string())
}

pub(crate) fn store_u32(src: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("str_u32")
        .field("src", src)
        .field("base", base)
        .field("offset", &offset.to_string())
}

pub(crate) fn store_u8(src: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("str_u8")
        .field("src", src)
        .field("base", base)
        .field("offset", &offset.to_string())
}

/// `ldr d<dst>, [<base>, #offset]` — load a 64-bit FP scalar (spill reload).
pub(crate) fn load_double(dst: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("ldr_d")
        .field("dst", dst)
        .field("base", base)
        .field("offset", &offset.to_string())
}

/// `str d<src>, [<base>, #offset]` — store a 64-bit FP scalar (spill).
pub(crate) fn store_double(src: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("str_d")
        .field("src", src)
        .field("base", base)
        .field("offset", &offset.to_string())
}

pub(crate) fn load_page_address(dst: &str, symbol: &str) -> CodeInstruction {
    CodeInstruction::new("adrp")
        .field("dst", dst)
        .field("symbol", symbol)
}

pub(crate) fn add_page_offset(dst: &str, src: &str, symbol: &str) -> CodeInstruction {
    CodeInstruction::new("add_pageoff")
        .field("dst", dst)
        .field("src", src)
        .field("symbol", symbol)
}

pub(crate) fn float_move_x_from_d(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fmov_x_from_d")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn float_move_d_from_x(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fmov_d_from_x")
        .field("dst", dst)
        .field("src", src)
}

/// `fmov Dd, Dn` — copy one scalar `d`-register into another.
pub(crate) fn float_move_d_from_d(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fmov_d_from_d")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn float_add_d(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("fadd_d")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn float_subtract_d(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("fsub_d")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn float_multiply_d(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("fmul_d")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn float_divide_d(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("fdiv_d")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn float_negate_d(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fneg_d")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn float_sqrt_d(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fsqrt_d")
        .field("dst", dst)
        .field("src", src)
}

/// `fabs Dd, Dn` — scalar double absolute value (clears the sign bit), so the
/// FP-domain finiteness check can fold ±Inf onto a single `fcmp` against +Inf.
pub(crate) fn float_abs_d(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fabs_d")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn float_compare_d(lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("fcmp_d")
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn float_compare_zero_d(src: &str) -> CodeInstruction {
    CodeInstruction::new("fcmp_zero_d").field("src", src)
}

pub(crate) fn signed_convert_to_float_d(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("scvtf_d_from_x")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn float_convert_to_signed_x(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fcvtzs_x_from_d")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn float_floor_to_signed_x(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fcvtms_x_from_d")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn float_ceil_to_signed_x(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fcvtps_x_from_d")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn float_round_to_signed_x(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("fcvtas_x_from_d")
        .field("dst", dst)
        .field("src", src)
}

// --- NEON vector constructors (plan-01-simd Phase 1) ---
//
// Vector operands are named `v0`..`v31`; the lane arrangement (`.2d` for the
// numeric kernels, `.16b` for the bitwise/select ops) is fixed by each op. The
// base GPR for `ldr_q`/`str_q` and the source GPR for `dup` use the ordinary
// `x*` names.

/// `ldr q<dst>, [<base>, #offset]` — load 128 bits (two i64/f64 lanes).
#[allow(dead_code)]
pub(crate) fn vector_load(dst: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("ldr_q")
        .field("dst", dst)
        .field("base", base)
        .field("offset", &offset.to_string())
}

/// `str q<src>, [<base>, #offset]` — store 128 bits (two i64/f64 lanes).
#[allow(dead_code)]
pub(crate) fn vector_store(src: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("str_q")
        .field("src", src)
        .field("base", base)
        .field("offset", &offset.to_string())
}

fn vector_three(op: &str, dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new(op)
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

fn vector_two(op: &str, dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new(op).field("dst", dst).field("src", src)
}

fn vector_shift(op: &str, dst: &str, src: &str, shift: u8) -> CodeInstruction {
    CodeInstruction::new(op)
        .field("dst", dst)
        .field("src", src)
        .field("shift", &shift.to_string())
}

macro_rules! vector_three_same {
    ($name:ident, $op:literal) => {
        #[allow(dead_code)]
        pub(crate) fn $name(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
            vector_three($op, dst, lhs, rhs)
        }
    };
}

macro_rules! vector_two_misc {
    ($name:ident, $op:literal) => {
        #[allow(dead_code)]
        pub(crate) fn $name(dst: &str, src: &str) -> CodeInstruction {
            vector_two($op, dst, src)
        }
    };
}

macro_rules! vector_shift_imm {
    ($name:ident, $op:literal) => {
        #[allow(dead_code)]
        pub(crate) fn $name(dst: &str, src: &str, shift: u8) -> CodeInstruction {
            vector_shift($op, dst, src, shift)
        }
    };
}

vector_three_same!(vector_fadd, "fadd_v");
vector_three_same!(vector_fsub, "fsub_v");
vector_three_same!(vector_fmul, "fmul_v");
vector_three_same!(vector_fdiv, "fdiv_v");
vector_three_same!(vector_fmla, "fmla_v");
vector_three_same!(vector_fmls, "fmls_v");
vector_three_same!(vector_fmin, "fmin_v");
vector_three_same!(vector_fmax, "fmax_v");
vector_three_same!(vector_fcmgt, "fcmgt_v");
vector_three_same!(vector_fcmge, "fcmge_v");
vector_three_same!(vector_fcmeq, "fcmeq_v");
vector_three_same!(vector_add, "add_v");
vector_three_same!(vector_sub, "sub_v");
vector_three_same!(vector_cmgt, "cmgt_v");
vector_three_same!(vector_cmge, "cmge_v");
vector_three_same!(vector_cmeq, "cmeq_v");
vector_three_same!(vector_sshl, "sshl_v");
vector_three_same!(vector_ushl, "ushl_v");
vector_three_same!(vector_and, "and_v");
vector_three_same!(vector_orr, "orr_v");
vector_three_same!(vector_eor, "eor_v");
vector_three_same!(vector_bsl, "bsl_v");
vector_three_same!(vector_bit, "bit_v");

vector_two_misc!(vector_fabs, "fabs_v");
vector_two_misc!(vector_fneg, "fneg_v");
vector_two_misc!(vector_fsqrt, "fsqrt_v");
vector_two_misc!(vector_frintp, "frintp_v");
vector_two_misc!(vector_frintm, "frintm_v");
vector_two_misc!(vector_frinta, "frinta_v");
vector_two_misc!(vector_frintn, "frintn_v");
vector_two_misc!(vector_frintz, "frintz_v");
vector_two_misc!(vector_fcvtzs, "fcvtzs_v");
vector_two_misc!(vector_fcvtas, "fcvtas_v");
vector_two_misc!(vector_scvtf, "scvtf_v");
vector_two_misc!(vector_neg, "neg_v");
vector_two_misc!(vector_abs, "abs_v");
vector_two_misc!(vector_fcmgt_zero, "fcmgt_zero_v");
vector_two_misc!(vector_fcmge_zero, "fcmge_zero_v");
vector_two_misc!(vector_fcmeq_zero, "fcmeq_zero_v");
vector_two_misc!(vector_fcmlt_zero, "fcmlt_zero_v");
vector_two_misc!(vector_fcmle_zero, "fcmle_zero_v");

vector_shift_imm!(vector_shl, "shl_v");
vector_shift_imm!(vector_sshr, "sshr_v");
vector_shift_imm!(vector_ushr, "ushr_v");

/// `dup v<dst>.2d, x<src>` — broadcast a 64-bit GPR into both lanes.
#[allow(dead_code)]
pub(crate) fn vector_dup_from_x(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("dup_v_from_x")
        .field("dst", dst)
        .field("src", src)
}

/// `umov x<dst>, v<src>.d[index]` — extract lane `index` (0 or 1) into a GPR.
#[allow(dead_code)]
pub(crate) fn vector_extract_to_x(dst: &str, src: &str, index: u8) -> CodeInstruction {
    CodeInstruction::new("umov_x_from_v")
        .field("dst", dst)
        .field("src", src)
        .field("index", &index.to_string())
}

/// `fmadd d<dst>, d<lhs>, d<rhs>, d<addend>` — `dst = addend + lhs*rhs` (one round).
#[allow(dead_code)]
pub(crate) fn float_multiply_add_d(dst: &str, addend: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("fmadd_d")
        .field("dst", dst)
        .field("addend", addend)
        .field("lhs", lhs)
        .field("rhs", rhs)
}
