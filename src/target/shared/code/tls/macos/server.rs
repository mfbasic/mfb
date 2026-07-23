use super::*;

// ===========================================================================
// Server side: tls.listen / tls.accept / tls.closeListener
// (plan-06-tls-server.md §7)
// ===========================================================================

#[allow(clippy::too_many_arguments)]
/// Read the whole file named by the MFBASIC `String` at `sp + path_off` into a
/// fresh arena buffer: pointer at `sp + buf_off`, byte length at
/// `sp + len_off`. `open_fail` is taken when the file cannot be opened (no fd
/// yet); `read_fail_fd` when a seek/read fails or the file is empty (the open
/// fd is at `sp + fd_off` for the caller to close).
fn emit_read_whole_file(
    ctx: &mut EmitCtx,
    prefix: &str,
    path_off: usize,
    cstr_off: usize,
    fd_off: usize,
    readoff_off: usize,
    buf_off: usize,
    len_off: usize,
    open_fail: &str,
    read_fail_fd: &str,
    alloc_fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let read_loop = format!("{symbol}_{prefix}_read");
    let read_done = format!("{symbol}_{prefix}_read_done");
    emit_cstring(
        symbol,
        prefix,
        path_off,
        cstr_off,
        alloc_fail,
        ctx.instructions,
        ctx.relocations,
    );
    // fd = open(path, O_RDONLY)
    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), cstr_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_open_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        // bug-102.3: narrow the C int `open` return before the signed compare
        // (lseek/read below return 64-bit off_t/ssize_t and must NOT be narrowed).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(open_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        // len = lseek(fd, 0, SEEK_END); an empty file is not a valid PEM.
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "2"),
    ]);
    platform.emit_seek_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(read_fail_fd),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), len_off),
        // rewind: lseek(fd, 0, SEEK_SET)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_seek_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    // buf = arena_alloc(len, 1)
    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), len_off),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, ctx.instructions, ctx.relocations, alloc_fail);
    ctx.instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), buf_off),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), readoff_off),
        abi::label(&read_loop),
        abi::load_u64("%v9", abi::stack_pointer(), readoff_off),
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&read_done),
        // n = read(fd, buf + off, len - off)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), buf_off),
        abi::add_registers(abi::ARG[1], abi::ARG[1], "%v9"),
        abi::subtract_registers(abi::ARG[2], "%v10", "%v9"),
    ]);
    platform.emit_read_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(read_fail_fd),
        abi::load_u64("%v9", abi::stack_pointer(), readoff_off),
        abi::add_registers("%v9", "%v9", abi::return_register()),
        abi::store_u64("%v9", abi::stack_pointer(), readoff_off),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
    ]);
    platform.emit_close_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// Import one PEM item (a certificate or a private key) from the bytes at
