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

use crate::target::shared::code::{CodegenPlatform, HelperResult};

/// Emit the runtime-helper body for `call` from the owning member's `Body::Native`
/// lowering, chosen by `platform.family()`. `call` may be the member's own name or
/// one of the auxiliary code-form symbols it covers (e.g. `process.spawnEnv`).
/// Returns `None` when no migrated OS-seam member owns `call`, so the caller can
/// fall back to the legacy runtime-call dispatch for not-yet-migrated packages.
///
/// `process` is the sole OS-seam package on the clean-room registry, and its
/// `Body::Native` posix/win slots have no generic covered-symbol (`all`) list, so
/// the aux code-form routing lives in the package module; delegate to it.
pub(crate) fn dispatch_runtime_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Option<HelperResult> {
    crate::codegen::builtins::process::dispatch_os_helper(call, symbol, platform_imports, platform)
}
