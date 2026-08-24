//! `codegen::compiler::opt` module wiring.
//!
//! What is left here is everything that is **not** on the `-O` dial. The two
//! machine peepholes (`forward_stores_to_loads`, `remove_fp_shuttles`) moved to
//! `crate::optimizer::opt2` when they became gated Level-1 rows (plan-100 §3).
//! These two stayed:
//!
//! * `fma_fusion` -- mandatory lowering, not an optimization. Contraction rounds
//!   once instead of twice, so gating it would make `-O0` emit different float
//!   results; two fixtures pin the fused semantics as a contract. See the
//!   `fuse_scalar_fma` doc comment.
//! * `selfmove_probe` -- a read-only `MFB_BUG387_SELFMOVE` diagnostic, not a
//!   transform at all.

pub(crate) mod fma_fusion;
pub(crate) mod selfmove_probe;
pub(crate) use selfmove_probe::*;
