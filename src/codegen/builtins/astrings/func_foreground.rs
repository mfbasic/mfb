//! `astrings::foreground` — (r, g, b) Byte-triple `Attribute` constructor
//! (`Body::Rewrite`).
//!
//! Backed by the injected source companion (`package.mfb`): a call rewrites to the
//! internal `__astrings_foreground` FUNC (which packs the triple into a `0xRRGGBB`
//! numeric payload) through the registry's `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

fn color_params() -> Vec<Parameter> {
    ["r", "g", "b"]
        .into_iter()
        .map(|name| Parameter {
            name,
            desc: "",
            aliases: &[],
            ty: ParameterType::Byte,
            default: DefaultValue::None,
        })
        .collect()
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "foreground",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: color_params(),
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::Rewrite("__astrings_foreground"),
        }],
    });
}
