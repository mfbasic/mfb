//! `net::setReadTimeout` — descriptor entry (native OS-seam). Overloaded over
//! `Socket` / `UdpSocket`. Docs in
//! `src/docs/man/builtins/net/setReadTimeout.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![
            super::req("sock", &[], ty),
            super::req("timeoutMs", &[], ParameterType::Integer),
        ],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: super::native_body(lower_set_read_timeout, &[]),
    }
}

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::set_read_timeout` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_set_read_timeout(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_poll::lower_net_set_timeout_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        false,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setReadTimeout",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket or UdpSocket, Integer"),
        internal_only: false,
        implementations: vec![overload(super::socket()), overload(super::udp())],
    });
}
