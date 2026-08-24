//! `net::poll` — descriptor entry (native OS-seam). Return-type-overloaded on
//! argument shape: a scalar `Socket` yields `Boolean` (readiness query), a
//! `List OF RES net.Socket` yields a borrowed `Socket` (readiness multiplex, the
//! `pollList` code form / `os_alias`). Two `Implementation`s, the datetime/net
//! idiom. Docs in `src/docs/man/builtins/net/poll.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `net::poll` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_poll(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == "net.pollList" {
        super::gen_poll::lower_net_poll_list_helper(&symbol, ctx.platform_imports, ctx.platform)?
    } else {
        super::gen_poll::lower_net_poll_helper(&symbol, ctx.platform_imports, ctx.platform)?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "poll",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: Some("Socket, Integer or List OF RES Socket, Integer"),
        internal_only: false,
        implementations: vec![
            // Scalar readiness query: `poll(Socket[, timeoutMs]) AS Boolean`.
            Implementation {
                params: vec![
                    super::req("sock", &[], super::socket()),
                    super::opt("timeoutMs", ParameterType::Integer),
                ],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: super::native_body(lower_poll, &[]),
            },
            // Readiness multiplex: `poll(List OF RES net.Socket[, timeoutMs]) AS
            // Socket` (borrowed). Emits the `net.pollList` code form.
            Implementation {
                params: vec![
                    super::req("socks", &[], ParameterType::list_of(super::socket())),
                    super::opt("timeoutMs", ParameterType::Integer),
                ],
                return_type: ParameterType::named(super::SOCKET_TYPE_ID),
                errors: vec![],
                body: super::native_body(lower_poll, &["pollList"]),
            },
        ],
    });
}
