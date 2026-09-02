//! `regex::genCat` — internal-only native general-category lookup (`Body::abi_inline`).
//!
//! Never user-callable (`internal_only`): the regex companion — an `internal`
//! file — resolves `\d`, `\w`, `\s`, `\b` and `\p{gc}` through it. It replaces
//! the generated `__regex_genCat`, a 4,099-arm `IF cp <= N THEN RETURN "Lu"`
//! chain that cost **1,057,783 machine instructions** (6.2% of the acceptance
//! module) and up to 4,099 compares per query; the lookup is a 12-step binary
//! search over a rodata run table (plan-118-B).
//!
//! `strings` registers its own copy over the same table for the same reason
//! `__strings_genCat` existed: a package's injected companion may call only its
//! OWN package's members, and making one import the other would drag a whole
//! package into every program using the other. Both route to
//! `lower_unicode_range_member`.

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
        "regex.genCat",
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
