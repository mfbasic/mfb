//! `http::ok` — descriptor entry (source-backed, body `__http_ok`). Docs in
//! `src/docs/man/builtins/http/ok.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "ok",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("body", &[], ParameterType::String)],
            return_type: ParameterType::named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_ok"),
        }],
    });
}
