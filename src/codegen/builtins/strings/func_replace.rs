//! `strings::replace` — descriptor entry (`Body::Intrinsic`).
//!
//! The `String` overload of `replace` shares its bare native lowering with the
//! `collections::` `List` overload through `builtins::native_builtin_target`
//! (`lower_replace` etc.), so its `Body` is
//! [`Body::Intrinsic`]; the descriptor exists for return-type resolution, arity,
//! errors, and parameter names.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "replace",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "old",
                    desc: "",
                    aliases: &["needle"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "new",
                    desc: "",
                    aliases: &["replacement"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::Intrinsic,
        }],
    });
}
