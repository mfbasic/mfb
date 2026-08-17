//! `strings::find` — descriptor entry (`Body::Intrinsic`).
//!
//! The `String` overload of `find` shares its bare native lowering with the
//! `collections::` `List` overload through `builtins::native_builtin_target`
//! (`lower_find` etc.), so its `Body` is
//! [`Body::Intrinsic`]; the descriptor exists for return-type resolution, arity,
//! errors, and parameter names.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "find",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
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
                    name: "needle",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "start",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
            body: Body::Intrinsic,
        }],
    });
}
