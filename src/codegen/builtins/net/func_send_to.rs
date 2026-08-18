//! `net::sendTo` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/sendTo.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sendTo",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("UdpSocket, Address, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", &[], super::udp()),
                super::req("address", &[], ParameterType::Named(super::ADDRESS_TYPE)),
                super::req("bytes", &[], ParameterType::list_of(ParameterType::Byte)),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::net_native(&[]),
        }],
    });
}
