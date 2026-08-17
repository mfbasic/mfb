//! `http::route` — descriptor entry (source-backed, body `__http_route`). Docs in
//! `src/docs/man/builtins/http/route.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "route",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String, FUNC(Request) AS Response"),
        implementations: vec![Implementation {
            params: vec![
                super::req("pattern", &[], ParameterType::String),
                super::req("handler", &[], ParameterType::Named(super::HANDLER_TYPE)),
            ],
            return_type: ParameterType::Named(super::ROUTE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_route"),
        }],
    });
}
