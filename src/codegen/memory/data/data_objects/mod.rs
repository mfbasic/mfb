// --- codegen tier imports (migration) ---
use crate::codegen::builtins;
use crate::codegen::engine::analysis::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::abi;
use crate::target::shared::nir;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
use std::collections::HashMap;
use std::collections::HashSet;

mod static_names;
mod string_values;
mod unicode_tables;
pub(crate) use static_names::{
    builtin_function_refs, builtin_function_symbol_for_type, collect_function_ref_names,
    static_type_name_for_fold_with_types, static_type_name_with_types,
};
pub(crate) use string_values::static_string_value_with_constants;
use string_values::*;
pub(crate) use unicode_tables::{unicode_runtime_data_objects, unicode_string_call_is_static};
/// Materialize the address of an internal symbol (data or code) into `dst` via
/// the `adrp`/`add` page pair. The `data` binding is the internal-symbol-address
/// relocation regardless of the target's section — the linker resolves it through
/// `symbol_vmaddr` (the same pattern used for the thread-trampoline address).
pub(crate) fn push_symbol_address(
    from: &str,
    symbol: &str,
    // plan-85-B: accept a typed `Operand` (`abi::c_arg(0)`) or a legacy `&str`.
    dst: impl Into<Operand>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let dst = dst.into();
    instructions.push(abi::load_page_address(&dst, symbol));
    instructions.push(abi::add_page_offset(&dst, &dst, symbol));
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

pub(crate) fn push_error_message_address(
    from: &str,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

/// Emit a runtime error from a **fixed runtime helper** (a free-function codegen
/// builder with no `CodeBuilder`/`self` in scope) — the free-function companion to
/// [`CodeBuilder::raise_error`]/[`raise_error_bare`] (plan-88-C). It reproduces the
/// historical lightweight fixed-helper error sequence exactly: set the error code
/// immediate, set the error tag, and load the message data-object address — but
/// sources the `(code, message-symbol)` from `ERRORCODE_CONSTANTS` instead of the
/// per-error `ERR_*_CODE`/`ERR_*_SYMBOL` codegen constants. This is why the two
/// emission worlds (per-call-site methods, fixed-helper free functions) now share
/// one metadata authority while keeping their distinct instruction shapes.
///
/// `error_name` must be a known `errorCode` constant (e.g. `"ErrEndOfFile"`);
/// unknown names are a codegen bug and panic. `from` is the emitting function's
/// symbol (the relocation origin), matching `push_error_message_address`.
pub(crate) fn raise_error_into(
    from: &str,
    error_name: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let (code, symbol) = crate::codegen::registry::runtime_error_emission(error_name)
        .unwrap_or_else(|| panic!("raise_error_into: `{error_name}` is not an errorCode constant"));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_ERR_TAG,
    ));
    push_error_message_address(from, symbol, instructions, relocations);
}

/// plan-158-A: the data-object symbol of the message prefix
/// `"<registered message> (<call>: "` that `_mfb_rt_fail_status` completes with a
/// status. The symbol carries both halves (`_mfb_str_errstatus_<ErrorName>_<hex>`,
/// `<hex>` the call name's bytes in lowercase hex) so the post-codegen relocation
/// scan can rebuild the text from the symbol alone
/// ([`decode_error_status_prefix_symbol`]), the same way the fixed
/// `_mfb_str_error_*` messages are emitted only when referenced.
///
/// `error_name` must be a known `errorCode` constant; an unknown name panics, as in
/// [`raise_error_into`].
pub(crate) fn error_status_prefix_symbol(error_name: &str, call: &str) -> String {
    assert!(
        crate::codegen::registry::runtime_error(error_name).is_some(),
        "error_status_prefix_symbol: `{error_name}` is not an errorCode constant"
    );
    let hex: String = call.bytes().map(|byte| format!("{byte:02x}")).collect();
    format!("{ERROR_STATUS_PREFIX_SYMBOL_PREFIX}{error_name}_{hex}")
}

