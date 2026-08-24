//! `net::connectTcp` — descriptor entry (native OS-seam). Four argument-shape
//! overloads (host/port, host/port/timeout, address, address/timeout); the two
//! `Address` forms lower to the `net.connectTcpAddr` code form (an `os_alias`), the
//! others to `net.connectTcp`. The overload split + timeout padding lives in
//! `builder_values`. Docs in `src/docs/man/builtins/net/connectTcp.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::connect_tcp` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_connect_tcp(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == "net.connectTcpAddr" {
        super::gen_shared::lower_net_connect_tcp_addr_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?
    } else {
        super::gen_shared::lower_net_connect_tcp_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let ret = || ParameterType::named(super::SOCKET_TYPE_ID);
    pkg.add_function(RegistryFunction {
        name: "connectTcp",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("String, Integer, Integer or Address, Integer"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    super::req("host", &[], ParameterType::String),
                    super::req("port", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect_tcp, &[]),
            },
            Implementation {
                params: vec![
                    super::req("host", &[], ParameterType::String),
                    super::req("port", &[], ParameterType::Integer),
                    super::req("timeoutMs", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect_tcp, &[]),
            },
            Implementation {
                params: vec![super::req(
                    "address",
                    &[],
                    ParameterType::named(super::ADDRESS_TYPE),
                )],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect_tcp, &["connectTcpAddr"]),
            },
            Implementation {
                params: vec![
                    super::req("address", &[], ParameterType::named(super::ADDRESS_TYPE)),
                    super::req("timeoutMs", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect_tcp, &[]),
            },
        ],
    });
}
