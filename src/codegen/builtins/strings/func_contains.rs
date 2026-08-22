//! `strings.contains` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.contains: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let needle = &args[1];

    let scratch16 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let value = builder.lower_value(value)?;
    builder.require_string("strings.contains value", &value)?;
    let value_slot = builder.spill_to_slot("strings_contains_value", &value.location);
    let needle = builder.lower_value(needle)?;
    builder.require_string("strings.contains needle", &needle)?;
    let needle_slot = builder.spill_to_slot("strings_contains_needle", &needle.location);

    let result_slot = builder.allocate_stack_object("strings_contains_result", 8);
    let true_label = builder.label("strings_contains_true");
    let false_label = builder.label("strings_contains_false");
    let done_label = builder.label("strings_contains_done");
    let loop_label = builder.label("strings_contains_loop");
    let no_match_label = builder.label("strings_contains_no_match");

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), needle_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::load_u64(&scratch10, &scratch17, 0));
    builder.emit(abi::compare_immediate(&scratch10, "0"));
    builder.emit(abi::branch_eq(&true_label));
    builder.emit(abi::compare_registers(&scratch10, &scratch9));
    builder.emit(abi::branch_hi(&false_label));
    builder.emit(abi::add_immediate(&scratch11, &scratch16, 8));
    builder.emit(abi::add_immediate(&scratch12, &scratch17, 8));
    builder.emit(abi::subtract_registers(&scratch13, &scratch9, &scratch10));
    builder.emit(abi::move_immediate(&scratch14, "Integer", "0"));
    builder.emit(abi::label(&loop_label));
    builder.emit(abi::compare_registers(&scratch14, &scratch13));
    builder.emit(abi::branch_hi(&false_label));
    builder.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
    builder.emit_string_byte_range_equal_branch(
        &scratch15,
        &scratch12,
        &scratch10,
        &true_label,
        &no_match_label,
    );
    builder.emit(abi::label(&no_match_label));
    builder.emit(abi::add_immediate(&scratch14, &scratch14, 1));
    builder.emit(abi::branch(&loop_label));
    builder.emit_string_predicate_result(result_slot, &true_label, &false_label, &done_label);
    builder.finish_string_predicate_result("strings.contains", result_slot)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "contains",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "needle",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
