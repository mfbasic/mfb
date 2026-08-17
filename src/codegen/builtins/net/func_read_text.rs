//! `net::readText` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/readText.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readText",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", &[], super::socket()),
                super::req("maxBytes", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
