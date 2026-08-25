//! Optimization-only MIR analyses ("plans") the Opt2 rows consume.
//!
//! Nothing here is needed to compile a program — the compile-required
//! instruction-effect model, CFG builder, and liveness stay with the register
//! allocator (`codegen::engine::regalloc::analysis`), and these analyses
//! *reuse* them (single vocabulary, no drift) rather than restating field
//! roles or terminators. Today: [`mark`] (def-use mark-live over a selected
//! pre-regalloc stream, the core of both DCE rows) and [`postdom`]
//! (postdominators + control dependence over the allocator's CFG, the extra
//! fact ADCE needs).

pub(crate) mod mark;
pub(crate) mod postdom;
