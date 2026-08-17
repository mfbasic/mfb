//! `net::remoteAddress` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/remoteAddress.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "remoteAddress",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("sock", &[], super::socket())],
            return_type: ParameterType::Named(super::ADDRESS_TYPE),
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
