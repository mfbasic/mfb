//! `net::setWriteTimeout` — descriptor entry (native OS-seam). Overloaded over
//! `Socket` / `UdpSocket`. Docs in
//! `src/docs/man/builtins/net/setWriteTimeout.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![
            super::req("sock", &[], ty),
            super::req("timeoutMs", &[], ParameterType::Integer),
        ],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: super::net_native(&[]),
    }
}

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setWriteTimeout",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket or UdpSocket, Integer"),
        internal_only: false,
        implementations: vec![overload(super::socket()), overload(super::udp())],
    });
}
