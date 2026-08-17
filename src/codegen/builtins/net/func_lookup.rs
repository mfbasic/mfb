//! `net::lookup` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/lookup.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "lookup",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String, Integer"),
        implementations: vec![Implementation {
            params: vec![
                super::req("host", &[], ParameterType::String),
                super::opt("port", ParameterType::Integer),
            ],
            return_type: ParameterType::list_of(ParameterType::Named(super::ADDRESS_TYPE)),
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
