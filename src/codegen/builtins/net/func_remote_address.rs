//! `net::remoteAddress` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/remoteAddress.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::remote_address` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_remote_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_address_helper(&symbol, ctx.platform_imports, ctx.platform, true)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "remoteAddress",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("sock", &[], super::socket())],
            return_type: ParameterType::named(super::ADDRESS_TYPE),
            errors: vec![],
            body: super::native_body(lower_remote_address, &[]),
        }],
    });
}
