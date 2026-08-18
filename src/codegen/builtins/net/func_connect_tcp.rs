//! `net::connectTcp` — descriptor entry (native OS-seam). Four argument-shape
//! overloads (host/port, host/port/timeout, address, address/timeout); the two
//! `Address` forms lower to the `net.connectTcpAddr` code form (an `os_alias`), the
//! others to `net.connectTcp`. The overload split + timeout padding lives in
//! `builder_values`. Docs in `src/docs/man/builtins/net/connectTcp.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let ret = || ParameterType::Named(super::SOCKET_TYPE_ID);
    pkg.add_function(RegistryFunction {
        name: "connectTcp",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String, Integer, Integer or Address, Integer"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    super::req("host", &[], ParameterType::String),
                    super::req("port", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::net_native(&[]),
            },
            Implementation {
                params: vec![
                    super::req("host", &[], ParameterType::String),
                    super::req("port", &[], ParameterType::Integer),
                    super::req("timeoutMs", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::net_native(&[]),
            },
            Implementation {
                params: vec![super::req(
                    "address",
                    &[],
                    ParameterType::Named(super::ADDRESS_TYPE),
                )],
                return_type: ret(),
                errors: vec![],
                body: super::net_native(&["connectTcpAddr"]),
            },
            Implementation {
                params: vec![
                    super::req("address", &[], ParameterType::Named(super::ADDRESS_TYPE)),
                    super::req("timeoutMs", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::net_native(&[]),
            },
        ],
    });
}
