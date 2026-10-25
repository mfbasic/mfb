//! plan-175-A Phase 5: a group's items, read IN PLACE on the graphics thread.
//!
//! `canvas::groupItems(slot)` copies a group's whole item list, and the walk used to call
//! it once per `canvas::Group` node per frame: 10,000 nodes naming one group were 10,000
//! list copies (and 10,000 frees, each scrubbing what it returns) a frame. These three
//! read the slot's items block where it is instead:
//!
//! * `canvas::groupItemsBlock(slot) AS Integer` — the block's address, `0` for an empty
//!   or out-of-range slot. Taken ONCE per node, after `canvas::groupRevision(slot)`, so
//!   every child that node reads is one `setGroup`'s: a replacement landing mid-walk
//!   moves the slot to a new block, and this node keeps reading the one it took.
//! * `canvas::groupBlockCount(block) AS Integer` — its item count, `0` for no block.
//! * `canvas::groupBlockItem(block, k, none) AS DrawItem` — its item `k`, or `none`.
//!
//! **Why an address is safe to hold for the frame.** A block a `setGroup` displaces is
//! RETIRED, not freed, and freed only once a frame has completed since
//! (`.ai/canvas-threading.md` §13): the same rule that makes the copy `groupItems`
//! takes safe makes reading the block itself safe until the frame ends. The address
//! never outlives the frame: it lives in a local of the walk, as the snapshot's raw
//! pointers live in `__CANVAS_SNAP_*`.

use super::gen_group::groups_base;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `DrawItem` as the program's type model keys it: the package's nominals are registered
/// qualified, and a bare spelling there is a user's own type (`gen_present.rs`'s
/// `content_type` resolves the scene the same way).
fn item_type(builder: &CodeBuilder) -> Result<ParameterType, String> {
    for spelling in ["canvas.DrawItem", "DrawItem"] {
        let candidate = ParameterType::named(spelling);
        if builder.type_model.union_names.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err("canvas::groupBlockItem: DrawItem has no layout in the type model".into())
}

fn arg(args: &[ValueResult], index: usize, what: &str, symbol: &str) -> Result<Operand, String> {
    Ok(args
        .get(index)
        .ok_or_else(|| format!("'{symbol}' expects the {what} argument"))?
        .location
        .clone())
}

fn integer_result(builder: &mut CodeBuilder, symbol: String) -> ValueResult {
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(RESULT_VALUE_REGISTER),
        text: symbol,
    }
}

/// `canvas::groupItemsBlock(slot) AS Integer`.
pub(crate) fn lower_group_items_block(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let slot_in = arg(args, 0, "slot", &symbol)?;
    let out = builder.label("canvas_group_block_out");
    let index = builder.temporary_vreg();
    builder.emit(abi::move_register(&index, &slot_in));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::compare_immediate(&index, "0"));
    builder.emit(abi::branch_lt(&out));
    builder.emit(abi::compare_immediate(
        &index,
        &CANVAS_MAX_GROUPS.to_string(),
    ));
    builder.emit(abi::branch_ge(&out));
    let base = groups_base(builder);
    let addr = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(
        &addr,
        &index,
        CANVAS_GROUP_SLOT_SHIFT,
    ));
    builder.emit(abi::add_registers(&addr, &base, &addr));
    builder.emit(abi::load_u64(
        RESULT_VALUE_REGISTER,
        &addr,
        CANVAS_GROUP_ITEMS,
    ));
    builder.emit(abi::label(&out));
    Ok(integer_result(builder, symbol))
}

/// `canvas::groupBlockCount(block) AS Integer`.
pub(crate) fn lower_group_block_count(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let block_in = arg(args, 0, "block", &symbol)?;
    let out = builder.label("canvas_group_block_count_out");
    let block = builder.temporary_vreg();
    builder.emit(abi::move_register(&block, &block_in));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::compare_immediate(&block, "0"));
    builder.emit(abi::branch_eq(&out));
    builder.emit(abi::load_u64(
        RESULT_VALUE_REGISTER,
        &block,
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::label(&out));
    Ok(integer_result(builder, symbol))
}

