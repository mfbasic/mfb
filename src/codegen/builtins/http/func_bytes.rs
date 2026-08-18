//! `http::bytes` — descriptor entry (source-backed, body `__http_bytes`). Docs in
//! `src/docs/man/builtins/http/bytes.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bytes",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("text", &[], ParameterType::String)],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::Rewrite("__http_bytes"),
        }],
    });
}
