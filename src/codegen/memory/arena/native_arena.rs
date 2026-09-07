//! Split from `the retired flat native_helpers.rs` (category `memory.arena`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
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

/// A marshalling **scratch** block a fixed runtime helper allocated for its own
/// use and never hands back (bug-574): the NUL-terminated copy a host call needs
/// of a `String`/`List OF Byte` argument, the `argv` vector `process::spawn`
/// builds, the `sockaddr` a `net::` call fills.
///
/// It is invisible to every caller-side ownership analysis — it is not a
/// `ValueResult` any node yielded, so no statement-scope drop and no `Bind` can
/// reach it — which is why nothing freed it and every such call leaked its
/// argument, proportionally to the argument's length.
///
/// `pointer` is the vreg holding the block (or 0 when the allocation did not
/// happen on this path); `size` is the vreg holding the **same byte count the
/// matching `_mfb_arena_alloc` was given**. `_mfb_arena_free` is size-taking and
/// re-normalizes exactly as `_mfb_arena_alloc` does, so a size that is not the
/// allocation's own returns the block to the wrong bin (bug-560).
#[must_use = "bug-574: a declared scratch block must reach \
              `emit_helper_scratch_release`, or the helper leaks it"]
pub(crate) struct HelperScratch {
    pub(crate) pointer: String,
    pub(crate) size: String,
}

impl HelperScratch {
    /// Name an EXISTING pointer/size vreg pair as a scratch block. The caller is
    /// responsible for nulling both before any branch that can reach `done`.
    pub(crate) fn new(pointer: &str, size: &str) -> Self {
        Self {
            pointer: pointer.to_string(),
            size: size.to_string(),
        }
    }

    /// Mint a fresh pointer/size vreg pair and emit the null-init for both.
    ///
    /// Emit this at the TOP of the helper body — before any branch that can reach
    /// `done`. Nulling at the allocation site instead is wrong for every helper
    /// that can skip the allocation at runtime (`net::listen` with an empty host
    /// jumps straight past its `emit_cstring`), which would leave the release
    /// reading an undefined vreg.
    pub(crate) fn declare(vregs: &mut Vregs, instructions: &mut Vec<CodeInstruction>) -> Self {
        let pointer = vregs.next();
        Self::declare_for(&pointer, vregs, instructions)
    }

    /// [`declare`](Self::declare) for a pointer vreg the body already names (the
    /// `cname`/`c_path` the rest of the lowering reads): mint only the size and
    /// null both.
    pub(crate) fn declare_for(
        pointer: &str,
        vregs: &mut Vregs,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Self {
        let size = vregs.next();
        instructions.extend([
            abi::move_immediate(pointer, "Integer", "0"),
            abi::move_immediate(&size, "Integer", "0"),
        ]);
        Self {
            pointer: pointer.to_string(),
            size,
        }
    }
}

/// Release every scratch block a fixed runtime helper allocated, immediately
/// before its single `ret` (bug-574).
///
/// Placed at the ONE exit rather than at each outcome: these bodies reach `ret`
/// from the success path, from every `raise_error_into` path, and from the
/// embedded-NUL rejection, and a per-outcome free would have to be repeated at
/// each and would be silently incomplete when a new outcome is added.
///
/// Two invariants make it safe there:
///
/// * every `pointer` vreg is **zero-initialized before its allocation**, so a path
///   that reaches `ret` without allocating (the `ErrOutOfMemory` path is exactly
///   that path) frees nothing — the runtime pointer guard, not a whole-program
///   proof;
/// * the fallible-ABI outputs (`RESULT_TAG`/`VALUE`/`MESSAGE`/`ERROR_SOURCE`, i.e.
///   `mfb_return(0..=3)`) are already set when control arrives, and
///   `_mfb_arena_free` is a PCS call that clobbers all of them — it takes its own
///   arguments in `c_arg(0..=1)`, which are the SAME registers (`Mfb`/`C` banks are
///   aligned on every backend). So they are saved into vregs across the frees and
///   restored, and the helper returns exactly the result it computed.
pub(crate) fn emit_helper_scratch_release(
    symbol: &str,
    scratch: &[HelperScratch],
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    if scratch.is_empty() {
        return;
    }
    let saved: Vec<String> = (0..4).map(|_| vregs.next()).collect();
    for (index, slot) in saved.iter().enumerate() {
        instructions.push(abi::move_register(slot, abi::mfb_return(index)));
    }
    for (index, block) in scratch.iter().enumerate() {
        let kept = format!("{symbol}_scratch_kept_{index}");
        instructions.extend([
            abi::compare_immediate(&block.pointer, "0"),
            abi::branch_eq(&kept),
            abi::move_register(abi::c_arg(0), &block.pointer),
            abi::move_register(abi::c_arg(1), &block.size),
        ]);
        emit_arena_free(symbol, instructions, relocations);
        instructions.push(abi::label(&kept));
    }
    for (index, slot) in saved.iter().enumerate() {
        instructions.push(abi::move_register(abi::mfb_return(index), slot));
    }
}
