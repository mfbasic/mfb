//! `astrings::getAttributes` — Tier-C query member (`Body::Rewrite`).
//!
//! Backed by the injected source companion (`package.mfb`): a call rewrites to the
//! internal `__astrings_getAttributes` FUNC through the registry's `rewrite_target`.
//! Returns the winning attributes covering the scalar at `index`
//! (higher-start-wins resolution).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getAttributes",
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
                    ty: ParameterType::named("AttributedString"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "index",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::named("Attribute")),
            errors: vec![],
            body: Body::Rewrite("__astrings_getAttributes"),
        }],
    });
}
