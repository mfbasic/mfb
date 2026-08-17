//! `astrings::overline` — `Attribute`-model flag constructor (`Body::Rewrite`).
//!
//! Backed by the injected source companion (`package.mfb`): a call rewrites to the
//! internal `__astrings_overline` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "overline",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Named("Attribute"),
            errors: vec![],
            body: Body::Rewrite("__astrings_overline"),
        }],
    });
}
