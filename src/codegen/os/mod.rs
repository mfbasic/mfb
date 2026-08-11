//! Shared OS-seam (runtime-call) framework.
//!
//! An OS-seam member lowers to a `_mfb_rt_<pkg>_*` runtime helper whose body is
//! **arch-neutral** `abi::` code that branches only on OS family (libc vs
//! kernel32). Each member owns that emission in its `func_*.rs` via
//! [`Implementation::Os`] (`posix`/`win`); this module is the generic dispatch the
//! shared runtime-call machinery calls into, replacing the per-package
//! `lower_<pkg>_call` match arms and the `process_dispatch!` family macro.

use std::collections::HashMap;

use crate::codegen::registry::{Implementation, REGISTRY};
use crate::target::shared::code::{CodegenPlatform, HelperResult, PlatformFamily};

/// Emit the runtime-helper body for `call` from the owning member's
/// [`Implementation::Os`] lowering, chosen by `platform.family()`. `call` may be
/// the member's own name or one of the auxiliary code-form symbols it declares in
/// `Os.all` (e.g. `process.spawnEnv`). Returns `None` when no registered member
/// carries an `Os` implementation covering `call`, so the caller can fall back to
/// the legacy runtime-call dispatch for not-yet-migrated packages.
pub(crate) fn dispatch_runtime_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Option<HelperResult> {
    for module in REGISTRY.modules() {
        for func in module.functions {
            if let Implementation::Os { posix, win, all } = func.implementation {
                if all.contains(&call) {
                    let lower = if platform.family() == PlatformFamily::Windows {
                        win
                    } else {
                        posix
                    };
                    return Some(lower(symbol, platform_imports, platform));
                }
            }
        }
    }
    None
}
