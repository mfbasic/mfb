//! `canvas::startGraphics` / `signalRedraw` / `waitForRedraw` — the graphics
//! thread's three seams (plan-98-D Phase 2).
//!
//! All internal-only: a program presents a scene and the runtime decides when and on
//! which thread to draw it. `.ai/canvas-threading.md` is the protocol these
//! implement.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::codegen::runtime::canvas::{
    emit_signal_redraw, emit_start_graphics, emit_wait_for_redraw, GraphicsScratch,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Finish an internal `Nothing`-returning body.
fn ok_return(builder: &mut CodeBuilder, symbol: String) -> ValueResult {
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    }
}

/// `canvas::startGraphics()` — spawn the render thread if it is not already running.
///
/// Idempotent, and called on every `present` rather than once at `setMode`: the
/// thread is only useful once there is something to draw, and a program that enters
/// canvas mode and never presents should not pay for one.
pub(crate) fn lower_start_graphics(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let fail = builder.label("canvas_graphics_alloc_fail");
    let done = builder.label("canvas_graphics_done");
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_start_graphics(
        &symbol,
        &scratch,
        ctx.arena_global_slots,
        ctx.platform_imports,
        ctx.platform,
        &mut builder.instructions,
        &mut builder.relocations,
        &fail,
    )?;
    builder.emit(abi::branch(&done));
    // Out of memory means no graphics thread, which means the program draws nothing
    // — but it keeps running, and `canvas::present` still publishes. Failing the
    // call instead would turn a memory shortage into a program crash on a path the
    // program never asked to take.
    builder.emit(abi::label(&fail));
    builder.emit(abi::label(&done));
    Ok(ok_return(builder, symbol))
}

/// `canvas::signalRedraw()` — tell the render thread a frame is owed.
pub(crate) fn lower_signal_redraw(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_signal_redraw(
        &symbol,
        &scratch,
        ctx.platform_imports,
        ctx.platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    Ok(ok_return(builder, symbol))
}

/// `canvas::waitForRedraw() AS Boolean` — block until a frame is owed.
///
/// Returns FALSE when shutdown has asked the loop to stop, which is what lets the
/// render loop be an ordinary `WHILE` rather than needing a separate kill path.
pub(crate) fn lower_wait_for_redraw(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_wait_for_redraw(
        &symbol,
        &scratch,
        ctx.platform_imports,
        ctx.platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

fn internal(name: &'static str, body: Body) -> RegistryFunction {
    RegistryFunction {
        name,
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body,
        }],
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(internal(
        "startGraphics",
        Body::abi_function(lower_start_graphics),
    ));
    pkg.add_function(internal(
        "signalRedraw",
        Body::abi_function(lower_signal_redraw),
    ));
    pkg.add_function(RegistryFunction {
        name: "waitForRedraw",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_wait_for_redraw),
        }],
    });
}