/// `sp + buf_off`/`len_off` via `CFDataCreate` + `SecItemImport`, leaving the
/// first imported item (`SecCertificateRef`/`SecKeyRef`) at `sp + ref_off`.
fn emit_import_pem_item(
    ctx: &mut EmitCtx,
    buf_off: usize,
    len_off: usize,
    data_off: usize,
    items_off: usize,
    ref_off: usize,
    sec_handle_off: usize,
    cf_handle_off: usize,
    fnptr_off: usize,
    fail: &str,
    load_fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // data = CFDataCreate(NULL, buf, len)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        cf_handle_off,
        "CFDataCreate",
        fnptr_off,
        load_fail,
    )?;
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), buf_off),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), len_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), data_off),
    ]);
    // SecItemImport(data, NULL, NULL, NULL, 0, NULL, NULL, &items) == errSecSuccess
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        sec_handle_off,
        "SecItemImport",
        fnptr_off,
        load_fail,
    )?;
    ctx.instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), items_off),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), data_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::move_immediate(abi::ARG[4], "Integer", "0"),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
        abi::move_immediate(abi::ARG[6], "Integer", "0"),
        abi::add_immediate(abi::ARG[7], abi::stack_pointer(), items_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(fail),
        abi::load_u64("%v9", abi::stack_pointer(), items_off),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(fail),
    ]);
    // CFArrayGetCount(items) >= 1
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        cf_handle_off,
        "CFArrayGetCount",
        fnptr_off,
        load_fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), items_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_lt(fail),
    ]);
    // ref = CFArrayGetValueAtIndex(items, 0)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        cf_handle_off,
        "CFArrayGetValueAtIndex",
        fnptr_off,
        load_fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), items_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ref_off),
    ]);
    // Take ownership of the extracted ref, then drop the two +1 objects this
    // import created (bug-236).
    //
    // `CFArrayGetValueAtIndex` follows the CoreFoundation *Get* rule: the ref is
    // UNRETAINED and owned by the array. Releasing ITEMS while holding only that
    // non-owned pointer would leave `ref_off` dangling — a use-after-free, worse
    // than the leak. So `CFRetain` first; the caller releases the ref once
    // `SecIdentityCreate` has taken its own.
    //
    // Releasing here (rather than after the caller is done with both items) is
    // also what makes the two imports safe: the cert and key calls share the DATA
    // and ITEMS slots, so the key import OVERWRITES the cert's handles. Deferring
    // would not merely leak them — it would lose the only pointers to them.
    //
    // `SecItemImport` does not retain DATA beyond the call, so it is released
    // here too.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        cf_handle_off,
        "CFRetain",
        fnptr_off,
        load_fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ref_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
    ]);
    emit_cf_release_slot(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        cf_handle_off,
        items_off,
        fnptr_off,
        load_fail,
    )?;
    emit_cf_release_slot(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        cf_handle_off,
        data_off,
        fnptr_off,
        load_fail,
    )?;
    Ok(())
}

/// `CFRelease(*(sp + slot_off))` when the slot is non-NULL, then NULL the slot so
/// a later release of the same slot is a no-op (bug-236). Resolves `CFRelease`
/// through the already-open CoreFoundation handle.
///
/// The NULL guard and the clear are what let the error exits release
/// unconditionally: an exit taken before the slot was filled, or after it was
/// already released, does nothing rather than over-releasing.
fn emit_cf_release_slot(
    ctx: &mut EmitCtx,
    cf_handle_off: usize,
    slot_off: usize,
    fnptr_off: usize,
    load_fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // Unique per emission point, not per slot: a slot is released from several
    // sites in one function (once per PEM import, again on the error exit), so a
    // slot-keyed label would collide.
    let skip = format!("{symbol}_cf_rel_skip_{slot_off}_{}", ctx.instructions.len());
    ctx.instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), slot_off),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&skip),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        cf_handle_off,
        "CFRelease",
        fnptr_off,
        load_fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), slot_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), slot_off),
        abi::label(&skip),
    ]);
    Ok(())
}

