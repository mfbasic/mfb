//! The Metal renderer's device layer (plan-98-E Phase 1).
//!
//! Built bottom-up and in the order each piece can be *tested*: the device first,
//! because it proves the framework plumbing — the `Metal.framework` install name, the
//! import table row, and the `MTLCreateSystemDefaultDevice` symbol binding — before
//! several hundred lines of pipeline setup are written on top of it. A device that
//! cannot be created is a link or dylib problem, and finding that out from a
//! one-call helper is much cheaper than finding it out from a blank window.
//!
//! Everything here is macOS-only. The seam (`canvas::useGpu`) is compiled for
//! every target so the renderer dispatch has one shape, but it reports FALSE
//! anywhere without a Metal path.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// `Metal.framework`, for the import table.
///
/// `CAMetalLayer` lives in `QuartzCore.framework`, not here — its constant arrives
/// with the layer that needs it rather than sitting unused.
pub(crate) const LIB_METAL: &str = "Metal";
/// `id<MTLDevice> MTLCreateSystemDefaultDevice(void)`.
///
/// A C function, not a message send: there is no class to message before a device
/// exists. Returns nil when the process has no Metal-capable GPU — which is a real
/// case (a headless session, some VMs), not a theoretical one, and is exactly why
/// the renderer seam has to be a runtime branch.
pub(crate) const MTL_CREATE_DEVICE: &str = "_MTLCreateSystemDefaultDevice";

/// `canvas::metalAvailable() AS Boolean` — can this process render with Metal?
///
/// Calls `MTLCreateSystemDefaultDevice` and reports whether it returned a device.
/// The device is **not** retained or cached here: this answers a question, and the
/// renderer creates and keeps its own. `MTLCreateSystemDefaultDevice` returns the
/// same system device on every call, so asking twice costs a lookup, not a device.
pub(crate) fn emit_metal_available(
    symbol: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    if platform.family() != PlatformFamily::MacOS {
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
        return Ok(());
    }
    let available = format!("{symbol}_metal_available");
    let done = format!("{symbol}_metal_done");

    instructions.push(abi::branch_link(MTL_CREATE_DEVICE));
    relocations.push(external_branch(
        symbol,
        MTL_CREATE_DEVICE,
        platform_imports,
    )?);
    instructions.push(abi::compare_immediate(abi::c_return(0), "0"));
    instructions.push(abi::branch_ne(&available));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    instructions.push(abi::branch(&done));
    instructions.push(abi::label(&available));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    instructions.push(abi::label(&done));
    Ok(())
}
