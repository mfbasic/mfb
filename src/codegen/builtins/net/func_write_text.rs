//! `net::writeText` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/writeText.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeText",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", &[], super::socket()),
                super::req("value", &[], ParameterType::String),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
