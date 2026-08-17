//! `astrings::clearAttributes` — Tier-C mutation member (`Body::Rewrite`),
//! overloaded on arity.
//!
//! Backed by the injected source companion (`package.mfb`): the whole form (1 arg)
//! rewrites to `__astrings_clearAttributes`, the ranged form (3 args) to
//! `__astrings_clearAttributesRange`. The registry's overload-aware `rewrite_target`
//! selects the body by argument count (replacing the legacy per-package
//! `implementation_name` arity branch).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

fn value_param() -> Parameter {
    Parameter {
        name: "value",
        desc: "",
        aliases: &[],
        ty: ParameterType::Named("AttributedString"),
        default: DefaultValue::None,
    }
}

fn integer_param(name: &'static str) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    }
}

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "clearAttributes",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![value_param()],
                return_type: ParameterType::Named("AttributedString"),
                errors: vec![],
                body: Body::Rewrite("__astrings_clearAttributes"),
            },
            Implementation {
                params: vec![
                    value_param(),
                    integer_param("start"),
                    integer_param("endIndex"),
                ],
                return_type: ParameterType::Named("AttributedString"),
                errors: vec![],
                body: Body::Rewrite("__astrings_clearAttributesRange"),
            },
        ],
    });
}
