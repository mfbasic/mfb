//! `http::responseDefault` — descriptor entry (source-backed, body
//! `__http_responseDefault`). Docs in
//! `src/docs/man/builtins/http/responseDefault.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "responseDefault",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("no arguments"),
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_responseDefault"),
        }],
    });
}
