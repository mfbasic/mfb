//! `regex::scriptOf` — internal-only native Script-property lookup (`Body::abi_inline`).
//!
//! Never user-callable (`internal_only`): the regex companion resolves
//! `\p{Script=…}` through it. It replaces the generated `__regex_scriptOf`, a
//! 1,708-arm IF-chain that cost **440,905 machine instructions** and up to 1,708
//! compares per query; the lookup is an 11-step binary search over a rodata run
//! table (plan-118-B). `__regex_scriptCanonName`, the other function in the same
//! generated file, is 171 arms over script *names* rather than scalars and
//! stays as MFBASIC.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::{UNICODE_SCRIPT_NAMES_SYMBOL, UNICODE_SCRIPT_RANGES_SYMBOL};
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
        "regex.scriptOf",
        args,
        crate::unicode::range_tables::script(),
        UNICODE_SCRIPT_RANGES_SYMBOL,
        UNICODE_SCRIPT_NAMES_SYMBOL,
    )
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "scriptOf",
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
