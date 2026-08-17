//! `strings::isDigit` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected source companion (`seam.mfb`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_isDigit` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isDigit",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "scalar",
                desc: "",
                aliases: &[],
                ty: ParameterType::Named("Scalar"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::Rewrite("__strings_isDigit"),
        }],
    });
}
