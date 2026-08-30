//! Optimization-only NIR analyses ("plans") the Opt1 rows consume.
//!
//! Nothing here is needed to compile a program — the compile-required analyses
//! (regalloc liveness, the allocator CFG, resource escape) stay with codegen.
//! These are the demand-driven fact bases plan-100 calls Plan-infrastructure,
//! built when a row needs them: today [`reads`] (the name-usage census behind
//! tree-level DCE and LICM's scope-safety check) and [`loops`] (invariance,
//! loop-control capture, and the pure statement class the loop rows consume).

pub(crate) mod globals;
pub(crate) mod loops;
pub(crate) mod reads;
