//! Optimization-only NIR analyses ("plans") the Opt1 rows consume.
//!
//! Nothing here is needed to compile a program — the compile-required analyses
//! (regalloc liveness, the allocator CFG, resource escape) stay with codegen.
//! These are the demand-driven fact bases plan-100 calls Plan-infrastructure,
//! built when a row needs them: today [`reads`] (the name-usage census behind
//! tree-level DCE).

pub(crate) mod reads;
