//! The process-global loaded-font table: register on load, drop on release, look up
//! by the handle a `FontRef` carries.
//!
//! `canvas::loadFont` runs on the **worker** and the geometry cache that needs a
//! glyph's outline runs on the **graphics thread**. The bytes are already reachable
//! from both — an arena block is ordinary process memory, and only the *allocator
//! state* is per-thread — but the graphics thread has no way to get from the integer
//! in a `FontRef` to the block. This table is that map, and it is a process-global for
//! exactly the reason the scene region is one (`.ai/canvas-threading.md` §2).
//!
//! Sixteen fixed slots, `handle` then `block`, no count word: a zero handle is the
//! empty slot, and scanning sixteen words is cheaper than keeping a count consistent
//! between two threads without a lock. Registering is a single 16-byte write into a
//! slot no other thread is writing, and releasing is a single store of zero — both are
//! naturally atomic enough for a reader that only ever asks "is this handle here".

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::push_symbol_address;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryHelper,
    RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Park the font table's address in a fresh vreg.
fn table_base(builder: &mut CodeBuilder) -> VirtualRegister {
    let base = builder.temporary_vreg();
    let symbol = builder.current_symbol.clone();
    push_symbol_address(
        &symbol,
        CANVAS_FONTS_SYMBOL,
        &base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    base
}

/// Claim the first free slot for `handle` -> `block`.
///
/// A full table is **not** an error. `loadFont` still returns a working `Font` — it
/// measures, because `measureText` reads the resource record directly — and only text
/// *drawn* in it comes out empty, which is the same thing that happens to a font the
/// program released while a scene still named it. Failing the load instead would turn
/// a rendering limit into a program-stopping error at a point the program cannot
/// predict.
pub(crate) fn emit_register_font(builder: &mut CodeBuilder, handle: &Operand, block: &Operand) {
    let scan = builder.label("canvas_font_table_scan");
    let next = builder.label("canvas_font_table_next");
    let done = builder.label("canvas_font_table_done");

    let base = table_base(builder);
    let slot = builder.temporary_vreg();
    let limit = builder.temporary_vreg();
    let occupant = builder.temporary_vreg();
    builder.emit(abi::move_register(&slot, &base));
    builder.emit(abi::add_immediate(&limit, &base, CANVAS_FONT_TABLE_BYTES));

    builder.emit(abi::label(&scan));
    builder.emit(abi::compare_registers(&slot, &limit));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::load_u64(&occupant, &slot, CANVAS_FONT_SLOT_HANDLE));
    builder.emit(abi::compare_immediate(&occupant, "0"));
    builder.emit(abi::branch_ne(&next));
    // The block goes in first. A reader that saw the handle appear before the block
    // would follow a null pointer, and the two stores are not one operation.
    builder.emit(abi::store_u64(block, &slot, CANVAS_FONT_SLOT_BLOCK));
    builder.emit(abi::store_u64(handle, &slot, CANVAS_FONT_SLOT_HANDLE));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&slot, &slot, CANVAS_FONT_SLOT_BYTES));
    builder.emit(abi::branch(&scan));

    builder.emit(abi::label(&done));
}

/// Clear the slot naming `handle`, if it is there.
///
/// Called from `destroyFont`, so that text still naming a released font draws empty
/// rather than reading a block the program has finished with. Clearing the *handle*
/// is what unpublishes it: a reader matches on the handle, so the block word left
/// behind is unreachable, and zeroing it too would be two stores where one does.
pub(crate) fn emit_unregister_font(builder: &mut CodeBuilder, handle: &Operand) {
    let scan = builder.label("canvas_font_drop_scan");
    let next = builder.label("canvas_font_drop_next");
    let done = builder.label("canvas_font_drop_done");

    let base = table_base(builder);
    let slot = builder.temporary_vreg();
    let limit = builder.temporary_vreg();
    let occupant = builder.temporary_vreg();
    builder.emit(abi::move_register(&slot, &base));
    builder.emit(abi::add_immediate(&limit, &base, CANVAS_FONT_TABLE_BYTES));

    builder.emit(abi::label(&scan));
    builder.emit(abi::compare_registers(&slot, &limit));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::load_u64(&occupant, &slot, CANVAS_FONT_SLOT_HANDLE));
    builder.emit(abi::compare_registers(&occupant, handle));
    builder.emit(abi::branch_ne(&next));
    builder.emit(abi::store_u64(abi::ZERO, &slot, CANVAS_FONT_SLOT_HANDLE));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&slot, &slot, CANVAS_FONT_SLOT_BYTES));
    builder.emit(abi::branch(&scan));

    builder.emit(abi::label(&done));
}

