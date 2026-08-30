//! Optimization-only MIR analyses ("plans") the Opt2 rows consume.
//!
//! Nothing here is needed to compile a program — the compile-required
//! instruction-effect model, CFG builder, and liveness stay with the register
//! allocator (`codegen::engine::regalloc::analysis`), and these analyses
//! *reuse* them (single vocabulary, no drift) rather than restating field
//! roles or terminators. Today: [`mark`] (def-use mark-live over a selected
//! pre-regalloc stream, the core of both DCE rows), [`postdom`]
//! (postdominators + control dependence over the allocator's CFG, the extra
//! fact ADCE needs), and [`ssa`] (the Plan2 SSA overlay — forward dominators,
//! phi placement, per-use value resolution, and copy-forwarding facts — that
//! the propagation rows and precise DCE marking consume), and [`memory`]
//! (stack-slot value availability, the fact base the store-to-load
//! forwarding and redundant-load rows share).

pub(crate) mod mark;
pub(crate) mod memory;
pub(crate) mod postdom;
pub(crate) mod ssa;
