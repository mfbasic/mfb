//! `strings::toScalars` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected source companion (`seam.mfb`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_toScalars` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toScalars",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::named("Scalar")),
            errors: vec![],
            body: Body::Rewrite("__strings_toScalars"),
        }],
    });
}
