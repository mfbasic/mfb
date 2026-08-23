//! `net::localAddress` — descriptor entry (native OS-seam). Overloaded over the
//! `Socket` / `Listener` / `UdpSocket` union, all returning `Address`. Docs in
//! `src/docs/man/builtins/net/localAddress.md`.

use crate::codegen::registry::{Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "sock",
            desc: "",
            aliases: &["listener"],
            ty,
            default: crate::codegen::registry::DefaultValue::None,
        }],
        return_type: ParameterType::Named(super::ADDRESS_TYPE),
        errors: vec![],
        body: super::native_body(lower_local_address, &[]),
    }
}

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::local_address` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_local_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_io::lower_net_address_helper(
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
        name: "localAddress",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket or Listener or UdpSocket"),
        internal_only: false,
        implementations: vec![
            overload(super::socket()),
            overload(super::listener()),
            overload(super::udp()),
        ],
    });
}
