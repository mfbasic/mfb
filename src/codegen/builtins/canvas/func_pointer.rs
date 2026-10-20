//! `canvas::pointerSnapshot` and `canvas::publishPointer` — the graphics thread's view
//! of the pointer (plan-172-J).
//!
//! Internal-only. Each platform's main thread publishes where the pointer is into the
//! graphics-state block (`runtime::canvas::emit_publish_pointer`), whether or not the
//! program enabled the mouse; `pointerSnapshot` is how the graphics thread reads it,
//! as one consistent set of four values.
//!
//! `publishPointer` is the same writer called from MFBASIC. Its one caller is the
//! `--debug` pointer script (`helper_render/render_loop.rs:__canvas_pointerScript`), which stands
//! in for a mouse on a headless run; a normal build's source never names it.
//!
//! `canvas::autoPublish` (plan-172-K) is the word that turns a pointer change into a
//! redraw: the graphics thread sets it while the scene it drew holds an `AutoPaint`,
//! and each platform's pointer handler reads it after publishing.
//!
//! None carries the `Mode.Canvas` gate: they run on the graphics thread, whose
//! presentation-mode word is its own zero (`.ai/canvas-threading.md` section 2), so a
//! gate there would raise on every frame.

use super::scene_base::orders_scene_accesses;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::codegen::runtime::canvas::{
    emit_publish_pointer, state_base, PointerRegs, GRAPHICS_OFFSET_AUTOPAINT_LIVE,
    GRAPHICS_OFFSET_AUTO_WINDOW_MS, GRAPHICS_OFFSET_POINTER_SEQ, GRAPHICS_POINTER_WORDS,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

fn ok_return(builder: &mut CodeBuilder, text: &str) -> ValueResult {
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
        text: text.to_string(),
    }
}

/// `canvas::pointerSnapshot() AS List OF Integer` — `[x, y, buttons, inside]`.
///
/// A sequence-lock read: the sequence word, the four values, the sequence word again,
/// retried until the two reads are equal and even. The writer's window is four stores,
/// so a retry is rare and short; nothing here blocks.
///
/// The list is reserved FIRST. The reservation is the one call this function makes,
/// and a snapshot taken before it would be a snapshot from before the allocation —
/// harmless, but the values would also have had to survive the call.
pub(crate) fn lower_pointer_snapshot(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ordered = orders_scene_accesses(builder);

    let n_slot = builder.allocate_stack_object("canvas_pointer_n", 8);
    let n = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &n,
        "Integer",
        &GRAPHICS_POINTER_WORDS.len().to_string(),
    ));
    builder.emit(abi::store_u64(&n, abi::stack_pointer(), n_slot));
    let list = builder.reserve_integer_index_list(n_slot)?;
    let list_slot = builder.allocate_stack_object("canvas_pointer_list", 8);
    builder.emit(abi::store_u64(
        &list.location,
        abi::stack_pointer(),
        list_slot,
    ));
    let words = builder.temporary_vreg();
    // A `List OF Integer` is entry-free: word i at HEADER + 8*i.
    builder.emit(abi::load_u64(&words, abi::stack_pointer(), list_slot));
    builder.emit(abi::add_immediate(&words, &words, COLLECTION_HEADER_SIZE));

    let base = builder.temporary_vreg();
    let at = builder.temporary_vreg();
    let before = builder.temporary_vreg();
    let after = builder.temporary_vreg();
    let value = builder.temporary_vreg();
    let retry = builder.label("canvas_pointer_retry");
    state_base(
        &symbol,
        &base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    let load = |builder: &mut CodeBuilder,
                dst: &crate::codegen::engine::operand::VirtualRegister,
                offset: usize| {
        if ordered {
            builder.emit(abi::add_immediate(&at, &base, offset));
            builder.emit(abi::load_acquire_u64(dst, &at));
        } else {
            builder.emit(abi::load_u64(dst, &base, offset));
        }
    };
    builder.emit(abi::label(&retry));
    load(builder, &before, GRAPHICS_OFFSET_POINTER_SEQ);
    // Odd: a publish is between its two marks.
    builder.emit(abi::move_immediate(&value, "Integer", "1"));
    builder.emit(abi::and_registers(&value, &before, &value));
    builder.emit(abi::compare_immediate(&value, "0"));
    builder.emit(abi::branch_ne(&retry));
    for (index, offset) in GRAPHICS_POINTER_WORDS.into_iter().enumerate() {
        load(builder, &value, offset);
        builder.emit(abi::store_u64(&value, &words, 8 * index));
    }
    load(builder, &after, GRAPHICS_OFFSET_POINTER_SEQ);
    builder.emit(abi::compare_registers(&before, &after));
    builder.emit(abi::branch_ne(&retry));

    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), list_slot));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result));
    Ok(ok_return(builder, "canvas.pointerSnapshot"))
}

