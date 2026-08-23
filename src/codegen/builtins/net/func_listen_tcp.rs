//! `net::listenTcp` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/listenTcp.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::listen_tcp` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_listen_tcp(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_net_listen_tcp_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "listenTcp",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("host", &[], ParameterType::String),
                super::req("port", &[], ParameterType::Integer),
                super::opt("backlog", ParameterType::Integer),
            ],
            return_type: ParameterType::Named(super::LISTENER_TYPE_ID),
            errors: vec![],
            body: super::native_body(lower_listen_tcp, &[]),
        }],
    });
}
