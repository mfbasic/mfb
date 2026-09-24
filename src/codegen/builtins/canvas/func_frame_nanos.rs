//! `canvas::frameNanos` — the monotonic clock, for the renderer's `--debug` phase timers
//! (bug-686 Phase 0).
//!
//! Internal-only, and deliberately not a `datetime` import: `datetime` carries MFBASIC
//! helpers of its own, and importing it into `canvas` would put that companion into
//! every canvas program to serve a timer only a `--debug` build ever calls. This is the
//! same `abi_function` body as `datetime::monotonicNanos`, registered under `canvas`.

use crate::codegen::builtins::datetime::func_monotonic_nanos::lower_monotonic_nanos;
use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "frameNanos",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec!["ErrOverflow"],
            body: Body::abi_function(lower_monotonic_nanos),
        }],
    });
}
