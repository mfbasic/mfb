//! Where the machine instructions came from: per-NIR-construct attribution for
//! the `-vv` "costliest expansion" report (plan-118-A).
//!
//! The counters in [`crate::trace`] answer *how much* code a module is
//! (17.1 M machine instructions over 52.5 k NIR ops on the acceptance corpus —
//! a 325:1 expansion) but not *what* it expanded from. The whole back-end cost
//! ceiling is that ratio, and the useful question about it is whether the
//! expansion is uniform — in which case the only fix is a different builder —
//! or concentrated in a handful of inline lowerings, in which case out-lining
//! those is the fix. Only an attribution answers that, and it cannot be
//! reconstructed after the fact: the emitted stream carries no record of which
//! NIR node produced which instruction.
//!
//! So the builder records it as it goes. [`enter`] pushes a frame holding the
//! instruction count at the moment a construct starts lowering; [`exit`] pops it
//! and credits the growth to that construct's key — minus whatever its children
//! already claimed, so a `RETURN a & b` credits the concat to `binop:&` and only
//! the return sequence to `op:Return`. **Exclusive**, in other words: the rows
//! partition the builder-emitted total instead of counting a nested site once
//! per enclosing level. That is what makes "the top five categories are 68 % of
//! all emitted instructions" a true statement rather than a double-counted one.
//!
//! The attributed total is *less* than the module's final instruction count:
//! frame prologues, regalloc spill code, and slot zeroing are emitted outside
//! any op frame, and the peepholes delete instructions after attribution. The
//! report is a map of the builder's output, not of the linker's.
//!
//! # Cost when disabled
//!
//! One relaxed atomic load per op and per value ([`crate::trace::enabled`]),
//! ahead of any allocation: the key `String` is built inside a closure the
//! disabled path never calls.

use std::cell::RefCell;

use crate::target::shared::nir::{NirOp, NirValue};

/// One in-progress construct: the instruction count when it started, and how
/// many instructions its children have already claimed.
struct Frame {
    key: String,
    start: usize,
    child: usize,
}

thread_local! {
    /// The open frames on this thread, innermost last. Thread-local because the
    /// builder is per-function and backends may lower functions in parallel;
    /// only the completed, exclusive amounts cross into the shared trace state.
    static STACK: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// The tally bucket both `enter`/`exit` pairs report into.
const BUCKET: &str = "expansion";

/// Open an attribution frame for a construct that is about to be lowered.
///
/// `key` is a closure so the `format!` behind a value key is paid for only when
/// `-vv` is on — these sites are per-NIR-node, the densest instrumentation in
/// the compiler.
pub(crate) fn enter(key: impl FnOnce() -> String, instructions_now: usize) {
    if !crate::trace::enabled() {
        return;
    }
    STACK.with(|stack| {
        stack.borrow_mut().push(Frame {
            key: key(),
            start: instructions_now,
            child: 0,
        });
    });
}

/// Close the innermost frame, crediting it with the instructions emitted since
/// it opened that its children did not already claim.
///
/// Every call site pairs `enter` with an `exit` that no `?` can jump over (the
/// two builder seams wrap their fallible work in a closure and exit after it),
/// so the stack stays balanced. A missing frame is still handled rather than
/// asserted: a panic unwinding through a builder would otherwise turn a
/// compiler bug into a *different* panic here and hide the original.
pub(crate) fn exit(instructions_now: usize) {
    if !crate::trace::enabled() {
        return;
    }
    let finished = STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let frame = stack.pop()?;
        let total = instructions_now.saturating_sub(frame.start);
        // Saturating both ways: a lowering that *removes* instructions (a
        // peephole running inside a frame) must not underflow into a huge
        // unsigned amount and swamp the report.
        let own = total.saturating_sub(frame.child) as u64;
        if let Some(parent) = stack.last_mut() {
            parent.child += total;
        }
        Some((frame.key, own))
    });
    if let Some((key, own)) = finished {
        crate::trace::count_tally(BUCKET, || key, own);
    }
}

