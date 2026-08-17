//! `http::pump` — descriptor entry (source-backed, body `__http_pump`). Docs in
//! `src/docs/man/builtins/http/pump.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pump",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Stream STATE PendingState"),
        implementations: vec![Implementation {
            params: vec![super::req("stream", &[], ParameterType::Named("Stream"))],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::Rewrite("__http_pump"),
        }],
    });
}
