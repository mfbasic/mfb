//! `http::respondFile` — descriptor entry (source-backed, body
//! `__http_respondFile`). Consumes the `RES fs::File` it serves (see
//! `syntaxcheck::builtins::http_consumes_argument`). Docs in
//! `src/docs/man/builtins/http/respondFile.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "respondFile",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("File[, String]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("file", &[], ParameterType::Named(super::FILE_TYPE)),
                super::fill("contentType", ParameterType::String, ""),
            ],
            return_type: ParameterType::Named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_respondFile"),
        }],
    });
}
