//! `term::drawText` — OS-seam member (native terminal I/O).
//!
//! Registers a `Body::native_os_seam` whose `posix`/`win` slots both hold the
//! shared `term` dispatcher (`super::native::lower_term_helper`), reached by the
//! generic OS-seam dispatch. The heavy terminal emission stays in the shared code
//! layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "drawText",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Integer, Integer, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "x",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "y",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "text",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native_os_seam(
                Some(super::native::lower_term_helper),
                Some(super::native::lower_term_helper),
                &[],
            ),
        }],
    });
}
