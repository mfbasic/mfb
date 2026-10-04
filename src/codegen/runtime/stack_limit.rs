//! bug-789: setting a thread's stack-limit word (`ARENA_STACK_LIMIT_OFFSET`).
//!
//! Each platform finds the thread's stack in its own way
//! (`CodegenPlatform::emit_stack_limit`); this module holds the arithmetic they
//! share. All of it runs once per thread, right after the thread's arena state is
//! cleared — the program entry and the worker trampoline. It works in the C
//! argument registers, never in `abi::SCRATCH`: on Win64 `SCRATCH[0]` realizes to
//! `rbx`, the pinned current-thread register the trampoline still needs.
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;

/// The reserve kept below the limit for a stack of `size` bytes: the full
/// [`STACK_LIMIT_RESERVE_BYTES`], or a quarter of a stack too small to spare it.
pub(crate) fn reserve_for(size: usize) -> usize {
    STACK_LIMIT_RESERVE_BYTES.min(size / 4)
}

/// `limit = low + min(RESERVE, size / 4)`, stored in this thread's limit word, for
/// a stack whose bottom (`low`) and size (`size`) are known at run time. `low` and
/// `size` are clobbered.
pub(crate) fn emit_limit_from_bounds(
    instructions: &mut Vec<CodeInstruction>,
    label_prefix: &str,
    low: Reg,
    size: Reg,
    scratch: Reg,
) {
    let keep = format!("{label_prefix}_stack_reserve_kept");
    instructions.extend([
        abi::shift_right_immediate(size.clone(), size.clone(), 2),
        abi::move_immediate(
            scratch.clone(),
            "Integer",
            &STACK_LIMIT_RESERVE_BYTES.to_string(),
        ),
        abi::compare_registers(size.clone(), scratch.clone()),
        abi::branch_lo(&keep),
        abi::move_register(size.clone(), scratch),
        abi::label(&keep),
        abi::add_registers(low.clone(), low.clone(), size),
        abi::store_u64(low, ARENA_STATE_REGISTER, ARENA_STACK_LIMIT_OFFSET),
    ]);
}

/// `limit = sp - (size - min(RESERVE, size / 4))` for a stack of a size known at
/// compile time, measured from the current stack pointer: the thread entered near
/// its top, so everything below `sp` is (at least) `size` minus what its start-up
/// frames already used, which the reserve covers.
pub(crate) fn emit_limit_for_known_stack(
    instructions: &mut Vec<CodeInstruction>,
    size: usize,
    sp_copy: Reg,
    budget: Reg,
) {
    let budget_bytes = size - reserve_for(size);
    instructions.extend([
        abi::add_immediate(sp_copy.clone(), abi::stack_pointer(), 0),
        abi::move_immediate(budget.clone(), "Integer", &budget_bytes.to_string()),
        abi::subtract_registers(sp_copy.clone(), sp_copy.clone(), budget),
        abi::store_u64(sp_copy, ARENA_STATE_REGISTER, ARENA_STACK_LIMIT_OFFSET),
    ]);
}

