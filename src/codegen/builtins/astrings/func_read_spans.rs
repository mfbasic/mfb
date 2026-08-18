//! `astrings::readSpans` — internal-only native overlay bridge (`Body::Native`).
//!
//! Never user-callable (`internal_only`): the source companion (`package.mfb`, an
//! `internal` file) reads the opaque `AttributedString` attribute overlay through
//! this native primitive; `resolver::resolution` rejects it from user source. The
//! lowering stays SHARED in `src/target/shared/code/builder_astrings.rs`.

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

/// Target-generic native lowering for `astrings.readSpans` (registry `Body::Native`
/// `common` slot), delegating to the shared `AttributedString` codegen carrier.
pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    builder
        .lower_astrings_package_call("astrings.readSpans", args)?
        .ok_or_else(|| "astrings.readSpans: no native lowering for these arguments".to_string())
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readSpans",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &[],
                ty: ParameterType::Named("AttributedString"),
                default: DefaultValue::None,
            }],
            return_type: span_list(),
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
