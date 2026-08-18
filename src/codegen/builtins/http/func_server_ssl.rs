//! `http::serverSSL` — descriptor entry (source-backed, body `__http_serverSSL`).
//! Docs in `src/docs/man/builtins/http/serverSSL.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "serverSSL",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Integer, String, String[, String[, Integer]]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("port", &[], ParameterType::Integer),
                super::req("certPath", &[], ParameterType::String),
                super::req("keyPath", &[], ParameterType::String),
                super::fill("host", ParameterType::String, "0.0.0.0"),
                super::fill("backlog", ParameterType::Integer, "128"),
            ],
            return_type: ParameterType::Named(super::TLS_LISTENER_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_serverSSL"),
        }],
    });
}
