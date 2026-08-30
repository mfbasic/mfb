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
//! forwarding and redundant-load rows share), and [`bits`] (the known-bits
//! lattice behind known-bits simplification, narrowing, and extension
//! elimination), and [`ranges`] (the integer range lattice with
//! dominating-predicate refinement, the fact base of the check-elision
//! cluster), and [`mirloops`] (natural loops over the flat MIR CFG — the
//! desugared and inlined ones included, which the structured Opt1 loop facts
//! cannot see).

pub(crate) mod bits;
pub(crate) mod mark;
pub(crate) mod memory;
pub(crate) mod mirloops;
pub(crate) mod postdom;
pub(crate) mod ranges;
pub(crate) mod ssa;
