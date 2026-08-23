//! `term::drawVLine` — abi_function member (native terminal I/O).
//!
//! Registers the shared [`gen_os_seam::lower_term_os_seam`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "drawVLine",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("LineStyle, Integer, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "line",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Named("LineStyle"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "col",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "rowA",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "rowB",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(super::gen_os_seam::lower_term_os_seam),
        }],
    });
}
