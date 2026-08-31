// The macOS endpoint queries for `tls::localAddress` / `tls::remoteAddress`
// (plan-110-D Phase 2).
//
// Linux and Windows get these for free: their TLS record keeps the OS descriptor
// in the canonical handle slot, so the `net` address emitter — plain
// `getsockname`/`getpeername` — works unchanged on a TLS socket. macOS cannot,
// because Network.framework owns the socket and the record's handle slot holds an
// `nw_connection`, not a descriptor (plan-110-D §C3/§C4).
//
// Network.framework does expose the endpoints, so this is a real implementation
// rather than an `ErrUnsupported` stub:
//
//   both:  nw_connection_copy_current_path(conn)          -> nw_path      (+1)
//          nw_path_copy_effective_{local,remote}_endpoint -> nw_endpoint  (+1)
//          nw_endpoint_get_address(endpoint)              -> const struct sockaddr *
//
// Note it is the PATH's effective endpoints, not `nw_connection_copy_endpoint`.
// That call returns the endpoint the connection was created from — a HOST
// endpoint here — and `nw_endpoint_get_address` answers only for
// `nw_endpoint_type_address`, so it would hand back NULL.
//
// `nw_endpoint_get_address` is a *get*, not a *copy*: the sockaddr is owned by the
// endpoint and is only valid while the endpoint is retained, so the Address record
// is built BEFORE anything is released. Every `copy_`/`create_` result above
// carries a +1 that this code drops on both the success and failure paths.

