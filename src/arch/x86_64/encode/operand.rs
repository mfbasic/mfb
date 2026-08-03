//! x86-64 operand decoding: instruction fields, register names, immediates.

// `field`/`immediate`/`shift` are ISA-neutral and shared (bug-341-B7); the
// register-name decoders below are x86-64-specific.
pub(super) use crate::arch::encode_operand::{field, immediate, shift};

/// x86-64 general-purpose register number (0..=15) for the canonical 64-bit
/// register names. The numbering is the architectural encoding:
/// `rax=0, rcx=1, rdx=2, rbx=3, rsp=4, rbp=5, rsi=6, rdi=7, r8..r15=8..15`.
/// `r8`..`r15` need the REX.B/R/X extension bit, handled by the emitter.
pub(super) fn reg(name: impl AsRef<str>) -> Result<u8, String> {
    let name = name.as_ref();
    Ok(match name {
        "rax" => 0,
        "rcx" => 1,
        "rdx" => 2,
        "rbx" => 3,
        "rsp" | "sp" | "raw_sp" => 4,
        "rbp" => 5,
        "rsi" => 6,
        "rdi" => 7,
        "r8" => 8,
        "r9" => 9,
        "r10" => 10,
        "r11" => 11,
        "r12" => 12,
        "r13" => 13,
        "r14" => 14,
        "r15" => 15,
        // The neutral zero token (`abi::ZERO`, spelled `xzr`) names "no register"
        // — used by the explicit-carry ops to express "no carry-in". Reported as
        // 16 so the emitter can branch on it without colliding with a real
        // register. The dead `"rzero"`/`"zero"` aliases were retired in plan-34-A.
        "xzr" => 16,
        other => return Err(format!("unknown x86-64 register '{other}'")),
    })
}

/// True when a parsed register number names the synthetic zero token rather than
/// a hardware register.
pub(super) fn is_zero_token(r: u8) -> bool {
    r == 16
}

/// Parse an SSE register name `xmm0`..`xmm15` to its 0–15 index (select_x86 maps
/// the AArch64 `dN` bank to `xmmN`, and the FP allocator colors `%fN` here too).
pub(super) fn fp_reg(name: impl AsRef<str>) -> Result<u8, String> {
    let name = name.as_ref();
    name.strip_prefix("xmm")
        .and_then(|rest| rest.parse::<u8>().ok())
        .filter(|n| *n < 16)
        .ok_or_else(|| format!("not an xmm register: '{name}'"))
}
