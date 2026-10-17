//! `strings::lineBreakClass` — internal-only native UAX #14 Line_Break lookup
//! (`Body::abi_inline`, plan-162-E).
//!
//! Never user-callable (`internal_only`): `strings::lineBreaks`' MFBASIC body asks it for
//! each scalar's class. It answers an `Integer` — the class after UAX #14 LB1's default
//! resolutions, numbered as `tools/unicode-tables/gen_linebreak_table.py`'s `CLASSES`
//! list, plus 64 for East_Asian_Width F/W/H, 128 for Extended_Pictographic ∧ Cn, and
//! 256/512 for a Pi/Pf quotation mark — read straight out of the run it bisects to in
//! the generated `unicode_linebreak_ranges.txt`. The same rodata bisection as
//! `strings::genCat` (plan-118-B), minus the name table: the run carries the value.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::UNICODE_LINEBREAK_RANGES_SYMBOL;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 1 {
        return Err("strings.lineBreakClass: no native lowering for these arguments".into());
    }
    let result = builder.allocate_register();
    builder.emit_unicode_range_value(
        args[0].location.clone(),
        result,
        UNICODE_LINEBREAK_RANGES_SYMBOL,
        crate::unicode::range_tables::linebreak().runs.len(),
    );
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(result),
        text: "strings.lineBreakClass".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "lineBreakClass",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "codepoint",
                desc: "",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
