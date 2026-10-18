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

/// Whether the target has a Metal renderer: macOS, the one target whose
/// `emit_metal_init` hook is implemented.
pub(crate) fn has_metal_backend(platform: &dyn CodegenPlatform) -> bool {
    platform.family() == PlatformFamily::MacOS
}

/// bug-1087: `GRAPHICS_OFFSET_MTL_READY`'s states. The encoding is bug-996's Vulkan
/// word's (`vulkan_handoff`), and so is the rule it carries: the Metal pipeline is never
/// on a frame's critical path.
///
/// | value | meaning | written by |
/// |---|---|---|
/// | [`METAL_UNTRIED`] | nobody has asked for a GPU frame yet | the linker (zero) |
/// | [`METAL_BUILDING`] | the build has been claimed and is running | the graphics thread, once |
/// | [`METAL_BUILT`] | the device, queue, pipelines and frame buffer exist | the building thread, as its last store |
/// | [`METAL_FAILED`] | no Metal here; software forever | the building thread, or the graphics thread when no thread could be started |
///
/// Only the graphics thread moves the word off `UNTRIED`, and before the builder thread
/// exists, so there is one builder at most and no compare-and-swap. The builder writes
/// every object word with plain stores and the ready word last, with a store-release;
/// the graphics thread reads the ready word with a load-acquire, and an object word only
/// after it has read `BUILT`.
pub(crate) const METAL_UNTRIED: &str = "0";
pub(crate) const METAL_BUILT: &str = "1";
pub(crate) const METAL_FAILED: &str = "2";
pub(crate) const METAL_BUILDING: &str = "3";

/// `canvas::metalBuild`'s argument (and `_mfb_macapp_metal_init`'s): start the build on
/// a thread of its own unless it has been started already.
///
/// Test-only as a name: the member compares against the two values below and treats
/// every other as this one, so nothing emitted spells it. The render loop does, as a
/// literal (`__canvas_metalReady`), and
/// `the_render_loop_starts_the_metal_build_where_the_handoff_says` holds that literal
/// to this.
#[cfg(test)]
pub(crate) const METAL_BUILD_ON_ITS_OWN_THREAD: &str = "0";
/// Run the build on the calling thread, to its end, unless it has been started already:
/// a program that asked for the GPU by name (`MFB_CANVAS_GPU`).
pub(crate) const METAL_BUILD_ON_THIS_THREAD: &str = "1";
/// Run the build: the claim was made by the thread that started this one. Only the
/// builder thread's entry (`_mfb_macapp_metal_builder`) passes it.
pub(crate) const METAL_BUILD_CLAIMED: &str = "2";

/// `canvas::metalReady() AS Boolean` — is the Metal pipeline built, now?
///
/// bug-1087: one load-acquire and a compare. It starts nothing and waits for nothing,
/// so the render branch asks it every frame and a `--debug` stats line asks it too. A
/// target with no Metal backend answers FALSE.
pub(crate) fn emit_metal_is_ready(
    symbol: &str,
    platform: &dyn CodegenPlatform,
    state: abi::Reg,
    value: abi::Reg,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    if !has_metal_backend(platform) {
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
        return;
    }
    let built = format!("{symbol}_mtl_built");
    let done = format!("{symbol}_mtl_answered");
    super::state_base(symbol, state, instructions, relocations);
    instructions.push(abi::add_immediate(
        value,
        state,
        super::GRAPHICS_OFFSET_MTL_READY,
    ));
    instructions.push(abi::load_acquire_u64(value, value));
    instructions.push(abi::compare_immediate(value, METAL_BUILT));
    instructions.push(abi::branch_eq(&built));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    instructions.push(abi::branch(&done));
    instructions.push(abi::label(&built));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    instructions.push(abi::label(&done));
}
