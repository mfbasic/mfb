//! plan-176-T: the OpenSSL `tls::close` and `tls::closeListener` lowerings. The shared imports live in
//! `builtins/tls/gen_openssl/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// tls.close
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_close_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 80;
    const REC_OFFSET: usize = 8;
    const SSL_OFFSET: usize = 16;
    const CTX_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const HANDLE_OFFSET: usize = 40;
    const FNPTR_OFFSET: usize = 48;

    let already = format!("{symbol}_already");
    let already_moved = format!("{symbol}_already_moved");
    let load_fail = format!("{symbol}_load_fail");
    let ctx_done = format!("{symbol}_ctx_done");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC_OFFSET),
        // Idempotent: a closed handle returns OK.
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CLOSED),
    ]);
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &already,
        &already_moved,
    ));
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
    ]);
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    // SSL_shutdown(ssl)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_shutdown",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // SSL_free(ssl)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // SSL_CTX_free(ctx) — null-guarded: an accepted (server-side) socket
    // stores 0 here because its context is owned by the listener and shared
    // with sibling sockets; freeing it would double-free / kill live sessions
    // (plan-06-tls-server.md §6.4). Client sockets own their ctx and free it
    // exactly as before.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&ctx_done),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&ctx_done));
    // close(fd)
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Mark the record closed.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, TLS_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // A failure to resolve OpenSSL during close still closes the fd and reports
    // success-ish OK (the session is gone); but to surface load problems we map
    // it to ErrTlsFailed.
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&already));
    // bug-525: an already-closed handle is refused, not reported as success.
    // `mfb spec language resource-management` §15: "an already-closed record is
    // flagged, and a second close is a defined no-op reported as
    // `ErrResourceClosed` rather than an operation on a dead handle" — the same
    // answer `fs`, `tcp` and `udp` have always given. A drop-time re-close still
    // costs nothing: lexical cleanup discards a close failure (§15), so the
    // "close once, or let the scope do it" idiom is unaffected.
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&already_moved));
    emit_fail(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// tls.closeListener (internal listener-shaped close body; the user-facing
// name stays `tls::close` — see plan-06-tls-server.md §4.1/§6.4)
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_close_listener_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 64;
    const REC_OFFSET: usize = 8;
    const FD_OFFSET: usize = 16;
    const CTX_OFFSET: usize = 24;
    const HANDLE_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;

    let already = format!("{symbol}_already");
    let already_moved = format!("{symbol}_already_moved");
    let load_fail = format!("{symbol}_load_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC_OFFSET),
        // Idempotent: a closed handle returns OK.
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
    ]);
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &already,
        &already_moved,
    ));
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
    ]);
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    // SSL_CTX_free(ctx) — the listener owns the shared server context and
    // frees it exactly once here; accepted sockets only point at it.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // close(fd)
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Mark the record closed.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, TLS_LISTENER_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&already));
    // bug-525: an already-closed handle is refused, not reported as success.
    // `mfb spec language resource-management` §15: "an already-closed record is
    // flagged, and a second close is a defined no-op reported as
    // `ErrResourceClosed` rather than an operation on a dead handle" — the same
    // answer `fs`, `tcp` and `udp` have always given. A drop-time re-close still
    // costs nothing: lexical cleanup discards a close failure (§15), so the
    // "close once, or let the scope do it" idiom is unaffected.
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&already_moved));
    emit_fail(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}
