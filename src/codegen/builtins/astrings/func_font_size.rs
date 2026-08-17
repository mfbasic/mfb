//! `astrings::fontSize` — Integer-valued `Attribute` constructor (`Body::Rewrite`).
//!
//! Backed by the injected source companion (`package.mfb`): a call rewrites to the
//! internal `__astrings_fontSize` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fontSize",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "size",
                desc: "",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Named("Attribute"),
            errors: vec![],
            body: Body::Rewrite("__astrings_fontSize"),
        }],
    });
}
