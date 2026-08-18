//! `http::done` — descriptor entry (source-backed, body `__http_done`). Docs in
//! `src/docs/man/builtins/http/done.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "done",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Stream STATE PendingState"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("stream", &[], ParameterType::Named("Stream"))],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::Rewrite("__http_done"),
        }],
    });
}
