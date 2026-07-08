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

/// The first register-passed argument index; arguments at or beyond this go on
/// the stack (bug-08). The custom calling convention (`mfb spec memory
/// 06_native-calling-convention`) delivers arguments 0..[`REGISTER_ARGUMENT_COUNT`]
/// in `x0`–`x7` and the rest in a stack tail.
pub(crate) const REGISTER_ARGUMENT_COUNT: usize = 8;

/// Sentinel base register naming the callee's *incoming* stack-argument area —
/// the caller's outgoing tail, read relative to the entry stack pointer. The
/// real `sp`-relative offset is not known until the frame is finalized (it sits
/// above the whole frame), so `finalize_frame` rewrites this base to `sp` and
/// resolves the offset to `frame_size + entry_padding + k*8` (bug-08).
pub(crate) const INCOMING_ARGS_BASE: &str = "incoming_args";

/// Sentinel base register naming the caller's *outgoing* stack-argument area,
/// reserved at the very bottom of the caller frame so that at the call the args
/// sit at `[sp+0..]` where the callee expects them. `finalize_frame` rewrites
/// this base to `sp` (the offset `k*8` is already frame-bottom-relative and is
/// left unshifted) (bug-08).
pub(crate) const OUTGOING_ARGS_BASE: &str = "outgoing_args";

/// Load the `k`-th incoming stack argument (0-based beyond the 8 register
/// arguments) into `dst`. Resolved to a concrete `sp`-relative load in
/// `finalize_frame` (bug-08).
pub(crate) fn incoming_stack_arg_load(dst: &str, k: usize) -> CodeInstruction {
    load_u64(dst, INCOMING_ARGS_BASE, k * 8)
}

/// Store `src` as the `k`-th outgoing stack argument (0-based beyond the 8
/// register arguments) into the caller's reserved outgoing area. Resolved to a
/// concrete `sp`-relative store in `finalize_frame` (bug-08).
pub(crate) fn outgoing_stack_arg_store(src: &str, k: usize) -> CodeInstruction {
    store_u64(src, OUTGOING_ARGS_BASE, k * 8)
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

pub(crate) fn syscall_register() -> &'static str {
    "x8"
}

pub(crate) fn string_length_register() -> &'static str {
    "x2"
}

pub(crate) fn string_data_register() -> &'static str {
    "x1"
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

/// `add_carry dst, carry_out, lhs, rhs, carry_in` (plan-00-G §4) — explicit-carry
/// add: `dst = lhs + rhs + carry_in`, `carry_out` the unsigned carry as a value.
/// The carry is a register, not the flags, so a multi-limb add survives register
/// allocation. Pass `xzr` for `carry_in` on the first limb and for `carry_out`
/// on the last limb.
pub(crate) fn add_carry(
    dst: &str,
    carry_out: &str,
    lhs: &str,
    rhs: &str,
    carry_in: &str,
) -> CodeInstruction {
    CodeInstruction::new("add_carry")
        .field("dst", dst)
        .field("carry_out", carry_out)
        .field("lhs", lhs)
        .field("rhs", rhs)
        .field("carry_in", carry_in)
}

