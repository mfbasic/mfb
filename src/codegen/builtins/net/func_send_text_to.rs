//! `net::sendTextTo` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/sendTextTo.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sendTextTo",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("UdpSocket, Address, String"),
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", &[], super::udp()),
                super::req("address", &[], ParameterType::Named(super::ADDRESS_TYPE)),
                super::req("value", &[], ParameterType::String),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
