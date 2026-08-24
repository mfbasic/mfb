//! `http::server` — descriptor entry (source-backed, body `__http_server`). Docs in
//! `src/docs/man/builtins/http/server.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "server",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Integer[, String[, Integer]]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("port", &[], ParameterType::Integer),
                super::fill("host", ParameterType::String, "0.0.0.0"),
                super::fill("backlog", ParameterType::Integer, "128"),
            ],
            return_type: ParameterType::named(super::LISTENER_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_server"),
        }],
    });
}
