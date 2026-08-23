//! `net::close` — descriptor entry (native OS-seam). Spans the resource union
//! (`Socket` / `Listener` / `UdpSocket`) as three overloads, all returning
//! `Nothing` and all lowering to `net.close` (the datetime/tls idiom, no custom
//! resolver). `close` consumes the handle it is given (see
//! `syntaxcheck::builtins::net_consumes_argument`). Docs in
//! `src/docs/man/builtins/net/close.md`.

use crate::codegen::registry::{Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "resource",
            desc: "",
            aliases: &["sock", "listener"],
            ty,
            default: crate::codegen::registry::DefaultValue::None,
        }],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: super::native_body(lower_close, &[]),
    }
}

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::close` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_close(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        crate::codegen::builtins::fs::gen_handle::lower_fs_close_helper(
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
        name: "close",
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
