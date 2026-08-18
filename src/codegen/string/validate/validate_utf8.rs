//! Split from `the retired flat codegen_utils.rs` (category `string.validate`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::target::shared::abi;
/// Symbol of the shared standalone UTF-8 validation runtime helper (generic; not
/// path-specific — moved here from `fs/paths.rs`, bug-331 §J).
pub(crate) const VALIDATE_UTF8_SYMBOL: &str = "_mfb_rt_validate_utf8";

/// Emit a call to the shared [`VALIDATE_UTF8_SYMBOL`] helper. The byte pointer
/// must already be in `x0` and the byte length in `x1`. The helper returns `0`
/// in `x0` for valid UTF-8 and `1` for invalid; this branches to `error_label`
/// when invalid. Keeping validation in a separate `bl`-reachable function (with
/// its own frame and short-range internal branches) keeps the filesystem read
/// helpers small.
pub(crate) fn emit_call_validate_utf8(
    symbol: &str,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(VALIDATE_UTF8_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: VALIDATE_UTF8_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::mfb_return(0), "0"),
        abi::branch_ne(error_label),
    ]);
}

/// Lower the standalone UTF-8 validation helper. It takes a byte pointer in `x0`
/// and a byte length in `x1`, and returns `0` in `x0` when the buffer is
/// well-formed UTF-8 or `1` otherwise. The working set is virtual registers the
/// allocator colors per-ISA (a hardcoded pool would land on x86 callee-saved
/// GPRs and clobber the caller); it makes no calls, so the resulting frame is
/// whatever callee-saved saves the coloring requires (typically none).
pub(crate) fn lower_validate_utf8_helper() -> CodeFunction {
    let symbol = VALIDATE_UTF8_SYMBOL;
    let invalid = format!("{symbol}_invalid");
    let mut vregs = Vregs::new();
    let mut instructions = vec![abi::label("entry")];
    emit_validate_utf8(
        symbol,
        abi::c_arg(0),
        abi::c_arg(1),
        &invalid,
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        abi::move_immediate(abi::mfb_return(0), "Integer", "0"),
        abi::return_(),
        abi::label(&invalid),
        abi::move_immediate(abi::mfb_return(0), "Integer", "1"),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.validateUtf8".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: Vec::new(),
    }
}

