//! `codegen::compiler::opt` module wiring.
//!
//! Only the read-only `MFB_BUG387_SELFMOVE` diagnostic lives here. The three
//! transforms that used to share this module (`fuse_scalar_fma`,
//! `forward_stores_to_loads`, `remove_fp_shuttles`) moved to
//! `crate::optimizer::opt2` when they went onto the `-O` dial (plan-100 §3);
//! the probe stayed because it is not a pass and is not level-gated.

pub(crate) mod selfmove_probe;
pub(crate) use selfmove_probe::*;