use super::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// `tls::localAddress` / `tls::remoteAddress` on macOS. `remote` selects the
/// peer endpoint over the effective local one.
pub(crate) fn lower_tls_address_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    remote: bool,
) -> Result<TlsBodyParts, String> {
    const FRAME_SIZE: usize = 256;
    const REC: usize = 8; // the TLS socket record
    const CONN: usize = 16; // its nw_connection
    const HANDLE: usize = 24; // dlopen handle for Network.framework
    const FNPTR: usize = 32; // scratch dlsym result
    const PATH: usize = 40; // nw_path (+1)
    const ENDPOINT: usize = 48; // nw_endpoint (+1)
    const SADDR: usize = 56; // const struct sockaddr *
    const HOSTLEN: usize = 64; // scratch for the shared Address builder
    const DST: usize = 72; // scratch
    const AHOST: usize = 80; // scratch
    const ADDRREC: usize = 88; // the built Address record

    let closed = format!("{symbol}_closed");
    let load_fail = format!("{symbol}_load_fail");
    let query_fail = format!("{symbol}_query_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let release_fail = format!("{symbol}_release_fail");
    let done = format!("{symbol}_done");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();

    // The socket record arrives in the first argument register. Refuse a closed
    // handle before touching the connection.
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), RESOURCE_OFFSET_HANDLE),
        abi::store_u64(&v9, abi::stack_pointer(), CONN),
        // Nothing retained yet; the failure tails release conditionally.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PATH),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), ENDPOINT),
    ]);

    // dlopen(Network.framework)
    emit_data_address(
        symbol,
        abi::return_register(),
        MACLIB_SYMBOL,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call("dlopen", symbol, platform_imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&load_fail),
    ]);

    // BOTH ends come from the connection's current PATH, not from
    // `nw_connection_copy_endpoint`. That call returns the endpoint the
    // connection was *created* from, which here is a HOST endpoint
    // (`nw_endpoint_create_host`), and `nw_endpoint_get_address` returns NULL for
    // a host endpoint — it only answers for `nw_endpoint_type_address`. The
    // path's effective endpoints are the resolved addresses actually in use.
    emit_nw_call1(
        symbol,
        platform,
        platform_imports,
        &mut ins,
        &mut rel,
        &v9,
        HANDLE,
        FNPTR,
        "nw_connection_copy_current_path",
        CONN,
        &load_fail,
    )?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&query_fail),
    ]);
    emit_nw_call1(
        symbol,
        platform,
        platform_imports,
        &mut ins,
        &mut rel,
        &v9,
        HANDLE,
        FNPTR,
        if remote {
            "nw_path_copy_effective_remote_endpoint"
        } else {
            "nw_path_copy_effective_local_endpoint"
        },
        PATH,
        &load_fail,
    )?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&query_fail),
    ]);

    // saddr = nw_endpoint_get_address(endpoint) — BORROWED, valid only while the
    // endpoint is retained, so the Address record is built before any release.
    emit_nw_call1(
        symbol,
        platform,
        platform_imports,
        &mut ins,
        &mut rel,
        &v9,
        HANDLE,
        FNPTR,
        "nw_endpoint_get_address",
        ENDPOINT,
        &load_fail,
    )?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SADDR),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&query_fail),
    ]);

    // Build the `net::Address` from the borrowed sockaddr with the same shared
    // builder `net`/`tcp`/`udp` use, so every package renders an endpoint
    // identically.
    crate::codegen::os::socket::shared::emit_address_from_sockaddr(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        "tlsaddr",
        SADDR,
        HOSTLEN,
        DST,
        AHOST,
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        ADDRREC,
    ));

    // Drop the +1s now that the sockaddr has been copied out.
    emit_release_if_set(
        symbol,
        platform,
        platform_imports,
        &mut ins,
        &mut rel,
        &v9,
        &v10,
        HANDLE,
        FNPTR,
        ENDPOINT,
        "ok_ep",
        &release_fail,
    )?;
    emit_release_if_set(
        symbol,
        platform,
        platform_imports,
        &mut ins,
        &mut rel,
        &v9,
        &v10,
        HANDLE,
        FNPTR,
        PATH,
        "ok_path",
        &release_fail,
    )?;
    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), ADDRREC),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Failure tails. `query_fail` may hold either or both retains; both are
    // released before the error is raised so a failed query cannot leak.
    ins.push(abi::label(&query_fail));
    emit_release_if_set(
        symbol,
        platform,
        platform_imports,
        &mut ins,
        &mut rel,
        &v9,
        &v10,
        HANDLE,
        FNPTR,
        ENDPOINT,
        "fail_ep",
        &release_fail,
    )?;
    emit_release_if_set(
        symbol,
        platform,
        platform_imports,
        &mut ins,
        &mut rel,
        &v9,
        &v10,
        HANDLE,
        FNPTR,
        PATH,
        "fail_path",
        &release_fail,
    )?;
    ins.push(abi::label(&release_fail));
    ins.push(abi::label(&addr_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    Ok((ins, rel, FRAME_SIZE))
}

/// `tls::localAddress(listener)` on macOS (bug-465).
///
/// Linux and Windows answer this with the same `getsockname` the plaintext
/// `tcp::localAddress(listener)` uses: their TLS `Listener` keeps the listening
/// descriptor in the canonical handle slot. macOS cannot — the slot holds an
/// `nw_listener`, and Network.framework has no listener-side counterpart to the
/// connection's `nw_connection_copy_current_path`. `nw_listener_get_port` is the
/// entire address surface a listener exposes.
///
/// So the two halves come from two places: the port from `nw_listener_get_port`,
/// and the host from the C string `tls::listen` parked at `REC_LHOST` when it
/// built the local endpoint. That string is the `host` argument as given (or
/// `"0.0.0.0"` for the bind-all spelling), which is what makes this differ, in
/// one visible way, from the descriptor-based answer: bind a *name* like
/// `"localhost"` and `getsockname` reports the resolved `127.0.0.1` while this
/// reports `localhost`. Documented on the member; there is no macOS API that
/// would close the gap.
///
/// `nw_listener_get_port` returns a `uint16_t` in **host** byte order, already
/// the shape the `Address` record wants — unlike the `sockaddr` path, which
/// decodes two network-order bytes. It is stored through a zeroed slot with a
/// 16-bit store so the C return's undefined upper bits cannot leak into the port.
pub(crate) fn lower_tls_listener_address_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    const FRAME_SIZE: usize = 128;
    // The TLS listener record. Parked for symmetry with `lower_tls_address_macos`
    // and never read back: everything this body needs (the nw_listener, the bound
    // host) is lifted out of the record in the prologue below, before anything can
    // clobber x0. Kept rather than dropped so the two emitters in this file stay
    // line-for-line comparable — they are read side by side, and the saved store
    // is four bytes in a body that dlopens a framework.
    const REC: usize = 8;
    const LISTENER: usize = 16; // its nw_listener
    const HANDLE: usize = 24; // dlopen handle for Network.framework
    const FNPTR: usize = 32; // scratch dlsym result
    const HOSTP: usize = 40; // const char * — the bound host
    const PORT: usize = 48; // host-order port from nw_listener_get_port
    const HOSTLEN: usize = 56; // scratch for the shared Address builder
    const AHOST: usize = 64; // scratch

    let closed = format!("{symbol}_closed");
    let load_fail = format!("{symbol}_load_fail");
    let query_fail = format!("{symbol}_query_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();

    // The listener record arrives in the first argument register. Refuse a closed
    // handle before touching the listener.
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), RESOURCE_OFFSET_HANDLE),
        abi::store_u64(&v9, abi::stack_pointer(), LISTENER),
        // The bound host, parked by `tls::listen`. A null here would mean a
        // listener record this build did not write; refuse rather than walk it.
        abi::load_u64(&v9, abi::return_register(), REC_LHOST),
        abi::store_u64(&v9, abi::stack_pointer(), HOSTP),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&query_fail),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PORT),
    ]);

    emit_dlopen_maclib(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        &load_fail,
    )?;

    // port = nw_listener_get_port(listener) — uint16_t, host byte order. Stored
    // 16 bits wide into the zeroed slot above.
    emit_nw_call1(
        symbol,
        platform,
        platform_imports,
        &mut ins,
        &mut rel,
        &v9,
        HANDLE,
        FNPTR,
        "nw_listener_get_port",
        LISTENER,
        &load_fail,
    )?;
    ins.push(abi::store_u16(
        abi::return_register(),
        abi::stack_pointer(),
        PORT,
    ));

    // Same `net::Address` the sockaddr path builds, so `tcp` and `tls` render an
    // endpoint identically.
    crate::codegen::os::socket::shared::emit_address_from_host_and_port(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        "tlslisten",
        HOSTP,
        PORT,
        HOSTLEN,
        AHOST,
        &alloc_fail,
        &mut vregs,
    );
    ins.extend([
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Failure tails. Nothing is retained anywhere above, so none of them release.
    ins.push(abi::label(&query_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    Ok((ins, rel, FRAME_SIZE))
}

/// `dlsym(handle, name)` then call it with one pointer argument loaded from
/// `arg_off`, leaving the result in the return register.
#[allow(clippy::too_many_arguments)]
fn emit_nw_call1(
    symbol: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    scratch: &str,
    handle_off: usize,
    fnptr_off: usize,
    name: &str,
    arg_off: usize,
    fail: &str,
) -> Result<(), String> {
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ins,
            relocations: rel,
        },
        handle_off,
        name,
        fnptr_off,
        fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), arg_off),
        abi::load_u64(scratch, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(scratch),
    ]);
    Ok(())
}

/// `nw_release(x)` for the slot at `off`, skipped when it is null, then the slot
/// is zeroed so a later tail cannot release it twice.
#[allow(clippy::too_many_arguments)]
fn emit_release_if_set(
    symbol: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    scratch: &str,
    scratch2: &str,
    handle_off: usize,
    fnptr_off: usize,
    off: usize,
    tag: &str,
    fail: &str,
) -> Result<(), String> {
    let skip = format!("{symbol}_rel_skip_{tag}");
    ins.extend([
        abi::load_u64(scratch2, abi::stack_pointer(), off),
        abi::compare_immediate(scratch2, "0"),
        abi::branch_eq(&skip),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ins,
            relocations: rel,
        },
        handle_off,
        "nw_release",
        fnptr_off,
        fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), off),
        abi::load_u64(scratch, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(scratch),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), off),
        abi::label(&skip),
    ]);
    Ok(())
}