/// Validate that the `len`-byte buffer at `ptr` is well-formed UTF-8, branching
/// to `error_label` on the first invalid sequence. Used by
/// [`lower_validate_utf8_helper`]. The working set is minted from `vregs`; `ptr`
/// and `len` are read into it before any other def, so they may name `x0`/`x1`.
fn emit_validate_utf8(
    symbol: &str,
    // plan-85-B: accept a typed `Operand` (`abi::c_arg(0/1)`) or a legacy `&str`.
    ptr: impl Into<Operand>,
    len: impl Into<Operand>,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let pos = &vregs.next();
    let rem = &vregs.next();
    let byte = &vregs.next();
    let cont = &vregs.next();
    let lo = &vregs.next();
    let hi = &vregs.next();

    let loop_start = format!("{symbol}_utf8_loop");
    let done = format!("{symbol}_utf8_done");
    let one = format!("{symbol}_utf8_one");
    let two = format!("{symbol}_utf8_two");
    let three = format!("{symbol}_utf8_three");
    let four = format!("{symbol}_utf8_four");
    let three_ed = format!("{symbol}_utf8_three_ed");
    let three_bounds = format!("{symbol}_utf8_three_bounds");
    let four_f4 = format!("{symbol}_utf8_four_f4");
    let four_bounds = format!("{symbol}_utf8_four_bounds");

    instructions.extend([
        abi::move_register(pos, ptr),
        abi::move_register(rem, len),
        abi::label(&loop_start),
        abi::compare_immediate(rem, "0"),
        abi::branch_eq(&done),
        abi::load_u8(byte, pos, 0),
        abi::compare_immediate(byte, "128"),
        abi::branch_lo(&one),
        abi::compare_immediate(byte, "194"),
        abi::branch_lo(error_label),
        abi::compare_immediate(byte, "224"),
        abi::branch_lo(&two),
        abi::compare_immediate(byte, "240"),
        abi::branch_lo(&three),
        abi::compare_immediate(byte, "245"),
        abi::branch_lo(&four),
        abi::branch(error_label),
        // 1-byte ASCII
        abi::label(&one),
        abi::add_immediate(pos, pos, 1),
        abi::subtract_immediate(rem, rem, 1),
        abi::branch(&loop_start),
        // 2-byte sequence
        abi::label(&two),
        abi::compare_immediate(rem, "2"),
        abi::branch_lo(error_label),
        abi::load_u8(cont, pos, 1),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 2),
        abi::subtract_immediate(rem, rem, 2),
        abi::branch(&loop_start),
        // 3-byte sequence
        abi::label(&three),
        abi::compare_immediate(rem, "3"),
        abi::branch_lo(error_label),
        abi::move_immediate(lo, "Integer", "128"),
        abi::move_immediate(hi, "Integer", "191"),
        abi::compare_immediate(byte, "224"),
        abi::branch_ne(&three_ed),
        abi::move_immediate(lo, "Integer", "160"),
        abi::branch(&three_bounds),
        abi::label(&three_ed),
        abi::compare_immediate(byte, "237"),
        abi::branch_ne(&three_bounds),
        abi::move_immediate(hi, "Integer", "159"),
        abi::label(&three_bounds),
        abi::load_u8(cont, pos, 1),
        abi::compare_registers(cont, lo),
        abi::branch_lo(error_label),
        abi::compare_registers(cont, hi),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 2),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 3),
        abi::subtract_immediate(rem, rem, 3),
        abi::branch(&loop_start),
        // 4-byte sequence
        abi::label(&four),
        abi::compare_immediate(rem, "4"),
        abi::branch_lo(error_label),
        abi::move_immediate(lo, "Integer", "128"),
        abi::move_immediate(hi, "Integer", "191"),
        abi::compare_immediate(byte, "240"),
        abi::branch_ne(&four_f4),
        abi::move_immediate(lo, "Integer", "144"),
        abi::branch(&four_bounds),
        abi::label(&four_f4),
        abi::compare_immediate(byte, "244"),
        abi::branch_ne(&four_bounds),
        abi::move_immediate(hi, "Integer", "143"),
        abi::label(&four_bounds),
        abi::load_u8(cont, pos, 1),
        abi::compare_registers(cont, lo),
        abi::branch_lo(error_label),
        abi::compare_registers(cont, hi),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 2),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 3),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 4),
        abi::subtract_immediate(rem, rem, 4),
        abi::branch(&loop_start),
        abi::label(&done),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::mir;

    /// The emitted `_mfb_rt_validate_utf8` helper must always be the real,
    /// multi-byte UTF-8 validator — never a build-time-toggled ASCII-only stub
    /// that rejects every byte > 127 (bug-407). Guards against reintroducing an
    /// undocumented env switch: the helper's structure must not depend on the
    /// process environment. The real validator is identified by its multi-byte
    /// continuation labels (`_utf8_two`/`_utf8_three`/`_utf8_four`), which the
    /// ASCII-only stub never emits.
    #[test]
    fn validate_utf8_helper_is_env_independent_multibyte_validator() {
        fn multibyte_labels(func: &CodeFunction) -> usize {
            func.instructions
                .iter()
                .filter(|ins| ins.op == CodeOp::Label)
                .filter_map(|ins| ins.fields.iter().find(|(n, _)| *n == "name"))
                .filter(|(_, v)| {
                    let v = v.render();
                    v.ends_with("_utf8_two")
                        || v.ends_with("_utf8_three")
                        || v.ends_with("_utf8_four")
                })
                .count()
        }

        // The helper lowers through the active backend; pick one for the test.
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);

        // Default environment: the real validator is emitted.
        let baseline = multibyte_labels(&lower_validate_utf8_helper());
        assert_eq!(
            baseline, 3,
            "default build must emit the multi-byte UTF-8 validator"
        );

        // Setting the former `MFB_ASCII` toggle must not change the emitted
        // helper — it is no longer consulted. Save/restore so we do not leak
        // the variable to other tests in the process.
        let saved = std::env::var_os("MFB_ASCII");
        // SAFETY: single-threaded within this test's synchronous body; the
        // variable is restored before returning.
        unsafe { std::env::set_var("MFB_ASCII", "1") };
        let toggled = multibyte_labels(&lower_validate_utf8_helper());
        match saved {
            Some(v) => unsafe { std::env::set_var("MFB_ASCII", v) },
            None => unsafe { std::env::remove_var("MFB_ASCII") },
        }

        assert_eq!(
            toggled, 3,
            "MFB_ASCII must not toggle the UTF-8 validator into an ASCII-only stub (bug-407)"
        );
    }
}