/// `sub_borrow dst, borrow_out, lhs, rhs, borrow_in` (plan-00-G §4) — explicit-
/// borrow subtract: `dst = lhs - rhs - borrow_in`, `borrow_out` the borrow as a
/// value. Subtractive counterpart to [`add_carry`].
#[allow(dead_code)]
pub(crate) fn sub_borrow(
    dst: &str,
    borrow_out: &str,
    lhs: &str,
    rhs: &str,
    borrow_in: &str,
) -> CodeInstruction {
    CodeInstruction::new("sub_borrow")
        .field("dst", dst)
        .field("borrow_out", borrow_out)
        .field("lhs", lhs)
        .field("rhs", rhs)
        .field("borrow_in", borrow_in)
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

/// `sxtw Xd, Wn` — sign-extend the low 32 bits of `src` into the 64-bit `dst`.
/// Narrows a C `int` return (AAPCS64 leaves x-bits[63:32] unspecified) so a
/// subsequent 64-bit `cmp`/`b.lt` sign-check is correct (bug-04).
pub(crate) fn sign_extend_word(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("sxtw")
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

/// `b.mi` — branch if N set. After `fcmp` this is the IEEE float `<` (an
/// unordered NaN clears N, so it falls through to the `false` side; plan-17).
pub(crate) fn branch_mi(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.mi").field("target", target)
}

/// `b.ls` — branch if C clear or Z set. After `fcmp` this is the IEEE float
/// `<=` (an unordered NaN has C set and Z clear, so it falls through to the
/// `false` side; plan-17).
pub(crate) fn branch_ls(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.ls").field("target", target)
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

/// `fminnm Dd, Dn, Dm` — scalar double minimum with IEEE number semantics (a
/// finite operand wins over a NaN). Selected for `math::min(Float)` (plan-02 §4).
pub(crate) fn float_min_d(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("fminnm_d")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

/// `fmaxnm Dd, Dn, Dm` — scalar double maximum, IEEE number semantics.
/// Selected for `math::max(Float)` (plan-02 §4).
pub(crate) fn float_max_d(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("fmaxnm_d")
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

/// Build one of the four scalar fused-multiply-add ops (one round). All share the
/// `dst`,`addend`,`lhs`,`rhs` field shape; the mnemonic fixes the sign combination
/// (see [`crate::arch::aarch64::ops::CodeOp`] docs / plan-02 §5):
///   `fmadd_d`  = `addend + lhs*rhs`
///   `fmsub_d`  = `lhs*rhs - addend`
///   `fnmsub_d` = `addend - lhs*rhs`
///   `fnmadd_d` = `-(lhs*rhs) - addend`
fn float_fma_op(mnemonic: &str, dst: &str, addend: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new(mnemonic)
        .field("dst", dst)
        .field("addend", addend)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

/// `dst = addend + lhs*rhs`, rounded once.
pub(crate) fn float_multiply_add_d(dst: &str, addend: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    float_fma_op("fmadd_d", dst, addend, lhs, rhs)
}

/// `dst = lhs*rhs - addend`, rounded once.
pub(crate) fn float_multiply_sub_d(dst: &str, addend: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    float_fma_op("fmsub_d", dst, addend, lhs, rhs)
}

/// `dst = addend - lhs*rhs`, rounded once.
pub(crate) fn float_negate_multiply_sub_d(
    dst: &str,
    addend: &str,
    lhs: &str,
    rhs: &str,
) -> CodeInstruction {
    float_fma_op("fnmsub_d", dst, addend, lhs, rhs)
}

/// `dst = -(lhs*rhs) - addend`, rounded once. The fourth sign combination of the
/// scalar FMA family; the op and its per-backend encodings are exercised by the
/// byte tests, but the multiply-accumulate recognizer only emits the other three
/// (a `-(a*b) - c` source is a rarer three-node shape), so this builder currently
/// has no caller — kept for completeness / future negated-product fusion.
#[allow(dead_code)]
pub(crate) fn float_negate_multiply_add_d(
    dst: &str,
    addend: &str,
    lhs: &str,
    rhs: &str,
) -> CodeInstruction {
    float_fma_op("fnmadd_d", dst, addend, lhs, rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(inst: &'a CodeInstruction, key: &str) -> Option<&'a str> {
        inst.fields
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn register_role_helpers() {
        assert_eq!(argument_register(0).unwrap(), "x0");
        assert_eq!(argument_register(7).unwrap(), "x7");
        assert!(argument_register(8).is_err());
        // bug-08: arguments beyond the register window go through the stack-tail
        // sentinels, resolved to concrete `sp`-relative accesses in the frame.
        assert_eq!(REGISTER_ARGUMENT_COUNT, 8);
        let incoming = incoming_stack_arg_load("x9", 2);
        assert_eq!(incoming.op.mnemonic(), "ldr_u64");
        assert_eq!(get(&incoming, "base"), Some(INCOMING_ARGS_BASE));
        assert_eq!(get(&incoming, "offset"), Some("16"));
        assert_eq!(get(&incoming, "dst"), Some("x9"));
        let outgoing = outgoing_stack_arg_store("x9", 0);
        assert_eq!(outgoing.op.mnemonic(), "str_u64");
        assert_eq!(get(&outgoing, "base"), Some(OUTGOING_ARGS_BASE));
        assert_eq!(get(&outgoing, "offset"), Some("0"));
        assert_eq!(get(&outgoing, "src"), Some("x9"));
        // Temporary allocations cover the caller-saved run and the callee-saved remap.
        assert_eq!(temporary_register(8).unwrap(), "x8");
        assert_eq!(temporary_register(17).unwrap(), "x17");
        assert_eq!(temporary_register(18).unwrap(), "x20");
        assert_eq!(temporary_register(26).unwrap(), "x28");
        assert!(temporary_register(27).is_err());
        // FP temporaries.
        assert_eq!(fp_temporary_register(0).unwrap(), "d0");
        assert_eq!(fp_temporary_register(7).unwrap(), "d7");
        assert!(fp_temporary_register(8).is_err());
        // Named ABI registers.
        assert_eq!(return_register(), "x0");
        assert_eq!(link_register(), "x30");
        assert_eq!(stack_pointer(), "sp");
        assert_eq!(syscall_register(), "x8");
        assert_eq!(string_length_register(), "x2");
        assert_eq!(string_data_register(), "x1");
        assert!(is_callee_saved("x19"));
        assert!(is_callee_saved("x28"));
        assert!(!is_callee_saved("x0"));
        assert!(is_stack_pointer("sp"));
        assert!(!is_stack_pointer("x0"));
    }

    #[test]
    fn instruction_constructors_carry_op_and_fields() {
        // Each constructor names its op and lays out the expected fields.
        assert_eq!(label("L").op.mnemonic(), "label");
        assert_eq!(get(&label("L"), "name"), Some("L"));

        let cases: Vec<(CodeInstruction, &str)> = vec![
            (move_register("x0", "x1"), "mov"),
            (move_immediate("x0", "Integer", "3"), "mov_imm"),
            (add_immediate("x0", "x1", 4), "add_imm"),
            (subtract_immediate("x0", "x1", 4), "sub_imm"),
            (add_registers("x0", "x1", "x2"), "add"),
            (add_registers_set_flags("x0", "x1", "x2"), "adds"),
            (subtract_registers("x0", "x1", "x2"), "sub"),
            (subtract_registers_set_flags("x0", "x1", "x2"), "subs"),
            (and_registers("x0", "x1", "x2"), "and"),
            (or_registers("x0", "x1", "x2"), "orr"),
            (exclusive_or_registers("x0", "x1", "x2"), "eor"),
            (bitwise_not("x0", "x1"), "mvn"),
            (multiply_registers("x0", "x1", "x2"), "mul"),
            (signed_multiply_high_registers("x0", "x1", "x2"), "smulh"),
            (unsigned_multiply_high_registers("x0", "x1", "x2"), "umulh"),
            (add_carry("x0", "x1", "x2", "x3", "xzr"), "add_carry"),
            (sub_borrow("x0", "x1", "x2", "x3", "xzr"), "sub_borrow"),
            (rotate_right_registers("x0", "x1", "x2"), "rorv"),
            (rotate_right_word_registers("x0", "x1", "x2"), "rorv_w"),
            (shift_left_variable("x0", "x1", "x2"), "lslv"),
            (shift_right_variable("x0", "x1", "x2"), "lsrv"),
            (arithmetic_shift_right_variable("x0", "x1", "x2"), "asrv"),
            (count_leading_zeros("x0", "x1"), "clz"),
            (reverse_bits("x0", "x1"), "rbit"),
            (reverse_bytes_word("x0", "x1"), "rev_w"),
            (reverse_bytes("x0", "x1"), "rev_x"),
            (signed_divide_registers("x0", "x1", "x2"), "sdiv"),
            (unsigned_divide_registers("x0", "x1", "x2"), "udiv"),
            (multiply_subtract_registers("x0", "x1", "x2", "x3"), "msub"),
            (shift_left_immediate("x0", "x1", 3), "lsl_imm"),
            (shift_right_immediate("x0", "x1", 3), "lsr_imm"),
            (arithmetic_shift_right_immediate("x0", "x1", 3), "asr_imm"),
            (subtract_stack(16), "sub_sp"),
            (add_stack(16), "add_sp"),
            (compare_immediate("x0", "1"), "cmp_imm"),
            (compare_registers("x0", "x1"), "cmp"),
            (branch_eq("L"), "b.eq"),
            (branch_ne("L"), "b.ne"),
            (branch_ge("L"), "b.ge"),
            (branch_lt("L"), "b.lt"),
            (branch_gt("L"), "b.gt"),
            (branch_le("L"), "b.le"),
            (branch_vc("L"), "b.vc"),
            (branch_vs("L"), "b.vs"),
            (branch_hi("L"), "b.hi"),
            (branch_lo("L"), "b.lo"),
            (branch_mi("L"), "b.mi"),
            (branch_ls("L"), "b.ls"),
            (branch("L"), "b"),
            (branch_link("f"), "bl"),
            (branch_link_register("x0"), "blr"),
            (branch_self(), "branch_self"),
            (syscall(), "svc"),
            (return_(), "ret"),
            (load_u64("x0", "x1", 8), "ldr_u64"),
            (load_u32("x0", "x1", 4), "ldr_u32"),
            (load_u16("x0", "x1", 2), "ldr_u16"),
            (load_u8("x0", "x1", 1), "ldr_u8"),
            (store_u64("x0", "x1", 8), "str_u64"),
            (store_u32("x0", "x1", 4), "str_u32"),
            (store_u8("x0", "x1", 1), "str_u8"),
            (load_double("d0", "x1", 8), "ldr_d"),
            (store_double("d0", "x1", 8), "str_d"),
            (load_page_address("x0", "g"), "adrp"),
            (add_page_offset("x0", "x0", "g"), "add_pageoff"),
            (float_move_x_from_d("x0", "d1"), "fmov_x_from_d"),
            (float_move_d_from_x("d0", "x1"), "fmov_d_from_x"),
            (float_move_d_from_d("d0", "d1"), "fmov_d_from_d"),
            (float_add_d("d0", "d1", "d2"), "fadd_d"),
            (float_subtract_d("d0", "d1", "d2"), "fsub_d"),
            (float_multiply_d("d0", "d1", "d2"), "fmul_d"),
            (float_divide_d("d0", "d1", "d2"), "fdiv_d"),
            (float_negate_d("d0", "d1"), "fneg_d"),
            (float_sqrt_d("d0", "d1"), "fsqrt_d"),
            (float_abs_d("d0", "d1"), "fabs_d"),
            (float_compare_d("d0", "d1"), "fcmp_d"),
            (float_compare_zero_d("d0"), "fcmp_zero_d"),
            (signed_convert_to_float_d("d0", "x1"), "scvtf_d_from_x"),
            (float_convert_to_signed_x("x0", "d1"), "fcvtzs_x_from_d"),
            (float_floor_to_signed_x("x0", "d1"), "fcvtms_x_from_d"),
            (float_ceil_to_signed_x("x0", "d1"), "fcvtps_x_from_d"),
            (float_round_to_signed_x("x0", "d1"), "fcvtas_x_from_d"),
            (float_multiply_add_d("d0", "d1", "d2", "d3"), "fmadd_d"),
        ];
        for (inst, mnemonic) in cases {
            assert_eq!(inst.op.mnemonic(), mnemonic);
        }
    }

    #[test]
    fn vector_constructors() {
        // Loads/stores and the macro-generated three-same/two-misc/shift builders.
        assert_eq!(vector_load("v0", "x1", 16).op.mnemonic(), "ldr_q");
        assert_eq!(vector_store("v0", "x1", 16).op.mnemonic(), "str_q");
        assert_eq!(vector_fadd("v0", "v1", "v2").op.mnemonic(), "fadd_v");
        assert_eq!(vector_bit("v0", "v1", "v2").op.mnemonic(), "bit_v");
        assert_eq!(vector_fabs("v0", "v1").op.mnemonic(), "fabs_v");
        assert_eq!(vector_fcmle_zero("v0", "v1").op.mnemonic(), "fcmle_zero_v");
        assert_eq!(vector_shl("v0", "v1", 3).op.mnemonic(), "shl_v");
        assert_eq!(vector_sshr("v0", "v1", 3).op.mnemonic(), "sshr_v");
        assert_eq!(vector_ushr("v0", "v1", 3).op.mnemonic(), "ushr_v");
        let dup = vector_dup_from_x("v0", "x1");
        assert_eq!(dup.op.mnemonic(), "dup_v_from_x");
        assert_eq!(get(&dup, "src"), Some("x1"));
        let ext = vector_extract_to_x("x0", "v1", 1);
        assert_eq!(ext.op.mnemonic(), "umov_x_from_v");
        assert_eq!(get(&ext, "index"), Some("1"));
    }
}
