//! `net::percentDecode` — descriptor entry (source-backed, body
//! `__net_percentDecode` in `package.mfb`). Docs in
//! `src/docs/man/builtins/net/percentDecode.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "percentDecode",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("s", &["text", "value"], ParameterType::String)],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::Rewrite("__net_percentDecode"),
        }],
    });
}
