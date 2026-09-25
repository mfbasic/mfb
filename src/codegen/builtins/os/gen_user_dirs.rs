//! plan-156-C: the Linux `os::userDocumentsPath` base — `XDG_DOCUMENTS_DIR` from
//! `$XDG_CONFIG_HOME/user-dirs.dirs` (else `<home>/.config/user-dirs.dirs`),
//! parsed exactly as GLib's `load_user_special_dirs` does, so an MFB program and a
//! GTK application on the same desktop agree on the folder.
//!
//! The file is read in 4 KiB chunks into a frame buffer and scanned one byte at a
//! time by a state machine whose state survives the refill, so no file size is
//! capped. Per line (GLib's rules):
//!
//! 1. leading spaces/tabs are skipped;
//! 2. the literal `XDG_DOCUMENTS_DIR`, spaces/tabs, `=`, spaces/tabs, `"`;
//! 3. `$HOME` (then the value is home-relative) or `/` (absolute), else the line
//!    is ignored;
//! 4. the value runs to the LAST `"` on the line (GLib's `strrchr`), and one
//!    trailing `/` is dropped;
//! 5. a later valid line replaces an earlier one.
//!
//! A home-relative value becomes `home ++ ("/" unless it starts with one) ++
//! value` (GLib's `g_build_filename`); an absolute value is used as-is. The one
//! deliberate deviation: an absolute value that is empty after the trailing-`/`
//! drop (`"/"`) is ignored rather than yielding an empty path. A value longer than
//! [`VALUE_CAP`] bytes (`PATH_MAX`) is ignored — no usable path is that long.
//! Nothing here allocates from the arena.

use super::gen_host_paths::BaseBytes;
use super::gen_shared::emit_copy_counted;
use crate::codegen::builtins::fs::gen_open::open_flag_set;
use crate::codegen::engine::builder::EmitCtx;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::target::shared::abi;

/// Longest accepted `XDG_DOCUMENTS_DIR` value, and the size of the two value
/// buffers (the line being read, and the last valid one).
const VALUE_CAP: usize = 4096;
/// The read chunk.
const CHUNK_CAP: usize = 4096;
/// The path buffer: `user-dirs.dirs`'s path while it is opened, then the composed
/// `home/value` result.
const PATH_CAP: usize = 8192;

/// Frame layout, after the host-path name slot (`[0..32)`, reused here for the
/// key and `$HOME` literals once every `getenv` is done).
const KEY_OFF: usize = 0;
const DOLLAR_OFF: usize = 20;
const PATH_OFF: usize = 32;
const CHUNK_OFF: usize = PATH_OFF + PATH_CAP;
const WORK_OFF: usize = CHUNK_OFF + CHUNK_CAP;
const BEST_OFF: usize = WORK_OFF + VALUE_CAP;
/// Frame locals the Linux `userDocumentsPath` body needs.
pub(crate) const USER_DIRS_FRAME_LOCALS: usize = BEST_OFF + VALUE_CAP;

const KEY: &[u8] = b"XDG_DOCUMENTS_DIR";
const DOLLAR_HOME: &[u8] = b"$HOME";
const EINTR: &str = "4";

