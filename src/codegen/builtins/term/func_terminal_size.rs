//! `term::terminalSize` — OS-seam member (native terminal I/O).
//!
//! Registers a `Body::native_os_seam` whose `posix`/`win` slots both hold the
//! shared `term` dispatcher (`super::native::lower_term_helper`), reached by the
//! generic OS-seam dispatch. The heavy terminal emission stays in the shared code
//! layer (`code::lower_term_helper` / `emit_app_term_helper`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "terminalSize",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Named("TermSize"),
            errors: vec![],
            body: Body::native_os_seam(
                Some(super::native::lower_term_helper),
                Some(super::native::lower_term_helper),
                &[],
            ),
        }],
    });
}
