//! `term::drawGlyph` — OS-seam member (native terminal I/O).
//!
//! Registers a `Body::native_os_seam` whose `posix`/`win` slots both hold the
//! shared `term` dispatcher (`super::native::lower_term_helper`), reached by the
//! generic OS-seam dispatch. The heavy terminal emission stays in the shared code
//! layer (`code::lower_term_helper` / `emit_app_term_helper`).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "drawGlyph",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Integer, Integer, Integer"),
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
                    name: "codepoint",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
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
