//! `net::toUrl` — descriptor entry (source-backed). Its body `__net_toUrl` lives
//! in `package.mfb`; `Body::Rewrite` repoints the call at it (the legacy
//! `Implementation::Rewrite`). Docs in `src/docs/man/builtins/net/toUrl.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toUrl",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("href", &["value", "url"], ParameterType::String)],
            return_type: ParameterType::Named(super::URL_TYPE),
            errors: vec![],
            body: Body::Rewrite("__net_toUrl"),
        }],
    });
}
