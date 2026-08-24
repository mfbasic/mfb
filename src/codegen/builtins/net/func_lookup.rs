//! `net::lookup` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/lookup.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::lookup` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_lookup(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_lookup_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "lookup",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("host", &[], ParameterType::String),
                super::opt("port", ParameterType::Integer),
            ],
            return_type: ParameterType::list_of(ParameterType::named(super::ADDRESS_TYPE)),
            errors: vec![],
            body: super::native_body(lower_lookup, &[]),
        }],
    });
}
