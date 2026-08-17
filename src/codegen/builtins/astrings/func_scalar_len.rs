//! `astrings::scalarLen` — internal-only native overlay bridge (`Body::Native`).
//!
//! Never user-callable (`internal_only`): the source companion (`package.mfb`, an
//! `internal` file) reads the visible scalar count of an opaque `AttributedString`
//! through this native primitive for inclusive-range bounds validation. The lowering
//! stays SHARED in `src/target/shared/code/builder_astrings.rs`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

/// Target-generic native lowering for `astrings.scalarLen` (registry `Body::Native`
/// `common` slot), delegating to the shared `AttributedString` codegen carrier.
pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    builder
        .lower_astrings_package_call("astrings.scalarLen", args)?
        .ok_or_else(|| "astrings.scalarLen: no native lowering for these arguments".to_string())
}

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "scalarLen",
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
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
