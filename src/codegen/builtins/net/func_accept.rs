//! `net::accept` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/accept.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "accept",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Listener, Integer"),
        implementations: vec![Implementation {
            params: vec![
                super::req("listener", &[], super::listener()),
                super::opt("timeoutMs", ParameterType::Integer),
            ],
            return_type: ParameterType::Named(super::SOCKET_TYPE_ID),
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
