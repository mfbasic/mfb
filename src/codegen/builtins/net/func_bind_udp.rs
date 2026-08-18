//! `net::bindUdp` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/bindUdp.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bindUdp",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("host", &[], ParameterType::String),
                super::req("port", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::Named(super::UDP_SOCKET_TYPE_ID),
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