/// `canvas::groupBlockItem(block, k, none) AS DrawItem`: the list reader's own element
/// copy (`lower_list_get_or`) over the block, so an item comes out exactly as
/// `collections::getOr` gives it from the list `groupItems` would have copied.
pub(crate) fn lower_group_block_item(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let block_slot = builder.spill_to_slot("canvas_group_block", &arg(args, 0, "block", &symbol)?);
    let key_slot = builder.spill_to_slot("canvas_group_block_k", &arg(args, 1, "index", &symbol)?);
    let none_slot = builder.spill_to_slot(
        "canvas_group_block_none",
        &arg(args, 2, "default", &symbol)?,
    );
    let element = item_type(builder)?;
    let got = builder.lower_list_get_or(
        block_slot,
        key_slot,
        none_slot,
        &ParameterType::list_of(element.clone()),
        &element,
    )?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &got.location));
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

/// `canvas::groupBlockIsGroup(block, k) AS Boolean`: whether item `k` of `block` is a
/// `Group` node, read from its union tag with nothing copied. `__canvas_drawGroup` walks
/// a node's children only for the nested groups among them, so the others are not
/// copied out at all.
pub(crate) fn lower_group_block_is_group(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let block_in = arg(args, 0, "block", &symbol)?;
    let k_in = arg(args, 1, "index", &symbol)?;
    let group_tag = builder
        .type_model
        .union_variant_tags
        .get(&ParameterType::named("canvas.Group"))
        .or_else(|| {
            builder
                .type_model
                .union_variant_tags
                .get(&ParameterType::named("Group"))
        })
        .copied()
        .ok_or_else(|| "canvas::groupBlockIsGroup: Group has no union tag".to_string())?;
    let element = item_type(builder)?;
    let block = builder.temporary_vreg();
    let k = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    builder.emit(abi::move_register(&block, &block_in));
    builder.emit(abi::move_register(&k, &k_in));
    let out = builder.label("canvas_group_block_is_out");
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::compare_immediate(&block, "0"));
    builder.emit(abi::branch_eq(&out));
    builder.emit(abi::load_u64(&n, &block, COLLECTION_OFFSET_COUNT));
    // `n <=u k` is `k < 0 || k >= n`.
    builder.emit(abi::compare_registers(&n, &k));
    builder.emit(abi::branch_ls(&out));
    let view =
        super::func_geo_build::emit_list_view(builder, &block, &ParameterType::list_of(element))?;
    let item = builder.temporary_vreg();
    super::func_geo_build::emit_list_element(builder, &item, &view, &k);
    let tag = builder.temporary_vreg();
    builder.emit(abi::load_u64(&tag, &item, 0));
    builder.emit(abi::compare_immediate(&tag, &group_tag.to_string()));
    builder.emit(abi::branch_ne(&out));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"));
    builder.emit(abi::label(&out));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from(RESULT_VALUE_REGISTER),
        text: symbol,
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let param = |name: &'static str, ty: ParameterType| Parameter {
        name,
        desc: "",
        aliases: &[],
        ty,
        default: DefaultValue::None,
    };
    let function = |name: &'static str,
                    params: Vec<Parameter>,
                    return_type: ParameterType,
                    errors: Vec<&'static str>,
                    body: Body| RegistryFunction {
        name,
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params,
            return_type,
            errors,
            body,
        }],
    };
    pkg.add_function(function(
        "groupItemsBlock",
        vec![param("slot", ParameterType::Integer)],
        ParameterType::Integer,
        vec![],
        Body::abi_function(lower_group_items_block),
    ));
    pkg.add_function(function(
        "groupBlockCount",
        vec![param("block", ParameterType::Integer)],
        ParameterType::Integer,
        vec![],
        Body::abi_function(lower_group_block_count),
    ));
    pkg.add_function(function(
        "groupBlockIsGroup",
        vec![
            param("block", ParameterType::Integer),
            param("index", ParameterType::Integer),
        ],
        ParameterType::Boolean,
        vec![],
        Body::abi_function(lower_group_block_is_group),
    ));
    pkg.add_function(function(
        "groupBlockItem",
        vec![
            param("block", ParameterType::Integer),
            param("index", ParameterType::Integer),
            param("none", ParameterType::named("DrawItem")),
        ],
        ParameterType::named("DrawItem"),
        vec!["ErrOutOfMemory"],
        Body::abi_function(lower_group_block_item),
    ));
}