pub(in crate::target::shared::code::tls) fn lower_tls_listen_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 448;
    const HOST: usize = 8;
    const PORT: usize = 16;
    const CERT: usize = 24;
    const KEY: usize = 32;
    // x4 (backlog) is accepted for ABI parity but unused: Network.framework
    // manages its own accept backlog.
    const NWH: usize = 40;
    const SECH: usize = 48;
    const CFH: usize = 56;
    const FNPTR: usize = 64;
    const HOSTCSTR: usize = 72;
    const PORTCSTR: usize = 80;
    const PORTBUF: usize = 88; // 88..112
    const PATHCSTR: usize = 112;
    const FILEFD: usize = 120;
    const READOFF: usize = 128;
    const CERTBUF: usize = 136;
    const CERTLEN: usize = 144;
    const KEYBUF: usize = 152;
    const KEYLEN: usize = 160;
    const DATA: usize = 168;
    const ITEMS: usize = 176;
    const CERTREF: usize = 184;
    const KEYREF: usize = 192;
    const IDENT: usize = 200;
    const SECIDENT: usize = 208;
    const CFG: usize = 216;
    const ENDPOINT: usize = 224;
    const PARAMS: usize = 232;
    const LISTENER: usize = 240;
    const QUEUE: usize = 248;
    const LCTX: usize = 256;
    const CFGBLOCK: usize = 264; // 264..328: the identity-config block literal
    const SBLOCK: usize = 328; // 328..368: state-changed block literal
    const CBLOCK: usize = 368; // 368..408: new-connection block literal
    const WAITFN: usize = 408;

    let cert_fail = format!("{symbol}_cert_fail");
    let read_fail_fd = format!("{symbol}_read_fail_fd");
    let net_fail = format!("{symbol}_net_fail");
    let listen_fail = format!("{symbol}_listen_fail");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let null_host = format!("{symbol}_null_host");
    let host_ready = format!("{symbol}_host_ready");
    let wait_loop = format!("{symbol}_wait");
    let ready = format!("{symbol}_ready");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // x0 = host; x1 = port; x2 = certPath; x3 = keyPath; x4 = backlog (unused).
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), CERT),
        abi::store_u64(abi::ARG[3], abi::stack_pointer(), KEY),
    ]);
    // Read the PEM pair into arena buffers before touching any framework.
    emit_read_whole_file(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        "cert",
        CERT,
        PATHCSTR,
        FILEFD,
        READOFF,
        CERTBUF,
        CERTLEN,
        &cert_fail,
        &read_fail_fd,
        &alloc_fail,
    )?;
    emit_read_whole_file(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        "key",
        KEY,
        PATHCSTR,
        FILEFD,
        READOFF,
        KEYBUF,
        KEYLEN,
        &cert_fail,
        &read_fail_fd,
        &alloc_fail,
    )?;
    // dlopen Network.framework, Security.framework, CoreFoundation.
    emit_dlopen_at(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        MACLIB_SYMBOL,
        NWH,
        &load_fail,
    )?;
    emit_dlopen_at(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        MACSEC_SYMBOL,
        SECH,
        &load_fail,
    )?;
    emit_dlopen_at(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        MACCF_SYMBOL,
        CFH,
        &load_fail,
    )?;
    // certRef / keyRef from the PEM bytes.
    emit_import_pem_item(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        CERTBUF,
        CERTLEN,
        DATA,
        ITEMS,
        CERTREF,
        SECH,
        CFH,
        FNPTR,
        &cert_fail,
        &load_fail,
    )?;
    emit_import_pem_item(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        KEYBUF,
        KEYLEN,
        DATA,
        ITEMS,
        KEYREF,
        SECH,
        CFH,
        FNPTR,
        &cert_fail,
        &load_fail,
    )?;
    // identity = SecIdentityCreate(NULL, certRef, keyRef) — the keychain-free
    // cert+key pairing entry point in Security.framework (resolved via dlsym;
    // absent => ErrTlsFailed, never a stub).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        SECH,
        "SecIdentityCreate",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CERTREF),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), KEYREF),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&cert_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), IDENT),
    ]);
    // SecIdentityCreate took its own references to the cert and key, so drop the
    // ones `emit_import_pem_item` retained for us (bug-236). Done here rather
    // than at function exit so the identity holds the only remaining refs, and so
    // the error exits below have nothing left to unwind for them.
    for slot in [CERTREF, KEYREF] {
        emit_cf_release_slot(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            CFH,
            slot,
            FNPTR,
            &load_fail,
        )?;
    }
    // secIdentity = sec_identity_create(identity)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        SECH,
        "sec_identity_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), IDENT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&cert_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SECIDENT),
    ]);
    // Build the configure-TLS block that installs the local identity:
    // CFG_INVOKE copies the sec_protocol_options and calls the captured
    // setter with the captured payload — here
    // sec_protocol_options_set_local_identity(options, secIdentity).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "_NSConcreteStackBlock",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, "%v9", CFG_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        CFGBLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, "%v9", CFG_DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        CFGBLOCK + BLK_DESC,
    ));
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SECIDENT),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_SNAME),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_tls_copy_sec_protocol_options",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_COPYFN),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        SECH,
        "sec_protocol_options_set_local_identity",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_SETFN),
    ]);
    // nw_release: the invoke releases the +1 sec_protocol_options the copy fn
    // returns, so each listener stops leaking one (bug-116).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_RELEASEFN),
    ]);
    // cfg = *_nw_parameters_configure_protocol_default_configuration
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "_nw_parameters_configure_protocol_default_configuration",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", "%v9", 0),
        abi::store_u64("%v9", abi::stack_pointer(), CFG),
    ]);
    // params = nw_parameters_create_secure_tcp(&cfgBlock, cfg)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_parameters_create_secure_tcp",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), CFGBLOCK),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CFG),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
    ]);
    // nw_parameters_set_reuse_local_address(params, true)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_parameters_set_reuse_local_address",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Local endpoint: empty host binds all interfaces ("0.0.0.0").
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), HOST),
        abi::load_u64("%v10", "%v9", 0),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&null_host),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST,
        HOSTCSTR,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::branch(&host_ready));
    ins.push(abi::label(&null_host));
    emit_data_address(symbol, "%v9", ANYHOST_SYMBOL, &mut ins, &mut rel);
    ins.extend([
        abi::store_u64("%v9", abi::stack_pointer(), HOSTCSTR),
        abi::label(&host_ready),
    ]);
    // itoa(port) -> NUL-terminated decimal at PORTBUF, pointer in PORTCSTR.
    emit_port_itoa(symbol, PORT, PORTBUF, PORTCSTR, &mut ins);
    // endpoint = nw_endpoint_create_host(host, port)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_endpoint_create_host",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HOSTCSTR),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), PORTCSTR),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
    ]);
    // nw_parameters_set_local_endpoint(params, endpoint)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_parameters_set_local_endpoint",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), ENDPOINT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // listener = nw_listener_create(params)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_listener_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
    ]);
    // The endpoint is retained into the parameters (set_local_endpoint) and the
    // parameters are retained by the listener (nw_listener_create), so release
    // our own references now; otherwise every successful listen leaks one
    // nw_endpoint and one nw_parameters (bug-55).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // queue = dispatch_queue_create("mfb.tls", NULL)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_queue_create",
        FNPTR,
        &load_fail,
    )?;
    emit_data_address(
        symbol,
        abi::return_register(),
        QLABEL_SYMBOL,
        &mut ins,
        &mut rel,
    );
    ins.extend([
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), QUEUE),
    ]);
    // Allocate + initialize the listener context (shared ctx prefix + ring).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", LCTX_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), LCTX));
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::store_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::store_u64(abi::ZERO, "%v9", CTX_STATE),
        abi::store_u64(abi::ZERO, "%v9", CTX_CONTENT),
        abi::store_u64(abi::ZERO, "%v9", CTX_ERROR),
        abi::store_u64(abi::ZERO, "%v9", LCTX_HEAD),
        abi::store_u64(abi::ZERO, "%v9", LCTX_TAIL),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_signal",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::store_u64("%v10", "%v9", CTX_SIGNAL),
    ]);
    // ctx->retain = &nw_retain (the conn handler retains queued connections).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_retain",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::store_u64("%v10", "%v9", CTX_RETAIN),
    ]);
    // nw_listener_set_queue(listener, queue)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_listener_set_queue",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // State-changed handler (the shared STATE_INVOKE trampoline over lctx).
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        STATE_INVOKE,
        LCTX,
        SBLOCK,
        FNPTR,
        &load_fail,
    )?;
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_listener_set_state_changed_handler",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), SBLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // New-connection handler (retain + enqueue + signal).
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        LCONN_INVOKE,
        LCTX,
        CBLOCK,
        FNPTR,
        &load_fail,
    )?;
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_listener_set_new_connection_handler",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CBLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // nw_listener_start(listener)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_listener_start",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Wait until the listener is ready (bind complete) or failed.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), WAITFN),
        abi::label(&wait_loop),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::bitwise_not(abi::ARG[1], abi::ARG[1]), // DISPATCH_TIME_FOREVER
        abi::load_u64("%v10", abi::stack_pointer(), WAITFN),
        abi::branch_link_register("%v10"),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u32("%v10", "%v9", CTX_STATE),
        abi::compare_immediate("%v10", NW_LISTENER_READY),
        abi::branch_eq(&ready),
        abi::compare_immediate("%v10", NW_LISTENER_FAILED),
        abi::branch_ge(&listen_fail), // failed / cancelled
        abi::branch(&wait_loop),      // invalid / waiting
        abi::label(&ready),
    ]);
    // Build the TlsListener record { closed=0, listener, queue, lctx }.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::ZERO, abi::RET[1], REC_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), LISTENER),
        abi::store_u64("%v9", abi::RET[1], REC_CONN),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE),
        abi::store_u64("%v9", abi::RET[1], REC_QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::store_u64("%v9", abi::RET[1], REC_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // listen_fail: bind/start failed — cancel the listener, report a network
    // failure (mirrors net::listenTcp's bind error).
    ins.push(abi::label(&listen_fail));
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_listener_cancel",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    ins.push(abi::label(&net_fail));
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    // read_fail_fd: a seek/read on an opened PEM file failed — close it first.
    ins.push(abi::label(&read_fail_fd));
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FILEFD,
    ));
    platform.emit_close_file(symbol, platform_imports, &mut ins, &mut rel)?;
    ins.push(abi::label(&cert_fail));
    // Best-effort release of the PEM-import CoreFoundation objects still held
    // when the import fails (bug-236). Every slot is NULL-guarded and cleared, so
    // this is correct for an exit taken before a slot was filled and for one
    // taken after the success path already released it. `read_fail_fd` falls
    // through to here, where all four slots are still NULL — a no-op.
    //
    // Deliberately NOT emitted at `load_fail`: these releases resolve `CFRelease`
    // through `dlsym`, whose own failure branches to `load_fail` — emitting them
    // there would let that branch loop back into itself.
    for slot in [CERTREF, KEYREF, ITEMS, DATA] {
        emit_cf_release_slot(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            CFH,
            slot,
            FNPTR,
            &load_fail,
        )?;
    }
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(in crate::target::shared::code::tls) fn lower_tls_accept_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 208;
    const REC: usize = 8;
    const TIMEOUT: usize = 16;
    const NWH: usize = 24;
    const FNPTR: usize = 32;
    const LCTX: usize = 40;
    const QUEUE: usize = 48;
    const DEADLINE: usize = 56;
    const WAITFN: usize = 64;
    const CONN: usize = 72;
    const CCTX: usize = 80;
    const SBLOCK: usize = 96; // 96..136: per-connection state block literal

    let closed = format!("{symbol}_closed");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let wait_forever = format!("{symbol}_wait_forever");
    let deadline_ready = format!("{symbol}_deadline_ready");
    let wait_loop = format!("{symbol}_wait");
    let pop = format!("{symbol}_pop");
    let listener_dead = format!("{symbol}_listener_dead");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let hs_loop = format!("{symbol}_hs_wait");
    let hs_timeout = format!("{symbol}_hs_timeout");
    let conn_fail = format!("{symbol}_conn_fail");
    let ready = format!("{symbol}_ready");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // x0 = listener record { closed@0, listener@8, queue@16, lctx@24 };
    // x1 = timeoutMs.
    ins.extend([
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT),
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), REC_CTX),
        abi::store_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64("%v9", abi::return_register(), REC_QUEUE),
        abi::store_u64("%v9", abi::stack_pointer(), QUEUE),
    ]);
    emit_dlopen_maclib(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        &load_fail,
    )?;
    // Deadline: timeoutMs > 0 => dispatch_time(NOW, ms*1e6); else FOREVER.
    // The one absolute deadline bounds both the wait for a connection and the
    // server handshake.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&wait_forever),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_time",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // DISPATCH_TIME_NOW
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT),
        abi::move_immediate("%v10", "Integer", "1000000"),
        abi::multiply_registers(abi::ARG[1], abi::ARG[1], "%v10"), // ms -> ns
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_forever),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::bitwise_not("%v9", "%v9"), // DISPATCH_TIME_FOREVER
        abi::store_u64("%v9", abi::stack_pointer(), DEADLINE),
        abi::label(&deadline_ready),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), WAITFN),
        // Wait for a queued connection (the ring is checked first so
        // connections that arrived before this accept are drained even when
        // their semaphore counts were consumed by earlier state wakeups).
        abi::label(&wait_loop),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64("%v10", "%v9", LCTX_HEAD),
        abi::load_u64("%v11", "%v9", LCTX_TAIL),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_ne(&pop),
        // Listener failed/cancelled while we wait?
        abi::load_u32("%v10", "%v9", CTX_STATE),
        abi::compare_immediate("%v10", NW_LISTENER_FAILED),
        abi::branch_ge(&listener_dead),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DEADLINE),
        abi::load_u64("%v10", abi::stack_pointer(), WAITFN),
        abi::branch_link_register("%v10"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&accept_timeout),
        abi::branch(&wait_loop),
        // Pop the oldest queued connection.
        abi::label(&pop),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64("%v11", "%v9", LCTX_TAIL),
        abi::move_immediate("%v12", "Integer", "15"),
        abi::and_registers("%v12", "%v11", "%v12"),
        abi::shift_left_immediate("%v12", "%v12", 3),
        abi::add_immediate("%v13", "%v9", LCTX_RING),
        abi::add_registers("%v13", "%v13", "%v12"),
        abi::load_u64("%v14", "%v13", 0),
        abi::store_u64("%v14", abi::stack_pointer(), CONN),
        abi::add_immediate("%v11", "%v11", 1),
        abi::store_u64("%v11", "%v9", LCTX_TAIL),
    ]);
    // Per-connection block context { sem, signal, state, content, error }.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", CTX_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), CCTX));
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::store_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::store_u64(abi::ZERO, "%v9", CTX_STATE),
        abi::store_u64(abi::ZERO, "%v9", CTX_CONTENT),
        abi::store_u64(abi::ZERO, "%v9", CTX_ERROR),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_signal",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::store_u64("%v10", "%v9", CTX_SIGNAL),
    ]);
    // nw_connection_set_queue(conn, queue) — the listener's serial queue.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_connection_set_queue",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Per-connection state handler, then start (runs the server handshake).
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        STATE_INVOKE,
        CCTX,
        SBLOCK,
        FNPTR,
        &load_fail,
    )?;
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_connection_set_state_changed_handler",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), SBLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_connection_start",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        // Wait for the connection to reach ready (handshake complete).
        abi::label(&hs_loop),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DEADLINE),
        abi::load_u64("%v10", abi::stack_pointer(), WAITFN),
        abi::branch_link_register("%v10"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&hs_timeout),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::load_u32("%v10", "%v9", CTX_STATE),
        abi::compare_immediate("%v10", NW_STATE_READY),
        abi::branch_eq(&ready),
        abi::compare_immediate("%v10", "2"), // preparing
        abi::branch_eq(&hs_loop),
        abi::compare_immediate("%v10", "0"), // invalid
        abi::branch_eq(&hs_loop),
        abi::branch(&conn_fail), // waiting/failed/cancelled
        abi::label(&ready),
    ]);
    // Build the TlsSocket record { closed=0, conn, queue=0, cctx } — the queue
    // slot is 0 (not the listener's shared serial queue) so the shared close
    // helper releases the connection and ctx semaphore this socket owns but not
    // the listener-owned queue, which closeListener releases (bug-55). read/
    // write/close otherwise work identically to a client socket.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::ZERO, abi::RET[1], REC_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), CONN),
        abi::store_u64("%v9", abi::RET[1], REC_CONN),
        abi::store_u64(abi::ZERO, abi::RET[1], REC_QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::store_u64("%v9", abi::RET[1], REC_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // conn_fail / hs_timeout: cancel the accepted connection, then drop the
    // reference `accept` owns. The new-connection trampoline `nw_retain`s the
    // connection into the ring and the successful path releases it at close;
    // cancelling alone tears down network activity but keeps the +1, so before
    // bug-317 every handshake failure leaked one nw_connection — a remotely
    // triggerable, unbounded server-side leak for a server looping on accept.
    ins.push(abi::label(&conn_fail));
    emit_cancel_and_release_conn(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        CONN,
        FNPTR,
        &load_fail,
    )?;
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&hs_timeout));
    emit_cancel_and_release_conn(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        CONN,
        FNPTR,
        &load_fail,
    )?;
    emit_fail(
        symbol,
        ERR_TIMEOUT_CODE,
        ERR_TIMEOUT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&accept_timeout));
    emit_fail(
        symbol,
        ERR_TIMEOUT_CODE,
        ERR_TIMEOUT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&listener_dead));
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(in crate::target::shared::code::tls) fn lower_tls_close_listener_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 96;
    const REC: usize = 8;
    const HANDLE: usize = 16;
    const FNPTR: usize = 24;
    const LCTX: usize = 32;
    const CONN: usize = 40;
    const SETQFN: usize = 48;
    const CANCELFN: usize = 56;
    const RELEASEFN: usize = 64;

    let already = format!("{symbol}_already");
    let load_fail = format!("{symbol}_load_fail");
    let drain_loop = format!("{symbol}_drain");
    let drained = format!("{symbol}_drained");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        // Idempotent: a closed handle returns OK.
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&already),
        abi::load_u64("%v9", abi::return_register(), REC_CTX),
        abi::store_u64("%v9", abi::stack_pointer(), LCTX),
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
    for (name, off) in [
        ("nw_connection_set_queue", SETQFN),
        ("nw_connection_cancel", CANCELFN),
        ("nw_release", RELEASEFN),
    ] {
        dlsym(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            HANDLE,
            name,
            FNPTR,
            &load_fail,
        )?;
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
            abi::store_u64("%v9", abi::stack_pointer(), off),
        ]);
    }
    // Reject every still-queued (retained, never-started) connection: give it
    // the listener's queue, cancel it, drop our retain.
    ins.extend([
        abi::label(&drain_loop),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64("%v10", "%v9", LCTX_HEAD),
        abi::load_u64("%v11", "%v9", LCTX_TAIL),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_eq(&drained),
        abi::move_immediate("%v12", "Integer", "15"),
        abi::and_registers("%v12", "%v11", "%v12"),
        abi::shift_left_immediate("%v12", "%v12", 3),
        abi::add_immediate("%v13", "%v9", LCTX_RING),
        abi::add_registers("%v13", "%v13", "%v12"),
        abi::load_u64("%v14", "%v13", 0),
        abi::store_u64("%v14", abi::stack_pointer(), CONN),
        abi::add_immediate("%v11", "%v11", 1),
        abi::store_u64("%v11", "%v9", LCTX_TAIL),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::ARG[1], "%v9", REC_QUEUE),
        abi::load_u64("%v10", abi::stack_pointer(), SETQFN),
        abi::branch_link_register("%v10"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v10", abi::stack_pointer(), CANCELFN),
        abi::branch_link_register("%v10"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v10", abi::stack_pointer(), RELEASEFN),
        abi::branch_link_register("%v10"),
        abi::branch(&drain_loop),
        abi::label(&drained),
    ]);
    // nw_listener_cancel(listener)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_listener_cancel",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_CONN),
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v10"),
        // Release the listener, its serial queue, and the listener-ctx
        // semaphore this handle owns; cancelling alone leaks them (bug-55). The
        // arena-allocated lctx block is reclaimed with the arena. RELEASEFN
        // already holds nw_release (resolved in the drain loop above).
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_CONN),
        abi::load_u64("%v10", abi::stack_pointer(), RELEASEFN),
        abi::branch_link_register("%v10"),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        // NB: the listener ctx semaphore is intentionally NOT released here, for
        // the same reason as the connection close: nw_listener_cancel is async
        // and the listener state handler still signals ctx->sem on the cancelled
        // transition. It is reclaimed with the arena-allocated lctx block.
        // Mark closed.
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", REC_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}
