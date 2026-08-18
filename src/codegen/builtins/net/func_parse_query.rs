//! `net::parseQuery` — descriptor entry (source-backed, body `__net_parseQuery`
//! in `package.mfb`). Docs in `src/docs/man/builtins/net/parseQuery.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "parseQuery",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("s", &["query", "value"], ParameterType::String)],
            return_type: ParameterType::map_of(ParameterType::String, ParameterType::String),
            errors: vec![],
            body: Body::Rewrite("__net_parseQuery"),
        }],
    });
}
