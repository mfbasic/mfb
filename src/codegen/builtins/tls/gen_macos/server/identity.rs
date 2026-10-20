//! plan-176-T: the macOS listener identity helpers — read a PEM file, import a PEM item,
//! release a CF slot. The shared imports live in `builtins/tls/gen_macos/server/mod.rs`.

use super::*;

#[allow(clippy::too_many_arguments)]
/// Read the whole file named by the MFBASIC `String` at `sp + path_off` into a
/// fresh arena buffer: pointer at `sp + buf_off`, byte length at
/// `sp + len_off`. `open_fail` is taken when the file cannot be opened (no fd
/// yet); `read_fail_fd` when a seek/read fails or the file is empty (the open
/// fd is at `sp + fd_off` for the caller to close).
///
/// bug-575: `path_scratch` is the marshalled path C-string this emitter allocates
/// for `open`. It is declared by the CALLER rather than here because the caller
/// owns the `done` the release has to sit at — this sub-emitter has no `ret` of
/// its own, and each of its failure exits (`open_fail`, `read_fail_fd`,
/// `alloc_fail`) lands in the caller's tail. The caller also calls it TWICE, for
/// the certificate and for the key, through the SAME `cstr_off` slot: one scratch
/// per call, or the first block would be overwritten and lost.
///
/// bug-984: `buf_scratch` is the file's contents, declared by the caller for the
/// same reason and released at the same `done`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_read_whole_file(
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
    path_scratch: &HelperScratch,
    buf_scratch: &HelperScratch,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
    let v10 = vregs.next();
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
        path_scratch,
        ctx.instructions,
        ctx.relocations,
        vregs,
    );
    // fd = open(path, O_RDONLY | O_CLOEXEC) — Darwin O_CLOEXEC = 0x1000000; the PEM
    // fd must not cross into a concurrent `process::spawn` child (bug-499).
    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), cstr_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "16777216"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "2"),
    ]);
    platform.emit_seek_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(read_fail_fd),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), len_off),
        // rewind: lseek(fd, 0, SEEK_SET)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_seek_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    // buf = arena_alloc(len, 1)
    //
    // bug-984: the buffer is scratch. `CFDataCreate` copies the bytes it is handed,
    // so nothing reads this block once the item is imported, and nothing freed it:
    // every `tls::listen` left both PEM files' bytes — the private key among them —
    // live in the arena. `buf_scratch` records the block and the byte count it was
    // allocated with, for the caller's single release at its `done`, which every
    // exit reaches (a short read and a failed import included).
    ctx.instructions.extend([
        abi::load_u64(buf_scratch.size, abi::stack_pointer(), len_off),
        abi::move_register(abi::return_register(), buf_scratch.size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ctx.instructions, ctx.relocations, alloc_fail);
    ctx.instructions.extend([
        abi::move_register(buf_scratch.pointer, abi::mfb_return(1)),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), buf_off),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), readoff_off),
        abi::label(&read_loop),
        abi::load_u64(&v9, abi::stack_pointer(), readoff_off),
        abi::load_u64(&v10, abi::stack_pointer(), len_off),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&read_done),
        // n = read(fd, buf + off, len - off)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), buf_off),
        abi::add_registers(abi::c_arg(1), abi::c_arg(1), &v9),
        abi::subtract_registers(abi::c_arg(2), &v10, &v9),
    ]);
    platform.emit_read_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(read_fail_fd),
        abi::load_u64(&v9, abi::stack_pointer(), readoff_off),
        abi::add_registers(&v9, &v9, abi::return_register()),
        abi::store_u64(&v9, abi::stack_pointer(), readoff_off),
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
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_import_pem_item(
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
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
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
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), buf_off),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), len_off),
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::move_immediate(abi::c_arg(4), "Integer", "0"),
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
        abi::add_immediate(abi::c_arg(7), abi::stack_pointer(), items_off),
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(fail),
        abi::load_u64(&v9, abi::stack_pointer(), items_off),
        abi::compare_immediate(&v9, "0"),
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
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
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
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
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
        vregs,
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
        vregs,
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
pub(super) fn emit_cf_release_slot(
    ctx: &mut EmitCtx,
    cf_handle_off: usize,
    slot_off: usize,
    fnptr_off: usize,
    load_fail: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // Unique per emission point, not per slot: a slot is released from several
    // sites in one function (once per PEM import, again on the error exit), so a
    // slot-keyed label would collide.
    let skip = format!("{symbol}_cf_rel_skip_{slot_off}_{}", ctx.instructions.len());
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), slot_off),
        abi::compare_immediate(&v9, "0"),
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
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), slot_off),
        abi::label(&skip),
    ]);
    Ok(())
}
