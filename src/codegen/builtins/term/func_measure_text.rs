//! `term::measureText` and its internal ungated measurement primitives.

use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::raise_error_into;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Measure text in terminal grid cells."#;

const DESC: &str = r#"`term::measureText` returns the number of terminal grid cells that
`term::drawText` advances for `value`. The empty string measures `0`; ordinary
characters measure one cell; East Asian wide and fullwidth characters generally
measure two; combining sequences and joined emoji are each one grapheme cluster.
A cluster made entirely from zero-width scalars still measures one cell, matching
the terminal drawing surface.

The call requires an active Console TUI. It raises `ErrUnsupported` before
`term::on()` and after `term::off()`. In an app build it raises `ErrWrongMode`
unless the current mode is `app::Mode.Console`."#;

const EX: &str = r#"Measure text while the terminal surface is active:

```
IMPORT io
IMPORT term

FUNC main() AS Nothing
  term::on()
  LET columns AS Integer = term::measureText("日本語")
  term::off()
  io::print(toString(columns))
END FUNC
```"#;

/// Emit the terminal grid-cell width walk. A cluster contributes the first
/// non-zero scalar width, or one when every scalar in it has raw width zero.
pub(crate) fn measure_text_of(
    builder: &mut CodeBuilder,
    value: &ValueResult,
) -> Result<ValueResult, String> {
    let ptr = builder.temporary_vreg();
    let len = builder.temporary_vreg();
    let data = builder.temporary_vreg();
    let pos = builder.temporary_vreg();
    let total = builder.temporary_vreg();
    let cluster_w = builder.temporary_vreg();
    let cp = builder.temporary_vreg();
    let adv = builder.temporary_vreg();
    let prop = builder.temporary_vreg();
    let bc_prev = builder.temporary_vreg();
    let icb_prev = builder.temporary_vreg();
    let bc_cur = builder.temporary_vreg();
    let icb_cur = builder.temporary_vreg();
    let cw = builder.temporary_vreg();
    let addr = builder.temporary_vreg();
    let (ptr, len, data, pos, total, cluster_w) = (&ptr, &len, &data, &pos, &total, &cluster_w);
    let (cp, adv, prop, bc_prev, icb_prev, bc_cur, icb_cur, cw, addr) = (
        &cp, &adv, &prop, &bc_prev, &icb_prev, &bc_cur, &icb_cur, &cw, &addr,
    );
    let value = value.clone();
    builder.require_string("term.measureText value", &value)?;
    let value_slot = builder.spill_to_slot("term_measure_text_value", &value.location);

    let empty = builder.label("term_measure_text_empty");
    let walk = builder.label("term_measure_text_loop");
    let is_break = builder.label("term_measure_text_break");
    let no_break = builder.label("term_measure_text_no_break");
    let after = builder.label("term_measure_text_after");
    let flush = builder.label("term_measure_text_flush");
    let final_flush = builder.label("term_measure_text_final_flush");
    let skip_set = builder.label("term_measure_text_skip_set");
    let loop_done = builder.label("term_measure_text_loop_done");
    let done = builder.label("term_measure_text_done");

    builder.emit(abi::load_u64(ptr, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(len, ptr, 0));
    builder.emit(abi::compare_immediate(len, "0"));
    builder.emit(abi::branch_eq(&empty));
    builder.emit(abi::add_immediate(data, ptr, 8));
    builder.emit(abi::move_immediate(total, "Integer", "0"));
    builder.emit_utf8_decode_next(data, cp, adv);
    builder.emit_unicode_property_lookup(cp, prop);
    builder.emit_unicode_property_boundclass(prop, bc_prev);
    builder.emit_unicode_property_indic_conjunct_break(prop, icb_prev);
    builder.emit_seed_indic_conjunct_state(icb_prev);
    builder.emit_unicode_property_charwidth(prop, cw);
    builder.emit(abi::move_register(cluster_w, cw));
    builder.emit(abi::move_register(pos, adv));
    builder.emit(abi::label(&walk));
    builder.emit(abi::compare_registers(pos, len));
    builder.emit(abi::branch_ge(&loop_done));
    builder.emit(abi::add_registers(addr, data, pos));
    builder.emit_utf8_decode_next(addr, cp, adv);
    builder.emit_unicode_property_lookup(cp, prop);
    builder.emit_unicode_property_boundclass(prop, bc_cur);
    builder.emit_unicode_property_indic_conjunct_break(prop, icb_cur);
    builder.emit_unicode_property_charwidth(prop, cw);
    builder.emit_grapheme_break_branch(bc_prev, icb_prev, bc_cur, icb_cur, &is_break, &no_break);
    builder.emit(abi::label(&is_break));
    builder.emit(abi::compare_immediate(cluster_w, "0"));
    builder.emit(abi::branch_ne(&flush));
    builder.emit(abi::move_immediate(cluster_w, "Integer", "1"));
    builder.emit(abi::label(&flush));
    builder.emit(abi::add_registers(total, total, cluster_w));
    builder.emit(abi::move_immediate(cluster_w, "Integer", "0"));
    builder.emit(abi::branch(&after));
    builder.emit(abi::label(&no_break));
    builder.emit(abi::label(&after));
    builder.emit(abi::compare_immediate(cluster_w, "0"));
    builder.emit(abi::branch_ne(&skip_set));
    builder.emit(abi::move_register(cluster_w, cw));
    builder.emit(abi::label(&skip_set));
    builder.emit_grapheme_state_update(bc_prev, icb_prev, bc_cur, icb_cur);
    builder.emit(abi::add_registers(pos, pos, adv));
    builder.emit(abi::branch(&walk));
    builder.emit(abi::label(&loop_done));
    builder.emit(abi::compare_immediate(cluster_w, "0"));
    builder.emit(abi::branch_ne(&final_flush));
    builder.emit(abi::move_immediate(cluster_w, "Integer", "1"));
    builder.emit(abi::label(&final_flush));
    builder.emit(abi::add_registers(total, total, cluster_w));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(total, "Integer", "0"));
    builder.emit(abi::label(&done));

    let result = builder.allocate_register();
    builder.emit(abi::move_register(&result, total));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(result),
        text: "term.measureTextRaw".to_string(),
    })
}

