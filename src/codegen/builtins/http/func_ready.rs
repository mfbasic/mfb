//! `http::ready` — descriptor entry (source-backed, body `__http_ready`). Docs in
//! `src/docs/man/builtins/http/ready.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "ready",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Stream STATE PendingState"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("stream", &[], ParameterType::Named("Stream"))],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::Rewrite("__http_ready"),
        }],
    });
}
