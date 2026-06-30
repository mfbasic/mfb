//! The x86-64 (System V / Linux) code-generation backend — plan-00-H.
//!
//! The first *new* ISA after AArch64, and the real test of the MIR's
//! neutrality. It is additive: it consumes the same neutral MIR
//! (`mir::MirInstruction`) the builders/helpers produce, supplies its own
//! [`regmodel::X86_64RegisterModel`] for the shared allocator, selects MIR into
//! x86-64 machine ops, and encodes them — with no edits to the AArch64 backend
//! or the shared lowering at the selection/allocation sites (those dispatch
//! through `mir::Backend`).
//!
//! Brought up in phases (plan-00-H §4): scalar integer core first, then float,
//! then `v128` (SSE2/FMA3/SSE4.1).

pub(crate) mod regmodel;
