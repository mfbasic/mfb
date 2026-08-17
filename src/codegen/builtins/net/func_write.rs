//! `net::write` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/write.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket, List OF Byte"),
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", &[], super::socket()),
                super::req("bytes", &[], ParameterType::list_of(ParameterType::Byte)),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
