//! Split from `src/target/shared/code/native_helpers.rs` (category `error.emission`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::types::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
/// Emit the standard `Result` error tail for a named `errorCode` error: set the
/// code and ERR tag, load the message data address, and branch to `done`. Sources
/// the `(code, message-symbol)` from `ERRORCODE_CONSTANTS` via `raise_error_into`
/// (plan-88-C), so it carries no per-error codegen constants of its own.
pub(crate) fn emit_fail(
    symbol: &str,
    error_name: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    raise_error_into(symbol, error_name, instructions, relocations);
    instructions.push(abi::branch(done));
}
