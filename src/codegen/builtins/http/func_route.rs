//! `http::route` — descriptor entry (source-backed, body `__http_route`). Docs in
//! `src/docs/man/builtins/http/route.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "route",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String, FUNC(Request) AS Response"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("pattern", &[], ParameterType::String),
                // The handler is the STRUCTURED function type `FUNC(Request) AS
                // Response` (parsed, not a `Named` blob): the matcher compares it
                // element-wise, so a wrong-shaped handler (`FUNC(Integer) AS Integer`)
                // is rejected — a `Named("FUNC(…)")` blob would match coarsely and let
                // it through.
                super::req("handler", &[], ParameterType::parse(super::HANDLER_TYPE)),
            ],
            return_type: ParameterType::named(super::ROUTE_TYPE),
            errors: vec![],
            body: Body::Rewrite("__http_route"),
        }],
    });
}
