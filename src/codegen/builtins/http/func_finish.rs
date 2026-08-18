//! `http::finish` — descriptor entry (source-backed, body `__http_finish`). Docs in
//! `src/docs/man/builtins/http/finish.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "finish",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Stream STATE PendingState"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("stream", &[], ParameterType::Named("Stream"))],
            return_type: ParameterType::Named(super::RESPONSE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_finish"),
        }],
    });
}