/// The Linux main thread: `cur` holds `RLIMIT_STACK`'s soft limit (or a negative
/// errno). The kernel caps the argument and environment area above the entry
/// `sp` at a quarter of that limit, so at least three quarters lie below `sp`:
/// `limit = sp - (cur - cur / 4 - min(RESERVE, cur / 4))`. An unlimited or failed
/// query counts on [`MAIN_STACK_CAP_BYTES`] / [`THREAD_STACK_BYTES`].
pub(crate) fn emit_limit_from_rlimit(
    instructions: &mut Vec<CodeInstruction>,
    label_prefix: &str,
    cur: Reg,
    quarter: Reg,
    scratch: Reg,
) {
    let failed = format!("{label_prefix}_rlimit_failed");
    let capped = format!("{label_prefix}_rlimit_capped");
    let sized = format!("{label_prefix}_rlimit_sized");
    let keep = format!("{label_prefix}_rlimit_reserve_kept");
    instructions.extend([
        // A raw syscall returns `-errno`; a libc wrapper returns -1.
        abi::compare_immediate(cur.clone(), "0"),
        abi::branch_lt(&failed),
        abi::load_u64(cur.clone(), ARENA_STATE_REGISTER, ARENA_STACK_LIMIT_OFFSET),
        abi::move_immediate(
            scratch.clone(),
            "Integer",
            &MAIN_STACK_CAP_BYTES.to_string(),
        ),
        abi::compare_registers(cur.clone(), scratch.clone()),
        abi::branch_hi(&capped),
        abi::branch(&sized),
        abi::label(&capped),
        abi::move_register(cur.clone(), scratch.clone()),
        abi::branch(&sized),
        abi::label(&failed),
        abi::move_immediate(cur.clone(), "Integer", &THREAD_STACK_BYTES.to_string()),
        abi::label(&sized),
        // quarter = cur / 4; budget = cur - quarter - min(RESERVE, quarter).
        abi::shift_right_immediate(quarter.clone(), cur.clone(), 2),
        abi::subtract_registers(cur.clone(), cur.clone(), quarter.clone()),
        abi::move_immediate(
            scratch.clone(),
            "Integer",
            &STACK_LIMIT_RESERVE_BYTES.to_string(),
        ),
        abi::compare_registers(quarter.clone(), scratch.clone()),
        abi::branch_lo(&keep),
        abi::move_register(quarter.clone(), scratch.clone()),
        abi::label(&keep),
        abi::subtract_registers(cur.clone(), cur.clone(), quarter),
        abi::add_immediate(scratch.clone(), abi::stack_pointer(), 0),
        abi::subtract_registers(scratch.clone(), scratch.clone(), cur),
        abi::store_u64(scratch, ARENA_STATE_REGISTER, ARENA_STACK_LIMIT_OFFSET),
    ]);
}

/// Whether `function` can reach another MFBASIC function: its body names a `Call`
/// or `CallResult` (a user or source-bodied function, or a function value). A body
/// without one cannot recurse; its own frame is covered by the reserve, and a
/// callback a native builtin runs for it checks on its own entry.
pub(crate) fn function_needs_stack_check(
    function: &crate::target::shared::nir::NirFunction,
) -> bool {
    use crate::target::shared::nir::visit::{walk_value, NirVisitor};
    use crate::target::shared::nir::NirValue;
    struct Finder {
        found: bool,
    }
    impl NirVisitor for Finder {
        fn visit_value(&mut self, value: &NirValue) {
            if matches!(value, NirValue::Call { .. } | NirValue::CallResult { .. }) {
                self.found = true;
                return;
            }
            walk_value(self, value);
        }
    }
    let mut finder = Finder { found: false };
    finder.visit_ops(&function.body);
    finder.found
}

impl crate::codegen::engine::builder::CodeBuilder<'_> {
    /// bug-789: at function entry, branch to `overflow` when the stack pointer is
    /// below this thread's limit — a load, a compare and a not-taken branch. Signed
    /// compare: every stack address and a limit are below 2^63, and an unset limit
    /// (`0`) never trips. The raise itself is [`Self::emit_stack_overflow_block`],
    /// placed after the body: inline, its calls sat inside every parameter's live
    /// range and pushed the parameters out of registers (`fib(32)` ran 35% slower).
    pub(crate) fn emit_stack_check(&mut self, overflow: &str) {
        let limit = self.allocate_register();
        let sp = self.allocate_register();
        self.emit(abi::load_u64(
            &limit,
            ARENA_STATE_REGISTER,
            ARENA_STACK_LIMIT_OFFSET,
        ));
        self.emit(abi::add_immediate(&sp, abi::stack_pointer(), 0));
        self.emit(abi::compare_registers(&sp, &limit));
        self.emit(abi::branch_lt(overflow));
    }

    /// The cold block [`Self::emit_stack_check`] branches to: raise
    /// `ErrStackOverflow` as an ordinary error return. It is reached only from the
    /// entry, where nothing is bound, so no cleanup is active.
    pub(crate) fn emit_stack_overflow_block(&mut self, overflow: &str) -> Result<(), String> {
        let cleanups = std::mem::take(&mut self.active_cleanups);
        self.emit(abi::label(overflow));
        let raised = self.raise_error_bare("ErrStackOverflow");
        self.active_cleanups = cleanups;
        raised
    }
}
