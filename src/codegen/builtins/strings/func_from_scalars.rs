//! `strings::fromScalars` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected source companion (`seam.mfb`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_fromScalars` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromScalars",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "scalars",
                desc: "",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::Named("Scalar")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::Rewrite("__strings_fromScalars"),
        }],
    });
}
