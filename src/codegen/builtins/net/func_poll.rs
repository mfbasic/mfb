//! `net::poll` — descriptor entry (native OS-seam). Return-type-overloaded on
//! argument shape: a scalar `Socket` yields `Boolean` (readiness query), a
//! `List OF RES net.Socket` yields a borrowed `Socket` (readiness multiplex, the
//! `pollList` code form / `os_alias`). Two `Implementation`s, the datetime/net
//! idiom. Docs in `src/docs/man/builtins/net/poll.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "poll",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket, Integer or List OF RES Socket, Integer"),
        internal_only: false,
        implementations: vec![
            // Scalar readiness query: `poll(Socket[, timeoutMs]) AS Boolean`.
            Implementation {
                params: vec![
                    super::req("sock", &[], super::socket()),
                    super::opt("timeoutMs", ParameterType::Integer),
                ],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: super::net_native(&[]),
            },
            // Readiness multiplex: `poll(List OF RES net.Socket[, timeoutMs]) AS
            // Socket` (borrowed). Emits the `net.pollList` code form.
            Implementation {
                params: vec![
                    super::req("socks", &[], ParameterType::list_of(super::socket())),
                    super::opt("timeoutMs", ParameterType::Integer),
                ],
                return_type: ParameterType::Named(super::SOCKET_TYPE_ID),
                errors: vec![],
                body: super::net_native(&["pollList"]),
            },
        ],
    });
}
