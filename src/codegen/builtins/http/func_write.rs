//! `http::write` — descriptor entry (source-backed, body `__http_write`). Docs in
//! `src/docs/man/builtins/http/write.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Url, String, Map OF String TO String, String"),
        implementations: vec![Implementation {
            params: vec![
                super::req("url", &[], ParameterType::Named("Url")),
                super::req("body", &[], ParameterType::String),
                super::fill("headers", super::header_map(), "{}"),
                super::fill("method", ParameterType::String, "POST"),
            ],
            return_type: ParameterType::Named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_write"),
        }],
    });
}
