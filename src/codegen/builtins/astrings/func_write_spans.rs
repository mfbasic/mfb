//! `astrings::writeSpans` — internal-only native overlay bridge (`Body::Native`).
//!
//! Never user-callable (`internal_only`): the source companion (`package.mfb`, an
//! `internal` file) rebuilds the opaque `AttributedString` attribute overlay through
//! this native primitive. The lowering stays SHARED in
//! `src/target/shared/code/builder_astrings.rs`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
fn span_list() -> ParameterType {
    ParameterType::list_of(ParameterType::Named("AttrSpan"))
}

/// Target-generic native lowering for `astrings.writeSpans` (registry `Body::Native`
/// `common` slot), delegating to the shared `AttributedString` codegen carrier.
pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    builder
        .lower_astrings_package_call("astrings.writeSpans", args)?
        .ok_or_else(|| "astrings.writeSpans: no native lowering for these arguments".to_string())
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeSpans",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Named("AttributedString"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "spans",
                    desc: "",
                    aliases: &[],
                    ty: span_list(),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Named("AttributedString"),
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