/// Invert [`error_status_prefix_symbol`]: the prefix text
/// `"<registered message> (<call>: "` for a well-formed symbol naming a known
/// error, `None` for anything else (only the canonical lowercase-hex spelling is
/// accepted, so one text has exactly one symbol).
pub(crate) fn decode_error_status_prefix_symbol(symbol: &str) -> Option<String> {
    let rest = symbol.strip_prefix(ERROR_STATUS_PREFIX_SYMBOL_PREFIX)?;
    // Error names are `Err` + CamelCase, with no `_`, so the first `_` ends it.
    let (error_name, hex) = rest.split_once('_')?;
    let (_, message) = crate::codegen::registry::runtime_error(error_name)?;
    let canonical = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.bytes().all(canonical) {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let call = String::from_utf8(bytes).ok()?;
    Some(format!("{message} ({call}: "))
}

/// The registered runtime-error message for an `errorCode` name (plan-88-D). The
/// data-object gating below emits each fixed `_mfb_str_error_*` string; it now
/// sources every message from `ERRORCODE_CONSTANTS` (the single metadata authority)
/// instead of the deleted per-error `ERR_*_MESSAGE` codegen constants.
fn err_msg(name: &str) -> String {
    crate::codegen::registry::runtime_error(name)
        .unwrap_or_else(|| panic!("err_msg: `{name}` is not an errorCode constant"))
        .1
        .to_string()
}

pub(crate) fn string_symbols(
    module: &NirModule,
    type_model: &TypeModel,
) -> HashMap<String, String> {
    let mut values = Vec::new();
    // The module's record / union-variant field types, so every walk below can
    // type a `MemberAccess` (bug-363, bug-366), and the builder's own enum table
    // (plan-140-B), so a walk tells an enum from any other declared type exactly
    // as the lowering will.
    let fields = module_field_types(module)
        .with_enums_of(type_model)
        .with_records_of(type_model)
        .with_function_returns(module);
    if module_uses_type_name(module) {
        collect_type_name_values(module, &mut values);
    }
    for function in &module.functions {
        collect_string_values_from_function(function, &mut values, &fields);
    }
    // Source file paths back `ErrorLoc.filename` for errors that originate in
    // each function; emit them as string constants so the origin can load them.
    for function in &module.functions {
        if !function.file.is_empty() {
            push_string_value(&mut values, function.file.clone());
        }
    }
    for value in [
        err_msg("ErrInvalidArgument"),
        err_msg("ErrOverflow"),
        err_msg("ErrUnderflow"),
        err_msg("ErrOutOfMemory"),
    ] {
        push_string_value(&mut values, value);
    }
    // bug-789: every function that can recurse raises `ErrStackOverflow` at entry.
    if module
        .functions
        .iter()
        .any(crate::codegen::runtime::stack_limit::function_needs_stack_check)
    {
        push_string_value(&mut values, err_msg("ErrStackOverflow"));
    }
    if module_may_emit_float_numeric_error(module) {
        for value in [
            err_msg("ErrFloatDomain"),
            err_msg("ErrFloatNaN"),
            err_msg("ErrFloatInf"),
            err_msg("ErrFloatOverflow"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    if module_uses_any_call(
        module,
        &[
            "io.print",
            "io.write",
            "io.printError",
            "io.writeError",
            "io.flush",
        ],
    ) {
        push_string_value(&mut values, err_msg("ErrWriteFailed"));
    }
    if module_uses_any_call(
        module,
        &[
            "io.input",
            "io.readLine",
            "io.readRaw",
            "io.readScalar",
            "io.readGrapheme",
            "io.readKey",
        ],
    ) {
        if module_uses_call(module, "io.input") {
            push_string_value(&mut values, String::new());
        }
        push_string_value(&mut values, err_msg("ErrEndOfFile"));
        push_string_value(&mut values, err_msg("ErrInputFailed"));
        push_string_value(&mut values, err_msg("ErrEncoding"));
        // plan-15 D1: reading stdin from an unsubscribed thread traps ErrInvalidContext.
        push_string_value(&mut values, err_msg("ErrInvalidContext"));
        // bug-256 class (found during plan-62-E): every console-read helper begins by
        // draining pending stdout (`STDOUT_DRAIN_SYMBOL`), which raises `ErrOutput`
        // on a write failure — so its `_mfb_str_error_output` data object must exist
        // whenever a read helper is emitted, even if the program never calls
        // `io::print`/`io::write`. Without this, an `io::readLine`-only program (any
        // build) failed to link with a dangling `_mfb_str_error_output` relocation.
        push_string_value(&mut values, err_msg("ErrWriteFailed"));
        // bug-699: the timeout convention (`timeoutMs` on every reader) and the
        // growable line / key record.
        push_string_value(&mut values, err_msg("ErrTimeout"));
        push_string_value(&mut values, err_msg("ErrInvalidArgument"));
        push_string_value(&mut values, err_msg("ErrOutOfMemory"));
    }
    // plan-62-E: in an app build that uses `app::` (so a non-`Console` presentation
    // mode is reachable), every `term::` and console-read `io::` helper carries an
    // `ErrWrongMode` gate. Registering the message emits its
    // `_mfb_str_error_wrong_mode` data object so the gate's relocation resolves
    // (the bug-256 class). Over-approximated to any `app::`-using app build; an
    // unreferenced pooled string is harmless dead data.
    if module.build_mode.is_app() && module_uses_any_call(module, &["app.getMode", "app.setMode"]) {
        push_string_value(&mut values, err_msg("ErrWrongMode"));
    }
    // plan-98-B: the `canvas::` surface members carry the same `ErrWrongMode` gate
    // (they require `Mode.Canvas`), and `canvas::present` deep-copies the scene into
    // the arena, so it can raise `ErrOutOfMemory`. A canvas program need not
    // reference `app::` by name to reach either — it always does in practice, since
    // it must `setMode` to get into canvas mode, but keying on the canvas calls
    // themselves makes the gate's relocation resolve from its own cause.
    if module_uses_any_call(
        module,
        &[
            "canvas.present",
            "canvas.setGroup",
            "canvas.groupCount",
            "canvas.groupBytes",
            "canvas.groupResolve",
            "canvas.groupRevision",
            "canvas.groupItems",
            "canvas.groupReclaim",
            "canvas.nextReclaimableGroup",
            "canvas.retiredItems",
            "canvas.groupSlots",
            "canvas.removeGroup",
            "canvas.presentLayers",
            "canvas.publishScene",
            "canvas.publishLayers",
            "canvas.blitSurface",
            "canvas.frameDone",
            "canvas.syncFrame",
            "canvas.setSyncMode",
            "canvas.surfaceWidth",
            "canvas.surfaceHeight",
            "canvas.setGpuMode",
            "canvas.useGpu",
            "canvas.metalAvailable",
            "canvas.vulkanReady",
            "canvas.vulkanDrawScene",
            "canvas.metalReady",
            "canvas.metalDrawScene",
            "canvas.startGraphics",
            "canvas.signalRedraw",
            "canvas.waitForRedraw",
            "canvas.newSurface",
            "canvas.installedItems",
            "canvas.installedLayers",
            "canvas.publishHashes",
            "canvas.installedHashes",
        ],
    ) {
        for value in [err_msg("ErrWrongMode"), err_msg("ErrOutOfMemory")] {
            push_string_value(&mut values, value);
        }
    }
    // The image members allocate (so they can raise `ErrOutOfMemory`), guard the
    // resource's closed flag, and validate the RGBA8 pixel count. Keyed on the
    // members themselves rather than on `canvas.present` so a program that only
    // manipulates image contents — which needs no scene at all — still gets them.
    if module_uses_any_call(
        module,
        &[
            "canvas.createImage",
            "canvas.loadImage",
            "canvas.imageHandle",
            "canvas.getSize",
            "canvas.getBytes",
            "canvas.setBytes",
            "canvas.setBytesRaw",
        ],
    ) {
        for value in [
            err_msg("ErrOutOfMemory"),
            err_msg("ErrResourceClosed"),
            err_msg("ErrBadPixelCount"),
            err_msg("ErrBadImageFile"),
            err_msg("ErrWrongMode"),
            // plan-116-G: `canvas::setGroup` is in the gate list above, and it is the
            // only member that can raise this. Registering the string is what emits
            // its data object — without the row the relocation the raise already
            // emitted dangles at link time (the bug-256 class).
            err_msg("ErrCanvasGroupLimit"),
            // plan-116-G: `canvas::present` raises this when a group's nesting exceeds
            // 64 levels, which is also how a cycle reports.
            err_msg("ErrDepthExceeded"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    // The font members, keyed the same way and for the same reason: a program that
    // loads a font and measures text needs these whether or not it ever presents a
    // scene. `ErrBadFontFile` rides with them because `loadFont` is the only member
    // that can raise it.
    if module_uses_any_call(
        module,
        &[
            "canvas.loadFont",
            "canvas.fontFromBytes",
            "canvas.fontHandle",
        ],
    ) {
        for value in [
            err_msg("ErrOutOfMemory"),
            err_msg("ErrResourceClosed"),
            err_msg("ErrBadFontFile"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    if module_uses_any_call(
        module,
        &[
            "thread.isRunning",
            "thread.waitFor",
            "thread.cancel",
            "thread.send",
            "thread.poll",
            "thread.receive",
            "thread.read",
        ],
    ) {
        push_string_value(&mut values, err_msg("ErrResourceClosed"));
        // `ErrResourceMoved` rides the SAME closed-guard as `ErrResourceClosed`
        // (both bits live in the offset-16 word, and the guard splits them only at
        // the report), so wherever the closed message is registered the moved one
        // must be too — plan-52-B. Registering the string is what emits its
        // `_mfb_str_error_resource_moved` data object; miss one and the reference
        // the guard already emitted dangles at link time (the bug-256 class:
        // `net::` programs link no `_mfb_rt_fs_*`/`_mfb_rt_thread_*` symbol, so
        // they do not get the whole standard set for free and failed with
        // "relocation target '_mfb_str_error_resource_moved' is not a data object").
        push_string_value(&mut values, err_msg("ErrResourceMoved"));
    }
    if module_uses_call(module, "fs.currentDirectory") {
        push_string_value(&mut values, err_msg("ErrReadFailed"));
    }
    // `os::getEnv` raises `ErrNotFound` for an unset variable; `os::setEnv`
    // reuses the always-emitted `ErrInvalidArgument`/allocation messages
    // (plan-31-A).
    if module_uses_call(module, "os.getEnv") {
        push_string_value(&mut values, err_msg("ErrNotFound"));
    }
    // plan-99: the worker branch of `os::sleep` returns `ErrInterrupted` on
    // cancellation, so its body always emits that message's relocation. A program
    // that only sleeps links no `_mfb_rt_thread_*`/`_mfb_rt_fs_*` symbol, so it does
    // NOT get the standard error-message set for free — register the message here or
    // the reference dangles at link time (the bug-256 class). `ErrInvalidArgument`
    // (a negative `ms`) is in the always-emitted set above.
    if module_uses_call(module, "os.sleep") {
        push_string_value(&mut values, err_msg("ErrInterrupted"));
    }
    // `os::hostName`/`userName`/`executablePath` raise ErrUnsupported when the
    // host lookup fails (no passwd entry, unreadable /proc/self/exe, …).
    if module_uses_any_call(
        module,
        &[
            "os.hostName",
            "os.userName",
            "os.executablePath",
            "os.version",
            "os.uptime",
            // plan-55-B: `os.appResourcePath` raises ErrUnsupported when the exe path
            // cannot be acquired (the same failure `executablePath` handles).
            "os.appResourcePath",
            // plan-157-B: no home / no passwd entry / the known-folder query failed.
            "os.appDataPath",
            "os.appCachePath",
            "os.userHomePath",
            "os.userDocumentsPath",
        ],
    ) {
        push_string_value(&mut values, err_msg("ErrUnsupported"));
    }
    // plan-55-B: `os.appResourcePath` additionally raises ErrInvalidPath when the
    // `relative` argument contains a `.`/`..` path component.
    if module_uses_any_call(
        module,
        &[
            "os.appResourcePath",
            "os.appDataPath",
            "os.appCachePath",
            "os.userHomePath",
            "os.userDocumentsPath",
        ],
    ) {
        push_string_value(&mut values, err_msg("ErrInvalidPath"));
    }
    if module_uses_any_call(
        module,
        &[
            "fs.setCurrentDirectory",
            "fs.deleteFile",
            "fs.createDirectory",
            "fs.deleteDirectory",
            "fs.listDirectory",
        ],
    ) {
        for value in [
            err_msg("ErrInvalidArgument"),
            err_msg("ErrNotFound"),
            err_msg("ErrAccessDenied"),
            err_msg("ErrAlreadyExists"),
            err_msg("ErrResourceBusy"),
            err_msg("ErrWriteFailed"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    if module_uses_any_call(
        module,
        &[
            "fs.open",
            "fs.openFile",
            "fs.openFileNoFollow",
            "fs.canonicalPath",
            "fs.isWithin",
            "fs.writeTextAtomic",
            "fs.writeBytesAtomic",
            "fs.close",
            "fs.writeAll",
        ],
    ) {
        for value in [
            err_msg("ErrInvalidArgument"),
            err_msg("ErrNotFound"),
            err_msg("ErrAccessDenied"),
            err_msg("ErrAlreadyExists"),
            err_msg("ErrWriteFailed"),
            err_msg("ErrResourceClosed"),
            err_msg("ErrResourceMoved"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    // `term.terminalSize` and `term.measureText` raise ErrUnsupported while the
    // terminal surface is inactive (terminalSize can also fail to read its size);
    // the `term::` draw helpers raise it in the Windows app/GDI backend,
    // which has no cell grid to stamp into (`win_x86_64::app::emit_app_term_helper`).
    // Either use needs the `_mfb_str_error_unsupported` data object present so the
    // error-return relocation resolves.
    if module_uses_any_call(
        module,
        &[
            "term.terminalSize",
            "term.measureText",
            "term.drawHLine",
            "term.drawVLine",
            "term.drawBox",
            "term.fillRect",
            "term.drawText",
            "term.drawGlyph",
        ],
    ) {
        push_string_value(&mut values, err_msg("ErrUnsupported"));
    }
    if module_uses_any_call(
        module,
        &[
            "net.lookup",
            "net.ping",
            "net.pingAddr",
            // plan-110-B: `tcp` raises the same error set as the `net` transport
            // members it replaces, so its calls arm the same message pool.
            "tcp.connect",
            "tcp.connectAddr",
            "tcp.listen",
            "tcp.accept",
            "tcp.read",
            "tcp.write",
            "tcp.writeText",
            "tcp.poll",
            "tcp.pollList",
            "tcp.close",
            "tcp.localAddress",
            "tcp.remoteAddress",
            "tcp.setReadTimeout",
            "tcp.setWriteTimeout",
            // plan-110-C: same error set as the `net` datagram members it replaces.
            "udp.bind",
            "udp.send",
            "udp.sendText",
            "udp.receive",
            "udp.poll",
            "udp.pollList",
            "udp.close",
            "udp.localAddress",
            "udp.setReadTimeout",
            "udp.setWriteTimeout",
        ],
    ) {
        for value in [
            err_msg("ErrAddressInvalid"),
            err_msg("ErrAddressNotFound"),
            err_msg("ErrNetworkFailed"),
            err_msg("ErrConnectionClosed"),
            err_msg("ErrMessageTooLarge"),
            err_msg("ErrResourceClosed"),
            err_msg("ErrResourceMoved"),
            err_msg("ErrCloseFailed"),
            err_msg("ErrEncoding"),
            err_msg("ErrTimeout"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    // `crypto::randomBytes` fails `ErrInvalidArgument` on a negative count and
    // `ErrUnknown` on an (essentially unreachable) OS-entropy failure
    // (plan-04-crypto.md §A.6).
    if module_uses_call(module, "crypto.randomBytes") {
        for value in [err_msg("ErrInvalidArgument"), err_msg("ErrUnknown")] {
            push_string_value(&mut values, value);
        }
    }
    // Every `tls::` helper that can raise one of these must be listed, including
    // the server-side ones: a `listen`+`accept` program's closes are emitted by
    // scope-drop rather than as NIR calls, so `tls.close`/`tls.closeListener`
    // alone never fire the gate for it (bug-249).
    if module_uses_any_call(
        module,
        &[
            "tls.connect",
            "tls.listen",
            "tls.accept",
            "tls.read",
            "tls.write",
            "tls.writeText",
            "tls.poll",
            "tls.pollList",
            "tls.close",
            "tls.closeListener",
            // The endpoint queries raise ErrResourceClosed/ErrNetworkFailed too.
            // Neither can fire this gate on its own — a handle to ask about only
            // comes from `connect`/`listen`/`accept`, all listed above — but the
            // rule this list encodes is "every helper that can raise one of these
            // is named", and bug-249 is what listing by inference costs.
            "tls.localAddress",
            "tls.localAddressListener",
            "tls.remoteAddress",
        ],
    ) {
        for value in [
            err_msg("ErrTlsFailed"),
            err_msg("ErrAddressInvalid"),
            err_msg("ErrAddressNotFound"),
            err_msg("ErrNetworkFailed"),
            err_msg("ErrConnectionClosed"),
            err_msg("ErrResourceClosed"),
            err_msg("ErrResourceMoved"),
            err_msg("ErrInvalidArgument"),
            err_msg("ErrEncoding"),
            err_msg("ErrTimeout"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    // Audio helpers raise ErrAudioUnavailable / ErrAudioDevice, and validate
    // parameters with ErrInvalidArgument (plan-33-A §7). Emit whenever any
    // `audio.*` call is present (surface or internal).
    if module_uses_any_call(
        module,
        &[
            "audio.devices",
            "audio.openInput",
            "audio.openInputDevice",
            "audio.openOutput",
            "audio.openOutputDevice",
            "audio.read",
            "audio.readTimeout",
            "audio.write",
            "audio.poll",
            "audio.pollTimeout",
            "audio.available",
            "audio.xruns",
            "audio.closeInput",
            "audio.closeOutput",
        ],
    ) {
        for value in [
            err_msg("ErrAudioUnavailable"),
            err_msg("ErrAudioDevice"),
            err_msg("ErrInvalidArgument"),
            // bug-525: `audio::close` refuses an already-closed stream with
            // ErrResourceClosed, so its message has to be present whenever any
            // `audio.` call is — a scope-exit close is emitted by cleanup rather
            // than as a NIR call, exactly like the `tls` case bug-249 records.
            err_msg("ErrResourceClosed"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    // plan-90: `process::spawn`/`shell` raise ErrSpawnFailed; lifecycle ops raise
    // ErrResourceClosed on a dropped handle; spawn raises ErrInvalidArgument on an
    // empty argv and ErrAllocation on OOM; the send/poll timeout overloads raise
    // ErrTimeout. `__drop` is emitted by scope-drop, so a program that only spawns
    // still needs the close-path strings. The whole surface is listed so an I/O-
    // or signal-only reference still pulls the shared close/timeout strings.
    // bug-475 adds ErrResourceBusy: `waitFor` drains the child's pipes while it
    // waits, and raises rather than buffering without bound past its cap.
    let process_calls = [
        "process.spawn",
        "process.spawnEnv",
        "process.shell",
        "process.pid",
        "process.isRunning",
        "process.waitFor",
        "process.closeInput",
        "process.send",
        "process.sendTimeout",
        "process.sendBytes",
        "process.sendBytesTimeout",
        "process.receive",
        "process.receiveFrom",
        "process.receiveBytes",
        "process.receiveBytesFrom",
        "process.poll",
        "process.pollFrom",
        "process.signal",
        "process.didSignal",
        "process.detach",
        "process.__drop",
    ];
    if module_uses_any_call(module, &process_calls) {
        for value in [
            err_msg("ErrSpawnFailed"),
            err_msg("ErrResourceClosed"),
            err_msg("ErrInvalidArgument"),
            err_msg("ErrOutOfMemory"),
            err_msg("ErrTimeout"),
            err_msg("ErrResourceBusy"),
        ] {
            push_string_value(&mut values, value);
        }
    }
    // `process::receive`/`receiveFrom` validate the line as UTF-8 and raise
    // ErrEncoding on a malformed byte sequence, so the receive path needs the
    // encoding string even when the program never calls `toString`.
    if module_uses_any_call(module, &["process.receive", "process.receiveFrom"]) {
        push_string_value(&mut values, err_msg("ErrEncoding"));
    }
    if module_uses_migrated(module, "find")
        || module_uses_migrated(module, "mid")
        || module_uses_migrated(module, "get")
        || module_uses_migrated(module, "append")
        || module_uses_migrated(module, "prepend")
        || module_uses_migrated(module, "insert")
        || module_uses_migrated(module, "transform")
        || module_uses_migrated(module, "filter")
        || module_uses_migrated(module, "removeAt")
        || module_uses_migrated(module, "set")
        || module_uses_call(module, "collections.repeat")
        || module_uses_call(module, "strings.graphemeAt")
        || crate::codegen::collection::assign::self_update::module_self_updates_with_replace(module)
    {
        push_string_value(&mut values, err_msg("ErrIndexOutOfRange"));
    }
    if module_uses_migrated(module, "find") || module_uses_migrated(module, "get") {
        push_string_value(&mut values, err_msg("ErrNotFound"));
    }
    if module_uses_call(module, "toString") {
        push_string_value(&mut values, "TRUE".to_string());
        push_string_value(&mut values, "FALSE".to_string());
        push_string_value(&mut values, err_msg("ErrEncoding"));
    }
    for value in [
        ENTRY_ERROR_THREAD,
        ENTRY_ERROR_PREFIX,
        ENTRY_ERROR_NEWLINE,
        ENTRY_ERROR_MESSAGE_PREFIX,
        ENTRY_ERROR_LOCATION_PREFIX,
        ENTRY_ERROR_LOCATION_SEPARATOR,
    ] {
        if !values.contains(&value.to_string()) {
            values.push(value.to_string());
        }
    }
    if module_may_record_cleanup_failure(module)
        && !values.contains(&CLEANUP_FAILURE_PREFIX.to_string())
    {
        values.push(CLEANUP_FAILURE_PREFIX.to_string());
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let symbol = if let Some(symbol) = standard_error_message_symbol(&value) {
                symbol.to_string()
            } else if value == ENTRY_ERROR_THREAD {
                ENTRY_ERROR_THREAD_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_PREFIX {
                ENTRY_ERROR_PREFIX_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_NEWLINE {
                ENTRY_ERROR_NEWLINE_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_MESSAGE_PREFIX {
                ENTRY_ERROR_MESSAGE_PREFIX_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_LOCATION_PREFIX {
                ENTRY_ERROR_LOCATION_PREFIX_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_LOCATION_SEPARATOR {
                ENTRY_ERROR_LOCATION_SEPARATOR_SYMBOL.to_string()
            } else if value == CLEANUP_FAILURE_PREFIX {
                CLEANUP_FAILURE_PREFIX_SYMBOL.to_string()
            } else {
                format!("_mfb_str_{index}")
            };
            (value, symbol)
        })
        .collect()
}

/// The Eisel–Lemire powers-of-ten table behind `_mfb_rt_string_to_float`
/// (plan-120-F). Emitted only when some generated function relocates against
/// it, the same reference-gated rule the unicode tables use — a program that
/// never converts a String to a Float should not carry 10 KiB of rodata.
pub(crate) fn powers_of_ten_data_object() -> CodeDataObject {
    use crate::codegen::string::format::float_parse_table;
    raw_data_object(
        float_parse_table::POWERS_OF_TEN_SYMBOL,
        "128-bit normalized powers of ten, 16 bytes each (lo, hi), indexed by q - Q_MIN",
        float_parse_table::ENTRY_COUNT * 16,
        float_parse_table::powers_of_ten_hex(),
        16,
    )
}

fn raw_data_object(
    symbol: &str,
    layout: &str,
    size: usize,
    value: String,
    alignment: usize,
) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: layout.to_string(),
        align: alignment,
        size: align(size, alignment),
        value,
    }
}

/// Build the `mfb.string.v1` constant data object for `value` — the single
/// layout every String literal, the empty-string sentinel, and every runtime
/// error message share (a `u64` byte length, the bytes, then a trailing NUL).
/// `size` is that footprint padded to the 8-byte String alignment; for the
/// empty string this is `align(9, 8) == 16`, matching the former hardcode.
pub(crate) fn string_data_object(symbol: &str, value: String) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "constant".to_string(),
        layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }".to_string(),
        align: 8,
        size: align(8 + value.len() + 1, 8),
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::{NirSourceLoc, NirValue};
    use std::collections::{HashMap, HashSet};

    /// plan-158-A: the prefix symbol carries the error name and the call name, and
    /// the relocation scan rebuilds the exact message prefix from it.
    #[test]
    fn fail_status_prefix_symbol_round_trips() {
        let symbol = error_status_prefix_symbol("ErrAudioDevice", "IAudioRenderClient::GetBuffer");
        assert_eq!(
            symbol,
            "_mfb_str_errstatus_ErrAudioDevice_49417564696f52656e646572436c69656e743a3a476574427566666572"
        );
        assert_eq!(
            decode_error_status_prefix_symbol(&symbol).as_deref(),
            Some(
                "Audio device open, configuration, or stream operation failed. \
                 (IAudioRenderClient::GetBuffer: "
            )
        );
    }

    /// plan-158-A: anything but the canonical spelling of a known error decodes to
    /// `None`, so the scan never emits a data object for a symbol it did not mint.
    #[test]
    fn fail_status_prefix_symbol_rejects_malformed() {
        for symbol in [
            "_mfb_str_error_audio_device",
            "_mfb_str_errstatus_ErrAudioDevice",
            "_mfb_str_errstatus_ErrAudioDevice_",
            "_mfb_str_errstatus_ErrAudioDevice_4",
            "_mfb_str_errstatus_ErrAudioDevice_4G",
            "_mfb_str_errstatus_ErrAudioDevice_4A",
            "_mfb_str_errstatus_ErrAudioDevice_+4",
            "_mfb_str_errstatus_ErrAudioDevice_ff",
            "_mfb_str_errstatus_ErrNoSuchError_41",
        ] {
            assert_eq!(decode_error_status_prefix_symbol(symbol), None, "{symbol}");
        }
    }

    fn const_of(type_: &ParameterType) -> NirValue {
        NirValue::Const {
            type_: type_.clone(),
            value: String::new(),
        }
    }

    fn call(target: &str, arg_types: &[&str]) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args: arg_types
                .iter()
                .map(|t| const_of(&crate::types::ParameterType::declared(t)))
                .collect(),
            loc: NirSourceLoc::default(),
        }
    }

    /// bug-354: the `typeName` fold happens in two places that MUST agree — the
    /// builder's `CodeBuilder::static_type_name_for_fold`
    /// (builder_value_semantics/static_eval.rs) emits the fold, and this pre-pass's
    /// `static_type_name_for_fold_with_types` interns the folded string the builder
    /// then looks up. They had drifted (the builder's base table knew zero
    /// `strings.*`; this side's base table knew 18 and no `math.*`), with no test
    /// relating them. Both fold wrappers now delegate any target their hand-written
    /// base table misses to the single authoritative resolver
    /// `builtins::resolve_call_return_type`. This pins that: for every builtin call
    /// target, the pre-pass fold equals the resolver's answer — so a future base-
    /// table arm that contradicts the resolver, or a resolver retype, fails here.
    /// The builder side's runtime output over the same catalog is proven by
    /// `tests/rt-behavior/general/func_typename_builtin_calls`.
    #[test]
    fn typename_fold_agrees_with_the_authoritative_resolver() {
        let types = HashMap::new();
        let fields = FieldTypes::new();
        let catalog: &[(&str, &[&str])] = &[
            // strings.* — the whole package was uncompilable in the builder fold.
            ("strings.upper", &["String"]),
            ("strings.lower", &["String"]),
            ("strings.trim", &["String"]),
            ("strings.caseFold", &["String"]),
            ("strings.normalizeNfc", &["String"]),
            ("strings.join", &["List OF String", "String"]),
            ("strings.split", &["String", "String"]),
            ("strings.graphemes", &["String"]),
            ("strings.byteLen", &["String"]),
            ("strings.contains", &["String", "String"]),
            ("strings.startsWith", &["String", "String"]),
            ("strings.padLeft", &["String", "Integer", "String"]),
            ("strings.padRight", &["String", "Integer", "String"]),
            ("strings.mid", &["String", "Integer", "Integer"]),
            ("strings.replace", &["String", "String", "String"]),
            ("strings.repeat", &["String", "Integer"]),
            ("strings.stripPrefix", &["String", "String"]),
            ("strings.stripSuffix", &["String", "String"]),
            ("strings.count", &["String", "String"]),
            // math.* — abs/min/max were in neither base table.
            ("math.abs", &["Float"]),
            ("math.min", &["Float", "Float"]),
            ("math.max", &["Float", "Float"]),
            ("math.sqrt", &["Float"]),
            ("math.pow", &["Float", "Float"]),
            // collections.* predicate/search returns.
            ("collections.find", &["List OF String", "String"]),
            ("collections.contains", &["List OF String", "String"]),
            (
                "collections.hasKey",
                &["Map OF String TO Integer", "String"],
            ),
            // general.* contrast cases (already resolved before the fix).
            ("toString", &["Integer"]),
            ("toInt", &["String"]),
            ("toFloat", &["String"]),
            ("isNumeric", &["String"]),
            ("typeName", &["String"]),
        ];
        for (target, arg_types) in catalog {
            // plan-106-E: both sides are `ParameterType` now, so the comparison is
            // structural rather than a name compare.
            let want = builtins::resolve_call_return_type_typed(
                target,
                &arg_types
                    .iter()
                    .map(|t| ParameterType::parse(t))
                    .collect::<Vec<_>>(),
                false,
            );
            let got =
                static_type_name_for_fold_with_types(&call(target, arg_types), &types, &fields);
            assert_eq!(
                got, want,
                "`{target}` folds to {got:?} in the pre-pass but the authoritative \
                 resolver says {want:?} — the two typeName folds have drifted (bug-354)"
            );
            assert!(
                got.is_some(),
                "`{target}` must resolve — it is a documented builtin call"
            );
        }
    }

    /// plan-77 U5: `unicode_runtime_data_objects` emits exactly the tables whose
    /// symbols are referenced. A `strings::graphemes`-only program (which reaches
    /// only the base trie) must NOT carry the six case-mapping tables nor the
    /// NFD/composition tables; a case-mapping-only program must NOT carry the base
    /// trie. `None` still emits every table (the coarse fallback).
    ///
    /// plan-118-B added the general-category and Script run tables, which are the
    /// largest objects here (46 KB together) and are read by `regex`/`strings`
    /// property queries only — so they join the leak check below, not just the
    /// count.
    #[test]
    fn unicode_runtime_data_objects_emit_only_referenced_tables() {
        let all = unicode_runtime_data_objects(None);
        assert_eq!(all.len(), 18, "None must emit every unicode table");
        let every: HashSet<&str> = all.iter().map(|o| o.symbol.as_str()).collect();
        for added in [
            UNICODE_GENCAT_RANGES_SYMBOL,
            UNICODE_GENCAT_NAMES_SYMBOL,
            UNICODE_SCRIPT_RANGES_SYMBOL,
            UNICODE_SCRIPT_NAMES_SYMBOL,
            UNICODE_LINEBREAK_RANGES_SYMBOL,
        ] {
            assert!(every.contains(added), "None must emit `{added}`");
        }

        let base: HashSet<&str> = [
            UNICODE_STAGE1_SYMBOL,
            UNICODE_STAGE2_SYMBOL,
            UNICODE_PROPERTIES_SYMBOL,
        ]
        .into_iter()
        .collect();
        let graphemes_only = unicode_runtime_data_objects(Some(&base));
        let emitted: HashSet<&str> = graphemes_only.iter().map(|o| o.symbol.as_str()).collect();
        assert_eq!(
            emitted, base,
            "graphemes-only must emit exactly the base trie"
        );
        for dead in [
            UNICODE_CASEFOLD_ENTRIES_SYMBOL,
            UNICODE_CASEFOLD_SEQUENCES_SYMBOL,
            UNICODE_UPPERCASE_ENTRIES_SYMBOL,
            UNICODE_UPPERCASE_SEQUENCES_SYMBOL,
            UNICODE_LOWERCASE_ENTRIES_SYMBOL,
            UNICODE_LOWERCASE_SEQUENCES_SYMBOL,
            UNICODE_NFD_ENTRIES_SYMBOL,
            UNICODE_COMBINATIONS_SECOND_SYMBOL,
            // plan-118-B: 46 KB of category/script runs a grapheme walk never
            // reads. The relocation scan is what keeps them out; this pins it.
            UNICODE_GENCAT_RANGES_SYMBOL,
            UNICODE_GENCAT_NAMES_SYMBOL,
            UNICODE_SCRIPT_RANGES_SYMBOL,
            UNICODE_SCRIPT_NAMES_SYMBOL,
            // plan-162-E: 22 KB of line-break runs only `strings::lineBreaks` reads.
            UNICODE_LINEBREAK_RANGES_SYMBOL,
        ] {
            assert!(
                !emitted.contains(dead),
                "graphemes-only leaked `{dead}` it never reads"
            );
        }

        let casefold: HashSet<&str> = [
            UNICODE_CASEFOLD_ENTRIES_SYMBOL,
            UNICODE_CASEFOLD_SEQUENCES_SYMBOL,
        ]
        .into_iter()
        .collect();
        let casefold_only = unicode_runtime_data_objects(Some(&casefold));
        let emitted: HashSet<&str> = casefold_only.iter().map(|o| o.symbol.as_str()).collect();
        assert_eq!(
            emitted, casefold,
            "casefold-only must emit exactly its two tables (no base trie)"
        );
    }
}
