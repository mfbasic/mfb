//! Shared OS-seam (runtime-call) framework.
//!
//! An OS-seam member lowers to a `_mfb_rt_<pkg>_*` runtime helper whose body is
//! **arch-neutral** `abi::` code that branches only on OS family (libc vs
//! kernel32). Each member owns that emission in its `func_*.rs` via its
//! `Body::Native` (`posix`/`win`) on the clean-room registry; this module is the
//! generic dispatch the shared runtime-call machinery calls into, replacing the
//! per-package `lower_<pkg>_call` match arms and the `process_dispatch!` family
//! macro.

use std::collections::HashMap;

use crate::codegen::engine::builder::HelperResult;
use crate::codegen::engine::types::CodegenPlatform;

pub(crate) mod ffi;
pub(crate) mod process;
pub(crate) mod syscall;

/// Emit the runtime-helper body for `call` from the owning member's `Body::Native`
/// lowering, chosen by `platform.family()`. `call` may be the member's own name or
/// one of the auxiliary code-form symbols it covers (e.g. `process.spawnEnv`).
/// Returns `None` when no migrated OS-seam member owns `call`, so the caller can
/// fall back to the legacy runtime-call dispatch for not-yet-migrated packages.
///
/// The aux code-form routing (`process.spawnEnv` → the `spawn` member's lowering) is
/// registry data — each OS-seam member declares its aux runtime-call names in its
/// `Body::Native` `os_aliases` — so this dispatch is a single generic
/// `registry::os_helper` call with no per-package branch.
pub(crate) fn dispatch_runtime_helper(
    call: &str,
    symbol: &str,
    ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Option<HelperResult> {
    crate::codegen::registry::os_helper(call, symbol, ctx, platform_imports, platform)
}
