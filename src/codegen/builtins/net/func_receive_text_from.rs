//! `net::receiveTextFrom` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/receiveTextFrom.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::receive_text_from` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_receive_text_from(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_io::lower_net_receive_from_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        true,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "receiveTextFrom",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("UdpSocket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", &[], super::udp()),
                super::req("maxBytes", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::Named(super::DATAGRAM_TEXT_TYPE),
            errors: vec![],
            body: super::native_body(lower_receive_text_from, &[]),
        }],
    });
}
