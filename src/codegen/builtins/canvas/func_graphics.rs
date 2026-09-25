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
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::codegen::runtime::canvas::metal::emit_metal_available;
use crate::codegen::runtime::canvas::vulkan::emit_vulkan_ready;
use crate::codegen::runtime::canvas::{
    emit_display_milli_hz, emit_frame_counters, emit_frame_done, emit_note_present,
    emit_set_display_override, emit_set_gpu_mode, FrameRecord, emit_set_sync_mode, emit_signal_redraw,
    emit_start_graphics, emit_surface_dimension, emit_sync_frame, emit_use_gpu,
    emit_wait_for_redraw, GraphicsScratch, DEFAULT_SURFACE_HEIGHT, DEFAULT_SURFACE_WIDTH,
    GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_WIDTH,
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
        ctx.debug_arena_registry,
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

/// Copy argument `index` into a fresh vreg: an argument arrives in an argument
/// register, and every emitter here makes a mutex call it would not survive.
fn argument_vreg(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    index: usize,
    what: &str,
) -> Result<String, String> {
    let location = args
        .get(index)
        .ok_or_else(|| format!("'{}' expects the {what} argument", builder.current_symbol))?
        .location
        .clone();
    let value = builder.temporary_vreg().to_string();
    builder.emit(abi::move_register(&value, &location));
    Ok(value)
}

/// `canvas::frameDone(drew, renderer, renderNanos, now)` — the render loop reports a
/// completed frame, and (plan-155-B) whether and how it was drawn.
pub(crate) fn lower_frame_done(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let drew = argument_vreg(builder, args, 0, "drew")?;
    let renderer = argument_vreg(builder, args, 1, "renderer")?;
    let render_nanos = argument_vreg(builder, args, 2, "renderNanos")?;
    let now = argument_vreg(builder, args, 3, "now")?;
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_frame_done(
        &symbol,
        &scratch,
        &FrameRecord {
            drew: &drew,
            renderer: &renderer,
            render_nanos: &render_nanos,
            now: &now,
        },
        ctx.platform_imports,
        ctx.platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    Ok(ok_return(builder, symbol))
}

/// `canvas::notePresent(now)` — count a present for `canvas::getFrameStats`.
pub(crate) fn lower_note_present(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let now = argument_vreg(builder, args, 0, "now")?;
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_note_present(
        &symbol,
        &scratch,
        &now,
        ctx.platform_imports,
        ctx.platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    Ok(ok_return(builder, symbol))
}

/// `canvas::frameCounters(now) AS List OF Integer` — the frame statistics, one lock.
pub(crate) fn lower_frame_counters(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let now = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the now argument"))?
        .location
        .clone();
    emit_frame_counters(builder, &now, ctx.platform_imports, ctx.platform)?;
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

/// One `Integer` parameter of an internal member.
fn integer_param(name: &'static str) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    }
}

/// `canvas::syncFrame()` — wait for the frame this present asked for, in sync mode.
pub(crate) fn lower_sync_frame(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_sync_frame(
        &symbol,
        &scratch,
        ctx.platform_imports,
        ctx.platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    Ok(ok_return(builder, symbol))
}

/// `canvas::setSyncMode(on AS Boolean)` — see `emit_set_sync_mode`.
pub(crate) fn lower_set_sync_mode(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let value = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the flag argument"))?
        .location
        .clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_set_sync_mode(
        &symbol,
        &scratch,
        &value,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    Ok(ok_return(builder, symbol))
}

/// `canvas::surfaceWidth()` / `canvas::surfaceHeight()` — the surface's current size.
///
/// Read from the graphics state rather than returned as a constant, which is what
/// makes a resize visible to the renderer without the program doing anything.
fn lower_dimension(
    builder: &mut CodeBuilder,
    offset: usize,
    default: usize,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_surface_dimension(
        &symbol,
        &scratch,
        offset,
        default,
        &mut builder.instructions,
        &mut builder.relocations,
    );
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

pub(crate) fn lower_surface_width(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_dimension(builder, GRAPHICS_OFFSET_WIDTH, DEFAULT_SURFACE_WIDTH)
}

pub(crate) fn lower_surface_height(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_dimension(builder, GRAPHICS_OFFSET_HEIGHT, DEFAULT_SURFACE_HEIGHT)
}

/// `canvas::displayMilliHz() AS Integer` — the display's refresh rate in millihertz
/// (plan-155-A): the `MFB_CANVAS_DISPLAY_HZ` override when set, else what the main
/// thread published, else `0`.
pub(crate) fn lower_display_milli_hz(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_display_milli_hz(
        &symbol,
        &scratch,
        &mut builder.instructions,
        &mut builder.relocations,
    );
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

/// `canvas::setDisplayOverride(mhz AS Integer)` — see `emit_set_display_override`.
pub(crate) fn lower_set_display_override(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let value = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the millihertz argument"))?
        .location
        .clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_set_display_override(
        &symbol,
        &scratch,
        &value,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    Ok(ok_return(builder, symbol))
}

/// An internal member returning an `Integer`.
fn internal_integer(name: &'static str, body: Body) -> RegistryFunction {
    RegistryFunction {
        name,
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body,
        }],
    }
}

/// `canvas::setGpuMode(on AS Boolean)` — select the Metal renderer.
pub(crate) fn lower_set_gpu_mode(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let value = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the flag argument"))?
        .location
        .clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_set_gpu_mode(
        &symbol,
        &scratch,
        &value,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    Ok(ok_return(builder, symbol))
}

/// `canvas::useGpu() AS Boolean` — the renderer seam's discriminant.
pub(crate) fn lower_use_gpu(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg().to_string());
    emit_use_gpu(
        &symbol,
        &scratch,
        &mut builder.instructions,
        &mut builder.relocations,
    );
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

/// `canvas::metalAvailable() AS Boolean` — is a Metal device obtainable?
pub(crate) fn lower_metal_available(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    emit_metal_available(
        &symbol,
        ctx.platform,
        ctx.platform_imports,
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

/// `canvas::metalReady() AS Boolean` — is the Metal renderer built and usable?
///
/// Builds the device, command queue and render pipeline on the first call and
/// reports whether they exist; every later call reports the remembered answer. It is
/// the second half of the renderer branch's condition (`canvas::useGpu` is the
/// first): asked for *and* actually available.
///
/// A target with no Metal implementation has no seam, and reports `FALSE` — which is
/// the honest answer and keeps `__canvas_renderFrame` one shape everywhere rather
/// than a per-target `IF`.
pub(crate) fn lower_metal_ready(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    match ctx
        .platform
        .emit_metal_init(&symbol, &mut builder.instructions, &mut builder.relocations)
    {
        Some(result) => {
            result?;
            builder.emit(abi::move_register(RESULT_VALUE_REGISTER, abi::c_return(0)));
        }
        None => builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0")),
    }
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

/// `canvas::vulkanReady() AS Boolean` — is the Vulkan device built and usable?
///
/// The Vulkan twin of `canvas::metalReady`. Builds the instance, physical device,
/// logical device and graphics queue on the first call and reports whether they
/// exist; every later call reports the remembered answer.
pub(crate) fn lower_vulkan_ready(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    emit_vulkan_ready(builder, ctx.platform, ctx.platform_imports)?;
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
        name: "setSyncMode",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "on",
                desc: "",
                aliases: &[],
                ty: ParameterType::Boolean,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_sync_mode),
        }],
    });
    pkg.add_function(internal_integer(
        "surfaceWidth",
        Body::abi_function(lower_surface_width),
    ));
    pkg.add_function(internal_integer(
        "surfaceHeight",
        Body::abi_function(lower_surface_height),
    ));
    pkg.add_function(internal_integer(
        "displayMilliHz",
        Body::abi_function(lower_display_milli_hz),
    ));
    pkg.add_function(RegistryFunction {
        name: "setDisplayOverride",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "mhz",
                desc: "",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_display_override),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "setGpuMode",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "on",
                desc: "",
                aliases: &[],
                ty: ParameterType::Boolean,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_gpu_mode),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "metalAvailable",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_metal_available),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "vulkanReady",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_vulkan_ready),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "metalReady",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_metal_ready),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "useGpu",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_use_gpu),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "frameDone",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "drew",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Boolean,
                    default: DefaultValue::None,
                },
                integer_param("renderer"),
                integer_param("renderNanos"),
                integer_param("now"),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_frame_done),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "notePresent",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![integer_param("now")],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_note_present),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "frameCounters",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![integer_param("now")],
            return_type: ParameterType::list_of(ParameterType::Integer),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_frame_counters),
        }],
    });
    pkg.add_function(internal("syncFrame", Body::abi_function(lower_sync_frame)));
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
