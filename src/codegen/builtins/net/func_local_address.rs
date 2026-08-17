//! `net::localAddress` — descriptor entry (native OS-seam). Overloaded over the
//! `Socket` / `Listener` / `UdpSocket` union, all returning `Address`. Docs in
//! `src/docs/man/builtins/net/localAddress.md`.

use crate::codegen::registry::{Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "sock",
            desc: "",
            aliases: &["listener"],
            ty,
            default: crate::codegen::registry::DefaultValue::None,
        }],
        return_type: ParameterType::Named(super::ADDRESS_TYPE),
        errors: vec![],
        body: super::net_native(&[]),
    }
}

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "localAddress",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket or Listener or UdpSocket"),
        internal_only: false,
        implementations: vec![
            overload(super::socket()),
            overload(super::listener()),
            overload(super::udp()),
        ],
    });
}