/// `canvas::publishPointer(x, y, buttons, inside)` — the platform publish, from MFBASIC.
pub(crate) fn lower_publish_pointer(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if args.len() < 4 {
        return Err(format!("'{symbol}' expects x, y, buttons and inside"));
    }
    let ordered = orders_scene_accesses(builder);
    // The four arguments leave the argument bank first: the writer names three
    // registers it overwrites, and a vreg may be given any of them.
    let mut held = Vec::with_capacity(4);
    for arg in &args[..4] {
        let reg = builder.temporary_vreg();
        builder.emit(abi::move_register(&reg, &arg.location));
        held.push(reg);
    }
    let base = builder.temporary_vreg();
    let seq = builder.temporary_vreg();
    let addr = builder.temporary_vreg();
    emit_publish_pointer(
        &symbol,
        ordered,
        &PointerRegs {
            x: (&held[0]).into(),
            y: (&held[1]).into(),
            buttons: (&held[2]).into(),
            inside: (&held[3]).into(),
            base: (&base).into(),
            seq: (&seq).into(),
            addr: (&addr).into(),
        },
        &mut builder.instructions,
        &mut builder.relocations,
    );
    Ok(ok_return(builder, "canvas.publishPointer"))
}

/// `canvas::autoPublish(live)` — say whether the scene just drawn holds an
/// `AutoPaint` (`GRAPHICS_OFFSET_AUTOPAINT_LIVE`). One plain store: the word is a hint,
/// and what makes a stale read of it harmless is in that constant's own comment.
pub(crate) fn lower_auto_publish(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let live = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects whether an AutoPaint is live"))?;
    let value = builder.temporary_vreg();
    builder.emit(abi::move_register(&value, &live.location));
    let base = builder.temporary_vreg();
    state_base(
        &symbol,
        &base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::store_u64(
        &value,
        &base,
        GRAPHICS_OFFSET_AUTOPAINT_LIVE,
    ));
    Ok(ok_return(builder, "canvas.autoPublish"))
}

/// `canvas::autoWindowMs() AS Integer` — the project's `options.canvasAutoPaintMS`,
/// read from the word the build baked into the graphics state
/// (`GRAPHICS_OFFSET_AUTO_WINDOW_MS`).
pub(crate) fn lower_auto_window_ms(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let base = builder.temporary_vreg();
    state_base(
        &symbol,
        &base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    let value = builder.allocate_register();
    builder.emit(abi::load_u64(&value, &base, GRAPHICS_OFFSET_AUTO_WINDOW_MS));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(value),
        text: "canvas.autoWindowMs".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "autoWindowMs",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_auto_window_ms),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "autoPublish",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "live",
                desc: "",
                aliases: &[],
                ty: ParameterType::Boolean,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_auto_publish),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "pointerSnapshot",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::list_of(ParameterType::Integer),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_pointer_snapshot),
        }],
    });
    let integer = |name: &'static str| Parameter {
        name,
        desc: "",
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    };
    pkg.add_function(RegistryFunction {
        name: "publishPointer",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                integer("x"),
                integer("y"),
                integer("buttons"),
                integer("inside"),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_publish_pointer),
        }],
    });
}
