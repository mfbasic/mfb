//! `net::receiveTextFrom` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/receiveTextFrom.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "receiveTextFrom",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("UdpSocket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", &[], super::udp()),
                super::req("maxBytes", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::Named(super::DATAGRAM_TEXT_TYPE),
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
