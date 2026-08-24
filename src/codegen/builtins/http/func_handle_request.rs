//! `http::handleRequest` — descriptor entry (source-backed). Overloaded by listener
//! type: a `net::Listener` rewrites to `__http_handleRequest`, a `tls::TlsListener`
//! to `__http_handleRequestSSL` — two `Implementation`s the generic overload
//! resolution selects by the first argument's type (the datetime/net idiom, no
//! custom resolver). Docs in `src/docs/man/builtins/http/handleRequest.md`.

use crate::codegen::registry::{
    Body, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

fn overload(listener_ty: &'static str, rewrite: &'static str) -> Implementation {
    Implementation {
        params: vec![
            Parameter {
                name: "listener",
                desc: "",
                aliases: &["server"],
                ty: ParameterType::named(listener_ty),
                default: crate::codegen::registry::DefaultValue::None,
            },
            super::req(
                "routes",
                &[],
                ParameterType::list_of(ParameterType::named(super::ROUTE_TYPE)),
            ),
        ],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: Body::Rewrite(rewrite),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "handleRequest",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Listener or TlsListener, List OF Route"),
        internal_only: false,
        implementations: vec![
            overload(super::LISTENER_TYPE, "__http_handleRequest"),
            overload(super::TLS_LISTENER_TYPE, "__http_handleRequestSSL"),
        ],
    });
}