/// Return the raw Unicode width of the first scalar. Generated padding code calls
/// this only after proving the String contains exactly one scalar.
pub(crate) fn scalar_width_of(
    builder: &mut CodeBuilder,
    value: &ValueResult,
) -> Result<ValueResult, String> {
    let value = value.clone();
    builder.require_string("term.scalarWidthRaw value", &value)?;
    let ptr = builder.temporary_vreg();
    let data = builder.temporary_vreg();
    let cp = builder.temporary_vreg();
    let adv = builder.temporary_vreg();
    let prop = builder.temporary_vreg();
    let width = builder.allocate_register();
    builder.emit(abi::move_register(&ptr, &value.location));
    builder.emit(abi::add_immediate(&data, &ptr, 8));
    builder.emit_utf8_decode_next(&data, &cp, &adv);
    builder.emit_unicode_property_lookup(&cp, &prop);
    builder.emit_unicode_property_charwidth(&prop, &width);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(width),
        text: "term.scalarWidthRaw".to_string(),
    })
}

fn string_arg(args: &[ValueResult], call: &str) -> Result<ValueResult, String> {
    let Some(arg) = args.first() else {
        return Err(format!("{call}: missing String argument"));
    };
    let mut arg = arg.clone();
    // AbiFunction's generic wrapper carries physical arguments as Integer-shaped
    // values; the registry signature is the authority for this String slot.
    arg.type_ = ParameterType::String;
    Ok(arg)
}

fn inline_arg(args: &[ValueResult], call: &str) -> Result<ValueResult, String> {
    args.first()
        .cloned()
        .ok_or_else(|| format!("{call}: missing String argument"))
}

pub(crate) fn lower_measure_text(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let term_state_offset = ctx
        .term_state_offset
        .ok_or_else(|| format!("native code plan emits '{symbol}' without reserving term state"))?;
    let active = builder.label("term_measure_text_active");
    let active_flag = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &active_flag,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    builder.emit(abi::compare_immediate(&active_flag, "0"));
    builder.emit(abi::branch_ne(&active));
    raise_error_into(
        &symbol,
        "ErrUnsupported",
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::return_());
    builder.emit(abi::label(&active));
    let result = measure_text_of(builder, &string_arg(args, "term.measureText")?)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    prepend_wrong_mode_gate(
        &mut builder.instructions,
        &mut builder.relocations,
        &symbol,
        ctx.presentation_mode_offset,
        ModeRequirement::Console,
    );
    Ok(super::gen_shared::void_result("term.measureText"))
}

fn lower_measure_text_raw(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    measure_text_of(builder, &inline_arg(args, "term.measureTextRaw")?)
}

fn lower_scalar_width_raw(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    scalar_width_of(builder, &inline_arg(args, "term.scalarWidthRaw")?)
}

fn string_param(desc: &'static str) -> Parameter {
    Parameter {
        name: "value",
        desc,
        aliases: &[],
        ty: ParameterType::String,
        default: DefaultValue::None,
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "measureText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![string_param(
                "The text whose terminal grid-cell width is measured.",
            )],
            return_type: ParameterType::Integer,
            errors: vec!["ErrUnsupported", "ErrWrongMode"],
            body: Body::abi_function(lower_measure_text),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "measureTextRaw",
        intro: "Internal ungated terminal-cell measurement.",
        desc: "Compiler-generated code only.",
        example: "",
        expected_arguments: Some("String"),
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![string_param(
                "Text to measure without terminal lifecycle gates.",
            )],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower_measure_text_raw),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "scalarWidthRaw",
        intro: "Internal raw scalar-width query.",
        desc: "Compiler-generated code only.",
        example: "",
        expected_arguments: Some("String"),
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![string_param(
                "One Unicode scalar to measure without clamping.",
            )],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower_scalar_width_raw),
        }],
    });
}
