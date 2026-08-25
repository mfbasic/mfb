//! `astrings::addAttribute` — Tier-C mutation member (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_addAttribute` FUNC through the registry's `rewrite_target`.
//! The end-of-range parameter is `endIndex` (not `end`, a reserved keyword).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

fn ranged_attr_params() -> Vec<Parameter> {
    vec![
        Parameter {
            name: "value",
            desc: "",
            aliases: &[],
            ty: ParameterType::named("AttributedString"),
            default: DefaultValue::None,
        },
        Parameter {
            name: "start",
            desc: "",
            aliases: &[],
            ty: ParameterType::Integer,
            default: DefaultValue::None,
        },
        Parameter {
            name: "endIndex",
            desc: "",
            aliases: &[],
            ty: ParameterType::Integer,
            default: DefaultValue::None,
        },
        Parameter {
            name: "attr",
            desc: "",
            aliases: &[],
            ty: ParameterType::named("Attribute"),
            default: DefaultValue::None,
        },
    ]
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "addAttribute",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: ranged_attr_params(),
            return_type: ParameterType::named("AttributedString"),
            errors: vec![],
            body: Body::Rewrite("__astrings_addAttribute"),
        }],
    });
}
