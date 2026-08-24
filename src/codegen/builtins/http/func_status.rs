//! `http::status` — descriptor entry (source-backed, body `__http_status`). Docs in
//! `src/docs/man/builtins/http/status.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "status",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Integer, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("code", &[], ParameterType::Integer),
                super::req("body", &[], ParameterType::String),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_status"),
        }],
    });
}
