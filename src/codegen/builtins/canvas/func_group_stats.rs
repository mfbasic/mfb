//! `canvas::groupCount` / `canvas::groupBytes` — the group table's two stats readings.
//!
//! Internal-only. `__canvas_presentSurface` writes them into the `MFB_CANVAS_STATS`
//! line, which `.ai/canvas-threading.md` §11 records as the only window a test has onto
//! worker-owned state — and the group table is worker-owned state living in a
//! process-global block no MFBASIC expression can reach.

use super::gen_group::{emit_group_bytes, emit_group_count};
use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "groupCount",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(emit_group_count),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "groupBytes",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(emit_group_bytes),
        }],
    });
}