/// Parse `user-dirs.dirs` for the Documents folder (see the module doc). `home` is
/// the already-resolved home base (the caller holds the env/pwd lock, which also
/// keeps the `XDG_CONFIG_HOME` pointer valid). Emits code that branches to `found`
/// with the folder in the returned [`BaseBytes`], or to `fallback` when the file
/// is missing, unreadable, or has no valid line — the caller then uses
/// `<home>/Documents`.
pub(crate) fn emit_linux_user_dirs_documents(
    ctx: &mut EmitCtx,
    label_prefix: &str,
    home: &BaseBytes,
    found: &str,
    fallback: &str,
    vregs: &mut Vregs,
) -> Result<BaseBytes, String> {
    let lp = |name: &str| format!("{label_prefix}_ud_{name}");
    let doc = BaseBytes {
        ptr: vregs.next(),
        len: vregs.next(),
    };
    let path = vregs.next();
    let out = vregs.next();
    let copy_src = vregs.next();
    let copy_index = vregs.next();
    let byte = vregs.next();

    // --- The file's path: <config>/user-dirs.dirs\0 into the path buffer. ---
    let cfg_missing = lp("cfg_missing");
    let cfg_ready = lp("cfg_ready");
    ctx.instructions
        .push(abi::add_immediate(&path, abi::stack_pointer(), PATH_OFF));
    let cfg = super::gen_host_paths::emit_posix_env_abs_base(
        ctx,
        &lp("cfg"),
        "XDG_CONFIG_HOME",
        &cfg_missing,
        vregs,
    )?;
    // Room for the longest tail ("/.config" + "/user-dirs.dirs" + NUL = 24).
    let room = (PATH_CAP - 24).to_string();
    ctx.instructions.extend([
        abi::compare_immediate(&cfg.len, &room),
        abi::branch_gt(fallback),
        abi::move_register(&out, &path),
    ]);
    emit_copy_counted(
        &cfg.ptr,
        &cfg.len,
        &out,
        &copy_src,
        &copy_index,
        &byte,
        &lp("copy_cfg"),
        ctx.instructions,
    );
    ctx.instructions.extend([
        abi::branch(&cfg_ready),
        abi::label(&cfg_missing),
        abi::compare_immediate(&home.len, &room),
        abi::branch_gt(fallback),
        abi::move_register(&out, &path),
    ]);
    emit_copy_counted(
        &home.ptr,
        &home.len,
        &out,
        &copy_src,
        &copy_index,
        &byte,
        &lp("copy_home_cfg"),
        ctx.instructions,
    );
    store_bytes(b"/.config", &out, &byte, ctx.instructions);
    ctx.instructions.push(abi::label(&cfg_ready));
    store_bytes(b"/user-dirs.dirs\0", &out, &byte, ctx.instructions);

    // The key and `$HOME` literals, matched byte by byte from the frame.
    for (i, b) in KEY.iter().enumerate() {
        ctx.instructions.extend([
            abi::move_immediate(&byte, "Byte", &b.to_string()),
            abi::store_u8(&byte, abi::stack_pointer(), KEY_OFF + i),
        ]);
    }
    for (i, b) in DOLLAR_HOME.iter().enumerate() {
        ctx.instructions.extend([
            abi::move_immediate(&byte, "Byte", &b.to_string()),
            abi::store_u8(&byte, abi::stack_pointer(), DOLLAR_OFF + i),
        ]);
    }

    // --- open(path, O_RDONLY | O_CLOEXEC). ---
    let fd = vregs.next();
    let flags = open_flag_set(PlatformFamily::Linux, false);
    ctx.instructions.extend([
        abi::move_register(abi::return_register(), &path),
        abi::move_immediate(abi::c_arg(1), "Integer", flags.read),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    ctx.platform.emit_open_file(
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        // C `int` fd — sign-extend before the signed compare (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(fallback),
        abi::move_register(&fd, abi::return_register()),
    ]);

    // --- The scan. ---
    let state = vregs.next();
    let idx = vregs.next();
    let work_len = vregs.next();
    let last_quote = vregs.next();
    let relative = vregs.next();
    let best_found = vregs.next();
    let best_len = vregs.next();
    let best_rel = vregs.next();
    let n = vregs.next();
    let pos = vregs.next();
    let expect = vregs.next();
    let at = vregs.next();
    let errno = vregs.next();
    let chunk = vregs.next();
    let work = vregs.next();
    let best = vregs.next();
    let refill = lp("refill");
    let read_neg = lp("read_neg");
    let next_byte = lp("next");
    let eof = lp("eof");
    let st = |k: u8| lp(&format!("st{k}"));
    ctx.instructions.extend([
        abi::move_immediate(&state, "Integer", "0"),
        abi::move_immediate(&idx, "Integer", "0"),
        abi::move_immediate(&work_len, "Integer", "0"),
        abi::move_immediate(&last_quote, "Integer", "0"),
        abi::subtract_immediate(&last_quote, &last_quote, 1),
        abi::move_immediate(&relative, "Integer", "0"),
        abi::move_immediate(&best_found, "Integer", "0"),
        abi::move_immediate(&best_len, "Integer", "0"),
        abi::move_immediate(&best_rel, "Integer", "0"),
        abi::move_immediate(&n, "Integer", "0"),
        abi::move_immediate(&pos, "Integer", "0"),
        abi::label(&refill),
        abi::move_register(abi::return_register(), &fd),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), CHUNK_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", &CHUNK_CAP.to_string()),
    ]);
    ctx.platform.emit_read_file(
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::move_register(&n, abi::return_register()),
        abi::compare_immediate(&n, "0"),
        abi::branch_eq(&eof),
        abi::branch_lt(&read_neg),
        abi::move_immediate(&pos, "Integer", "0"),
        abi::branch(&next_byte),
        abi::label(&read_neg),
    ]);
    // A failed read retries on EINTR; anything else ends the file here (the lines
    // already read still count, exactly as a short file would).
    ctx.platform.emit_errno(
        ctx.symbol,
        (&errno).into(),
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(&errno, EINTR),
        abi::branch_eq(&refill),
        abi::branch(&eof),
        // Next byte: refill when the chunk is spent.
        abi::label(&next_byte),
        abi::compare_registers(&pos, &n),
        abi::branch_ge(&refill),
        abi::add_immediate(&chunk, abi::stack_pointer(), CHUNK_OFF),
        abi::add_registers(&at, &chunk, &pos),
        abi::load_u8(&byte, &at, 0),
        abi::add_immediate(&pos, &pos, 1),
        abi::compare_immediate(&state, "0"),
        abi::branch_eq(&st(0)),
        abi::compare_immediate(&state, "1"),
        abi::branch_eq(&st(1)),
        abi::compare_immediate(&state, "2"),
        abi::branch_eq(&st(2)),
        abi::compare_immediate(&state, "3"),
        abi::branch_eq(&st(3)),
        abi::compare_immediate(&state, "4"),
        abi::branch_eq(&st(4)),
        abi::compare_immediate(&state, "5"),
        abi::branch_eq(&st(5)),
        abi::compare_immediate(&state, "6"),
        abi::branch_eq(&st(6)),
        abi::branch(&st(9)),
    ]);
    let skip_ws = |target: &str, instructions: &mut Vec<CodeInstruction>| {
        instructions.extend([
            abi::compare_immediate(&byte, "32"), // ' '
            abi::branch_eq(target),
            abi::compare_immediate(&byte, "9"), // '\t'
            abi::branch_eq(target),
        ]);
    };
    let to_skip = lp("to_skip");
    // State 0: a line starts. Reset the per-line value, skip blanks and empty lines.
    ctx.instructions.extend([
        abi::label(&st(0)),
        abi::move_immediate(&work_len, "Integer", "0"),
        abi::move_immediate(&last_quote, "Integer", "0"),
        abi::subtract_immediate(&last_quote, &last_quote, 1),
        abi::move_immediate(&relative, "Integer", "0"),
    ]);
    skip_ws(&next_byte, ctx.instructions);
    ctx.instructions.extend([
        abi::compare_immediate(&byte, "10"), // '\n'
        abi::branch_eq(&next_byte),
        abi::move_immediate(&state, "Integer", "1"),
        abi::move_immediate(&idx, "Integer", "0"),
        // State 1: the key, one byte at a time.
        abi::label(&st(1)),
        abi::add_immediate(&at, abi::stack_pointer(), KEY_OFF),
        abi::add_registers(&at, &at, &idx),
        abi::load_u8(&expect, &at, 0),
        abi::compare_registers(&byte, &expect),
        abi::branch_ne(&to_skip),
        abi::add_immediate(&idx, &idx, 1),
        abi::compare_immediate(&idx, &KEY.len().to_string()),
        abi::branch_ne(&next_byte),
        abi::move_immediate(&state, "Integer", "2"),
        abi::branch(&next_byte),
        // State 2: blanks, then `=`.
        abi::label(&st(2)),
    ]);
    skip_ws(&next_byte, ctx.instructions);
    ctx.instructions.extend([
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_ne(&to_skip),
        abi::move_immediate(&state, "Integer", "3"),
        abi::branch(&next_byte),
        // State 3: blanks, then the opening `"`.
        abi::label(&st(3)),
    ]);
    skip_ws(&next_byte, ctx.instructions);
    let not_dollar = lp("not_dollar");
    let dollar_done = lp("dollar_done");
    let quote_seen = lp("quote_seen");
    let append = lp("append");
    ctx.instructions.extend([
        abi::compare_immediate(&byte, "34"), // '"'
        abi::branch_ne(&to_skip),
        abi::move_immediate(&state, "Integer", "4"),
        abi::branch(&next_byte),
        // State 4: `$HOME` (home-relative) or `/` (absolute); anything else voids
        // the line.
        abi::label(&st(4)),
        abi::compare_immediate(&byte, "36"), // '$'
        abi::branch_ne(&not_dollar),
        abi::move_immediate(&state, "Integer", "5"),
        abi::move_immediate(&idx, "Integer", "1"),
        abi::branch(&next_byte),
        abi::label(&not_dollar),
        abi::compare_immediate(&byte, "47"), // '/'
        abi::branch_ne(&to_skip),
        abi::move_immediate(&relative, "Integer", "0"),
        abi::move_immediate(&state, "Integer", "6"),
        abi::branch(&append),
        // State 5: the rest of `$HOME`.
        abi::label(&st(5)),
        abi::add_immediate(&at, abi::stack_pointer(), DOLLAR_OFF),
        abi::add_registers(&at, &at, &idx),
        abi::load_u8(&expect, &at, 0),
        abi::compare_registers(&byte, &expect),
        abi::branch_ne(&to_skip),
        abi::add_immediate(&idx, &idx, 1),
        abi::compare_immediate(&idx, &DOLLAR_HOME.len().to_string()),
        abi::branch_ne(&next_byte),
        abi::label(&dollar_done),
        abi::move_immediate(&relative, "Integer", "1"),
        abi::move_immediate(&state, "Integer", "6"),
        abi::branch(&next_byte),
        // State 6: the value, to the end of the line.
        abi::label(&st(6)),
    ]);
    let line_end = lp("line_end");
    ctx.instructions.extend([
        abi::compare_immediate(&byte, "10"),
        abi::branch_eq(&line_end),
        abi::label(&append),
        abi::compare_immediate(&work_len, &VALUE_CAP.to_string()),
        abi::branch_ge(&to_skip), // too long for a path: void the line
        abi::compare_immediate(&byte, "34"),
        abi::branch_ne(&quote_seen),
        abi::move_register(&last_quote, &work_len),
        abi::label(&quote_seen),
        abi::add_immediate(&work, abi::stack_pointer(), WORK_OFF),
        abi::add_registers(&at, &work, &work_len),
        abi::store_u8(&byte, &at, 0),
        abi::add_immediate(&work_len, &work_len, 1),
        abi::branch(&next_byte),
        // State 9: the rest of a void line.
        abi::label(&to_skip),
        abi::move_immediate(&state, "Integer", "9"),
        abi::label(&st(9)),
        abi::compare_immediate(&byte, "10"),
        abi::branch_ne(&next_byte),
        abi::move_immediate(&state, "Integer", "0"),
        abi::branch(&next_byte),
    ]);
    // End of a value line — reached from a `\n` (then back to state 0) and from EOF.
    let mut end_line = |tag: &str, after: &str, instructions: &mut Vec<CodeInstruction>| {
        let void = lp(&format!("{tag}_void"));
        let kept = lp(&format!("{tag}_kept"));
        let no_slash = lp(&format!("{tag}_no_slash"));
        let vlen = vregs.next();
        instructions.extend([
            abi::compare_immediate(&last_quote, "0"),
            abi::branch_lt(&void),
            abi::move_register(&vlen, &last_quote),
            // Drop ONE trailing `/`.
            abi::compare_immediate(&vlen, "0"),
            abi::branch_eq(&no_slash),
            abi::add_immediate(&work, abi::stack_pointer(), WORK_OFF),
            abi::add_registers(&at, &work, &vlen),
            abi::subtract_immediate(&at, &at, 1),
            abi::load_u8(&expect, &at, 0),
            abi::compare_immediate(&expect, "47"),
            abi::branch_ne(&no_slash),
            abi::subtract_immediate(&vlen, &vlen, 1),
            abi::label(&no_slash),
            // An absolute value that is now empty names no folder.
            abi::compare_immediate(&relative, "0"),
            abi::branch_ne(&kept),
            abi::compare_immediate(&vlen, "0"),
            abi::branch_eq(&void),
            abi::label(&kept),
            abi::add_immediate(&work, abi::stack_pointer(), WORK_OFF),
            abi::add_immediate(&best, abi::stack_pointer(), BEST_OFF),
        ]);
        emit_copy_counted(
            &work,
            &vlen,
            &best,
            &copy_src,
            &copy_index,
            &byte,
            &lp(&format!("{tag}_keep")),
            instructions,
        );
        instructions.extend([
            abi::move_register(&best_len, &vlen),
            abi::move_register(&best_rel, &relative),
            abi::move_immediate(&best_found, "Integer", "1"),
            abi::label(&void),
            abi::move_immediate(&state, "Integer", "0"),
            abi::branch(after),
        ]);
    };
    ctx.instructions.push(abi::label(&line_end));
    end_line("nl", &next_byte, ctx.instructions);
    let closed = lp("closed");
    let close = lp("close");
    ctx.instructions.extend([
        abi::label(&eof),
        abi::compare_immediate(&state, "6"),
        abi::branch_ne(&close),
    ]);
    end_line("eof", &close, ctx.instructions);
    ctx.instructions.extend([
        abi::label(&close),
        abi::move_register(abi::return_register(), &fd),
    ]);
    ctx.platform.emit_close_file(
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    // --- The result. ---
    let relative_result = lp("relative_result");
    let no_join_slash = lp("no_join_slash");
    ctx.instructions.extend([
        abi::label(&closed),
        abi::compare_immediate(&best_found, "0"),
        abi::branch_eq(fallback),
        abi::add_immediate(&best, abi::stack_pointer(), BEST_OFF),
        abi::compare_immediate(&best_rel, "0"),
        abi::branch_ne(&relative_result),
        abi::move_register(&doc.ptr, &best),
        abi::move_register(&doc.len, &best_len),
        abi::branch(found),
        // home ++ ("/" unless the value starts with one) ++ value, in the path
        // buffer (the file is closed; its path is no longer needed).
        abi::label(&relative_result),
        abi::add_registers(&at, &home.len, &best_len),
        abi::compare_immediate(&at, &(PATH_CAP - 1).to_string()),
        abi::branch_gt(fallback),
        abi::add_immediate(&path, abi::stack_pointer(), PATH_OFF),
        abi::move_register(&out, &path),
    ]);
    emit_copy_counted(
        &home.ptr,
        &home.len,
        &out,
        &copy_src,
        &copy_index,
        &byte,
        &lp("compose_home"),
        ctx.instructions,
    );
    ctx.instructions.extend([
        abi::compare_immediate(&best_len, "0"),
        abi::branch_eq(&no_join_slash),
        abi::add_immediate(&best, abi::stack_pointer(), BEST_OFF),
        abi::load_u8(&expect, &best, 0),
        abi::compare_immediate(&expect, "47"),
        abi::branch_eq(&no_join_slash),
        abi::move_immediate(&byte, "Byte", "47"),
        abi::store_u8(&byte, &out, 0),
        abi::add_immediate(&out, &out, 1),
        abi::label(&no_join_slash),
        abi::add_immediate(&best, abi::stack_pointer(), BEST_OFF),
    ]);
    emit_copy_counted(
        &best,
        &best_len,
        &out,
        &copy_src,
        &copy_index,
        &byte,
        &lp("compose_value"),
        ctx.instructions,
    );
    ctx.instructions.extend([
        abi::add_immediate(&path, abi::stack_pointer(), PATH_OFF),
        abi::move_register(&doc.ptr, &path),
        abi::subtract_registers(&doc.len, &out, &path),
        abi::branch(found),
    ]);
    Ok(doc)
}

/// Store `bytes` at `out`, advancing it.
fn store_bytes(bytes: &[u8], out: &str, scratch: &str, instructions: &mut Vec<CodeInstruction>) {
    for &b in bytes {
        super::gen_shared::emit_store_byte_advance(b, out, scratch, instructions);
    }
}
