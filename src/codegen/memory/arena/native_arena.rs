//! Split from `the retired flat native_helpers.rs` (category `memory.arena`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
/// Load the address of a read-only data symbol into `dst` (adrp + add).
pub(crate) fn emit_data_address(
    from: &str,
    // plan-85-B: accept a typed `Operand` (`abi::c_arg(1)`) or a legacy `&str`.
    dst: impl Into<Operand>,
    data_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let dst = dst.into();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", &dst)
            .field("symbol", data_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", &dst)
            .field("src", &dst)
            .field("symbol", data_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

/// `bl _mfb_arena_free` returning a single compiler-sized block to the arena.
/// The caller stages the block pointer in the return register (`x0`) and its
/// original allocation size in `ARG[1]` (`x1`).
pub(crate) fn emit_arena_free(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(ARENA_FREE_SYMBOL));
    relocations.push(crate::codegen::engine::builder::internal_branch(
        symbol,
        ARENA_FREE_SYMBOL,
    ));
}
