//! `http::respondPath` — descriptor entry (source-backed, body
//! `__http_respondPath`). Docs in `src/docs/man/builtins/http/respondPath.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "respondPath",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Request, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req(
                    "req",
                    &["request"],
                    ParameterType::named(super::REQUEST_TYPE),
                ),
                super::req("root", &[], ParameterType::String),
            ],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_respondPath"),
        }],
    });
}
