//! `strings.displayWidth` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() == 1 {
        if let Some(value) = builder.static_string_value_vr(&args[0]) {
            let width: i64 = crate::unicode::backend::graphemes(&value)
                .iter()
                .map(|cluster| {
                    cluster
                        .chars()
                        .map(|c| {
                            crate::unicode::runtime_tables::property_for_codepoint(c as u32)
                                .charwidth() as i64
                        })
                        .find(|w| *w != 0)
                        .unwrap_or(0)
                })
                .sum();
            return builder.lower_value(&NirValue::Const {
                type_: ParameterType::Integer,
                value: width.to_string(),
            });
        }
    }
    if args.len() != 1 {
        return Err("strings.displayWidth: no native lowering for these arguments".to_string());
    }
    let value = &args[0];

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
    builder.require_string("strings.displayWidth value", &value)?;
    let value_slot = builder.spill_to_slot("strings_display_width_value", &value.location);

    let empty = builder.label("strings_display_width_empty");
    let walk = builder.label("strings_display_width_loop");
    let is_break = builder.label("strings_display_width_break");
    let no_break = builder.label("strings_display_width_no_break");
    let after = builder.label("strings_display_width_after");
    let skip_set = builder.label("strings_display_width_skip_set");
    let loop_done = builder.label("strings_display_width_loop_done");
    let done = builder.label("strings_display_width_done");

    builder.emit(abi::load_u64(ptr, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(len, ptr, 0));
    builder.emit(abi::compare_immediate(len, "0"));
    builder.emit(abi::branch_eq(&empty));
    builder.emit(abi::add_immediate(data, ptr, 8));
    builder.emit(abi::move_immediate(total, "Integer", "0"));
    // Seed: decode the first scalar, prime the grapheme state, and set the
    // first cluster's width from it (cluster_w starts 0, so this is the
    // first-non-zero-width rule for scalar 0).
    builder.emit_utf8_decode_next(data, cp, adv);
    builder.emit_unicode_property_lookup(cp, prop);
    builder.emit_unicode_property_boundclass(prop, bc_prev);
    builder.emit_unicode_property_indic_conjunct_break(prop, icb_prev);
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
    // A boundary ends the current cluster: flush its width and start fresh so
    // the current scalar seeds the new cluster below.
    builder.emit(abi::label(&is_break));
    builder.emit(abi::add_registers(total, total, cluster_w));
    builder.emit(abi::move_immediate(cluster_w, "Integer", "0"));
    builder.emit(abi::branch(&after));
    builder.emit(abi::label(&no_break));
    builder.emit(abi::label(&after));
    // First-non-zero-width rule: if this cluster has no width yet, take this
    // scalar's. cw==0 for a combining mark, so a zero-width scalar never
    // overrides a base already seen.
    builder.emit(abi::compare_immediate(cluster_w, "0"));
    builder.emit(abi::branch_ne(&skip_set));
    builder.emit(abi::move_register(cluster_w, cw));
    builder.emit(abi::label(&skip_set));
    builder.emit_grapheme_state_update(bc_prev, icb_prev, bc_cur, icb_cur);
    builder.emit(abi::add_registers(pos, pos, adv));
    builder.emit(abi::branch(&walk));
    builder.emit(abi::label(&loop_done));
    builder.emit(abi::add_registers(total, total, cluster_w));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(total, "Integer", "0"));
    builder.emit(abi::label(&done));

    let result = builder.allocate_register()?;
    builder.emit(abi::move_register(&result, total));
    Ok(ValueResult {
        origin: None,
        type_: "Integer".to_string(),
        location: Operand::from(result.render()),
        text: "strings.displayWidth".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "displayWidth",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