/// Find the block registered for `handle`, or zero.
///
/// Zero is the answer for a handle that was never registered, one whose font was
/// released, and the `0` a `FontRef` holds when nothing was ever assigned to it. All
/// three mean the same thing to a caller — there is nothing to draw — so they get the
/// same answer rather than three.
pub(crate) fn emit_lookup_font(builder: &mut CodeBuilder, handle: &Operand, out: &VirtualRegister) {
    let scan = builder.label("canvas_font_find_scan");
    let next = builder.label("canvas_font_find_next");
    let done = builder.label("canvas_font_find_done");

    let base = table_base(builder);
    let slot = builder.temporary_vreg();
    let limit = builder.temporary_vreg();
    let occupant = builder.temporary_vreg();
    builder.emit(abi::move_immediate(out, "Integer", "0"));
    builder.emit(abi::compare_immediate(handle, "0"));
    builder.emit(abi::branch_eq(&done));
    builder.emit(abi::move_register(&slot, &base));
    builder.emit(abi::add_immediate(&limit, &base, CANVAS_FONT_TABLE_BYTES));

    builder.emit(abi::label(&scan));
    builder.emit(abi::compare_registers(&slot, &limit));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::load_u64(&occupant, &slot, CANVAS_FONT_SLOT_HANDLE));
    builder.emit(abi::compare_registers(&occupant, handle));
    builder.emit(abi::branch_ne(&next));
    builder.emit(abi::load_u64(out, &slot, CANVAS_FONT_SLOT_BLOCK));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&slot, &slot, CANVAS_FONT_SLOT_BYTES));
    builder.emit(abi::branch(&scan));

    builder.emit(abi::label(&done));
}

/// `canvas::fontRegistered(id) AS Boolean` — is there a blob for this handle?
///
/// Split from the blob read so the MFBASIC wrapper can answer "no font" with an
/// ordinary `[]` literal. Building an empty collection from an emitter would mean
/// hand-writing a header the language already knows how to write, for the one case
/// that is not a font at all.
pub(crate) fn lower_font_registered(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let handle = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the font handle"))?
        .location
        .clone();

    let found = builder.temporary_vreg();
    emit_lookup_font(builder, &handle, &found);
    let no = builder.label("canvas_font_registered_no");
    let done = builder.label("canvas_font_registered_done");
    builder.emit(abi::compare_immediate(&found, "0"));
    builder.emit(abi::branch_eq(&no));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&no));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    builder.emit(abi::label(&done));
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
        text: "canvas.fontRegistered".to_string(),
    })
}

/// `canvas::fontBlobUnchecked(id) AS List OF Byte` — an owned copy of the font file.
///
/// It copies, for the reason `canvas::fontBytes` does: the returned value binds to an
/// ordinary `LET`, and that binding's scope-drop reclaims it — so returning the table's
/// own block would hand the *next* caller a block the previous one freed and the arena
/// filled with noise. Measured there, not reasoned (exit 139 on the second call).
///
/// The copy is paid once per geometry-cache **miss**, not per glyph: the caller reads
/// every glyph of a string out of one copy, and the cache means a string is only walked
/// again when it changes.
///
/// `Unchecked` because it must be called with a handle `canvas::fontRegistered` just
/// said yes to; on any other it would copy from a null pointer.
pub(crate) fn lower_font_blob_unchecked(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let handle = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the font handle"))?
        .location
        .clone();

    let block = builder.temporary_vreg();
    emit_lookup_font(builder, &handle, &block);
    let copy = builder.copy_flat_block(&ParameterType::list_of(ParameterType::Byte), &block)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &copy));
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
        text: "canvas.fontBlobUnchecked".to_string(),
    })
}

/// The safe front door the geometry builder calls.
#[rustfmt::skip]
const FONT_BLOB: &str =
r#"FUNC __canvas_fontBlob(id AS Integer) AS List OF Byte
  IF NOT canvas::fontRegistered(id) THEN
    RETURN []
  END IF
  RETURN canvas::fontBlobUnchecked(id)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    for (name, intro, lower) in [(
        "fontRegistered",
        "Is a font blob published for this handle?",
        lower_font_registered
            as fn(&mut CodeBuilder, &[ValueResult], &AbiCtx) -> Result<ValueResult, String>,
    )] {
        pkg.add_function(RegistryFunction {
            name,
            intro,
            desc: "Internal. Reads the process-global font table the worker publishes \
                   into and the graphics thread reads.",
            example: "",
            expected_arguments: None,
            internal_only: true,
            implementations: vec![Implementation {
                params: vec![Parameter {
                    name: "id",
                    desc: "The handle a `canvas::FontRef` carries.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: Body::abi_function(lower),
            }],
        });
    }
    pkg.add_function(RegistryFunction {
        name: "fontBlobUnchecked",
        intro: "The published font file for a handle, as an owned copy.",
        desc: "Internal, and only valid for a handle `canvas::fontRegistered` has \
               answered yes to.",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "id",
                desc: "The handle a `canvas::FontRef` carries.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::abi_function(lower_font_blob_unchecked),
        }],
    });
    pkg.add_helper(RegistryHelper::always("canvas_fontBlob", FONT_BLOB));
}
