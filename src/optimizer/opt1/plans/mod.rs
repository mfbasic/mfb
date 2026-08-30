//! Optimization-only NIR analyses ("plans") the Opt1 rows consume.
//!
//! Nothing here is needed to compile a program — the compile-required analyses
//! (regalloc liveness, the allocator CFG, resource escape) stay with codegen.
//! These are the demand-driven fact bases plan-100 calls Plan-infrastructure,
//! built when a row needs them: [`reads`] (the name-usage census behind
//! tree-level DCE and LICM's scope-safety check), [`loops`] (invariance,
//! loop-control capture, and the pure statement class the loop rows consume),
//! [`globals`] (the whole-module global census), and [`shape`] (the
//! own-values / owned-bodies accessors every rewriting walk needs to tell
//! "here" from "inside").

pub(crate) mod globals;
pub(crate) mod loops;
pub(crate) mod reads;
pub(crate) mod shape;