/// The attribution key for a statement: the op's variant name.
///
/// Exhaustive on purpose — a new [`NirOp`] variant must be given a key here
/// rather than silently joining someone else's row.
pub(crate) fn op_key(op: &NirOp) -> &'static str {
    match op {
        NirOp::Bind { .. } => "op:Bind",
        NirOp::StoreGlobal { .. } => "op:StoreGlobal",
        NirOp::Assign { .. } => "op:Assign",
        NirOp::StateAssign { .. } => "op:StateAssign",
        NirOp::Return { .. } => "op:Return",
        NirOp::ExitLoop { .. } => "op:ExitLoop",
        NirOp::ContinueLoop { .. } => "op:ContinueLoop",
        NirOp::ExitProgram { .. } => "op:ExitProgram",
        NirOp::Fail { .. } => "op:Fail",
        NirOp::Eval { .. } => "op:Eval",
        NirOp::If { .. } => "op:If",
        NirOp::Match { .. } => "op:Match",
        NirOp::While { .. } => "op:While",
        NirOp::For { .. } => "op:For",
        NirOp::DoUntil { .. } => "op:DoUntil",
        NirOp::ForEach { .. } => "op:ForEach",
        NirOp::Trap { .. } => "op:Trap",
    }
}

/// The attribution key for an expression.
///
/// Calls and operators carry their *target* into the key — `call:toString`,
/// `binop:Concat` — because "calls are expensive" is not actionable and
/// "`toString` costs 177 instructions at each of 5,826 sites" is.
pub(crate) fn value_key(value: &NirValue) -> String {
    match value {
        NirValue::Call { target, .. } => format!("call:{target}"),
        NirValue::CallResult { target, .. } => format!("callres:{target}"),
        NirValue::RuntimeCall { target, .. } => format!("rtcall:{target}"),
        NirValue::Binary { op, .. } => format!("binop:{op:?}"),
        NirValue::Unary { op, .. } => format!("unop:{op:?}"),
        NirValue::Const { .. } => "val:Const".to_string(),
        NirValue::Local(_) => "val:Local".to_string(),
        NirValue::LocalRef { .. } => "val:LocalRef".to_string(),
        NirValue::Global { .. } => "val:Global".to_string(),
        NirValue::FunctionRef { .. } => "val:FunctionRef".to_string(),
        NirValue::Closure { .. } => "val:Closure".to_string(),
        NirValue::Capture { .. } => "val:Capture".to_string(),
        NirValue::Checked { .. } => "val:Checked".to_string(),
        NirValue::Constructor { .. } => "val:Constructor".to_string(),
        NirValue::UnionWrap { .. } => "val:UnionWrap".to_string(),
        NirValue::UnionExtract { .. } => "val:UnionExtract".to_string(),
        NirValue::ResultIsOk { .. } => "val:ResultIsOk".to_string(),
        NirValue::ResultValue { .. } => "val:ResultValue".to_string(),
        NirValue::ResultError { .. } => "val:ResultError".to_string(),
        NirValue::WithUpdate { .. } => "val:WithUpdate".to_string(),
        NirValue::ListLiteral { .. } => "val:ListLiteral".to_string(),
        NirValue::SetLiteral { .. } => "val:SetLiteral".to_string(),
        NirValue::MapLiteral { .. } => "val:MapLiteral".to_string(),
        NirValue::MemberAccess { .. } => "val:MemberAccess".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nested frames partition the emitted range: the inner construct is
    /// credited with its own growth and the outer with the remainder, never
    /// with the sum. This is the property the "top five are 68 %" reading rests
    /// on, and it is a pure function of the stack arithmetic.
    #[test]
    fn nested_frames_are_exclusive() {
        // Drive the arithmetic directly rather than through `crate::trace`,
        // which is a process global other tests share.
        STACK.with(|stack| stack.borrow_mut().clear());
        let outer_own = STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            stack.push(Frame {
                key: "outer".to_string(),
                start: 0,
                child: 0,
            });
            // Inner frame spans instructions 10..90 of the outer's 0..100.
            stack.push(Frame {
                key: "inner".to_string(),
                start: 10,
                child: 0,
            });
            let inner = stack.pop().expect("inner");
            let inner_total = 90usize.saturating_sub(inner.start);
            stack.last_mut().expect("outer").child += inner_total;
            assert_eq!(inner_total, 80);
            let outer = stack.pop().expect("outer");
            100usize.saturating_sub(outer.start) - outer.child
        });
        assert_eq!(outer_own, 20);
        STACK.with(|stack| assert!(stack.borrow().is_empty()));
    }

    /// `exit` without a matching `enter` records nothing rather than panicking,
    /// so a builder panic cannot be masked by a second one from the profiler.
    #[test]
    fn exit_without_enter_is_inert() {
        STACK.with(|stack| stack.borrow_mut().clear());
        exit(42);
        STACK.with(|stack| assert!(stack.borrow().is_empty()));
    }
}
