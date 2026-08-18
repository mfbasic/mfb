//! `http::read` — descriptor entry (source-backed, body `__http_read` in
//! `package.mfb`). Docs in `src/docs/man/builtins/http/read.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "read",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Url, Map OF String TO String, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("url", &[], ParameterType::Named("Url")),
                super::fill("headers", super::header_map(), "{}"),
                super::fill("method", ParameterType::String, "GET"),
            ],
            return_type: ParameterType::Named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_read"),
        }],
    });
}
