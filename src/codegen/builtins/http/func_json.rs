//! `http::json` — descriptor entry (source-backed, body `__http_json`). Docs in
//! `src/docs/man/builtins/http/json.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "json",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String"),
        implementations: vec![Implementation {
            params: vec![super::req("body", &[], ParameterType::String)],
            return_type: ParameterType::Named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_json"),
        }],
    });
}
