//! `strings::genCat` — internal-only native general-category lookup (`Body::abi_inline`).
//!
//! Never user-callable (`internal_only`): the scalar-seam companion's five
//! classification predicates (`isLetter`/`isDigit`/`isWhitespace`/`isUpper`/
//! `isLower`) resolve through it past their `cp < 128` ASCII fast paths. It
//! replaces `__strings_genCat` — the same generated 4,099-arm IF-chain `regex`
//! embedded under its own name, compiled a SECOND time at another **1,057,783
//! machine instructions** (plan-118-B phase 1 proved the two instruction
//! streams identical). Both packages now route to `lower_unicode_range_member`
//! over one shared rodata table, so the duplication is gone with the IF-chain.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::{UNICODE_GENCAT_NAMES_SYMBOL, UNICODE_GENCAT_RANGES_SYMBOL};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_unicode_range_member(
        "strings.genCat",
        args,
        crate::unicode::range_tables::gencat(),
        UNICODE_GENCAT_RANGES_SYMBOL,
        UNICODE_GENCAT_NAMES_SYMBOL,
    )
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "genCat",
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
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
