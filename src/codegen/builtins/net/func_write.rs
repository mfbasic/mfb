//! `net::write` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/write.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::write` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_write(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_write_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", &[], super::socket()),
                super::req("bytes", &[], ParameterType::list_of(ParameterType::Byte)),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::native_body(lower_write, &[]),
        }],
    });
}
