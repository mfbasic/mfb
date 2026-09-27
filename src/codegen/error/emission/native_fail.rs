//! Split from `the retired flat native_helpers.rs` (category `error.emission`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
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

/// `bl _mfb_rt_fail_status` with the prefix already in `c_arg(0)` and the status
/// in `c_arg(1)`: stage the code and the static fallback message, then call.
fn emit_fail_status_call(
    symbol: &str,
    error_name: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let (code, message_symbol) = crate::codegen::registry::runtime_error_emission(error_name)
        .unwrap_or_else(|| panic!("StatusFailSites: `{error_name}` is not an errorCode constant"));
    instructions.push(abi::move_immediate(abi::c_arg(2), "Integer", code));
    push_symbol_address(
        symbol,
        message_symbol,
        abi::c_arg(3),
        instructions,
        relocations,
    );
    instructions.push(abi::branch_link(FAIL_STATUS_SYMBOL));
    relocations.push(internal_branch(symbol, FAIL_STATUS_SYMBOL));
}

/// plan-158-A: [`emit_fail`] for failed OS calls, each reporting its own call name
/// and status: `"<registered message> (<call>: 0x<8 hex digits>)"`, raised as a
/// parked `Error` block through `_mfb_rt_fail_status` (see `fail_status.rs`). The
/// calls share one cleanup tail and raise one error.
///
/// Only a helper of a package in [`PARKING_HELPER_PACKAGES`] may use this: its
/// call sites are the ones that re-stamp a parked helper error with their own
/// location. The module build refuses any other (`builder/mod.rs`, the
/// `_mfb_rt_fail_status` gate).
///
/// Each failing check branches to its own stub ([`Self::site`]). The stub saves the
/// status and the call's message-prefix address in two frame slots, then jumps to
/// the shared tail. The tail runs the helper's cleanup and then
/// [`Self::emit_raise`], which reloads both and calls `_mfb_rt_fail_status`. The
/// frame slots are what carry the two values across the cleanup's own calls.
pub(crate) struct StatusFailSites {
    symbol: String,
    error_name: &'static str,
    tail: String,
    status_off: usize,
    prefix_off: usize,
    stubs: Vec<(String, String, Reg)>,
    /// How many of `stubs` [`Self::emit_stubs`] has emitted.
    emitted: usize,
}

impl StatusFailSites {
    /// `tail` is the label the caller places before its cleanup and
    /// [`Self::emit_raise`]; `status_off` / `prefix_off` are two otherwise-unused
    /// `sp`-relative frame slots.
    pub(crate) fn new(
        symbol: &str,
        error_name: &'static str,
        tail: &str,
        status_off: usize,
        prefix_off: usize,
    ) -> Self {
        Self {
            symbol: symbol.to_string(),
            error_name,
            tail: tail.to_string(),
            status_off,
            prefix_off,
            stubs: Vec::new(),
            emitted: 0,
        }
    }

    /// The label a failing check of `call` branches to. `status` is the register
    /// holding the OS status at that branch.
    pub(crate) fn site(&mut self, call: &str, status: impl Into<Reg>) -> String {
        let label = format!("{}_s{}", self.tail, self.stubs.len());
        self.stubs
            .push((label.clone(), call.to_string(), status.into()));
        label
    }

    /// The tail label (the target of every stub).
    pub(crate) fn tail(&self) -> &str {
        &self.tail
    }

    /// Emit every stub. Place them where control cannot fall into them (after an
    /// unconditional branch). `scratch` is a register the stubs may clobber.
    pub(crate) fn emit_stubs(
        &mut self,
        scratch: impl Into<Reg>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) {
        let scratch = scratch.into();
        for (label, call, status) in &self.stubs {
            instructions.push(abi::label(label));
            instructions.push(abi::store_u64(
                *status,
                abi::stack_pointer(),
                self.status_off,
            ));
            push_symbol_address(
                &self.symbol,
                &error_status_prefix_symbol(self.error_name, call),
                scratch,
                instructions,
                relocations,
            );
            instructions.push(abi::store_u64(
                scratch,
                abi::stack_pointer(),
                self.prefix_off,
            ));
            instructions.push(abi::branch(&self.tail));
        }
        self.emitted = self.stubs.len();
    }

    /// Raise the error the taken stub recorded, then branch to `done`. Emit it at
    /// the end of the tail, after the cleanup.
    pub(crate) fn emit_raise(
        &self,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
        done: &str,
    ) {
        instructions.push(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            self.status_off,
        ));
        instructions.push(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            self.prefix_off,
        ));
        emit_fail_status_call(&self.symbol, self.error_name, instructions, relocations);
        instructions.push(abi::branch(done));
    }
}

/// A site registered after (or without) [`StatusFailSites::emit_stubs`] is a branch
/// to a label nothing defines: the module then fails to assemble with a message
/// about a compiler-internal label, and only for the target that lowers that
/// helper. Refuse it where it happens instead.
impl Drop for StatusFailSites {
    fn drop(&mut self) {
        if !std::thread::panicking() && self.emitted != self.stubs.len() {
            panic!(
                "StatusFailSites `{}`: {} site(s) registered but only {} stub(s) emitted",
                self.tail,
                self.stubs.len(),
                self.emitted
            );
        }
    }
}
