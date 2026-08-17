//! `net::close` — descriptor entry (native OS-seam). Spans the resource union
//! (`Socket` / `Listener` / `UdpSocket`) as three overloads, all returning
//! `Nothing` and all lowering to `net.close` (the datetime/tls idiom, no custom
//! resolver). `close` consumes the handle it is given (see
//! `syntaxcheck::builtins::net_consumes_argument`). Docs in
//! `src/docs/man/builtins/net/close.md`.

use crate::codegen::registry::{Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "resource",
            desc: "",
            aliases: &["sock", "listener"],
            ty,
            default: crate::codegen::registry::DefaultValue::None,
        }],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: super::net_native(&[]),
    }
}

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "close",
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
