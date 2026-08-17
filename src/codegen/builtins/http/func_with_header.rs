//! `http::withHeader` — descriptor entry (source-backed, body `__http_withHeader`).
//! Docs in `src/docs/man/builtins/http/withHeader.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "withHeader",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Response, String, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req(
                    "resp",
                    &["response"],
                    ParameterType::Named(super::RESPONSE_TYPE),
                ),
                super::req("name", &[], ParameterType::String),
                super::req("value", &[], ParameterType::String),
            ],
            return_type: ParameterType::Named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_withHeader"),
        }],
    });
}
