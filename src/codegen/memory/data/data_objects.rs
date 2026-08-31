// --- codegen tier imports (migration) ---
use crate::codegen::builtins;
use crate::codegen::engine::analysis::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
use std::collections::HashMap;
use std::collections::HashSet;
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

pub(crate) fn string_symbols(module: &NirModule) -> HashMap<String, String> {
    let mut values = Vec::new();
    // The module's record / union-variant field types, so every walk below can
    // type a `MemberAccess` (bug-363, bug-366).
    let fields = module_field_types(module);
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
        &["io.input", "io.readLine", "io.readChar", "io.readByte"],
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
    }
    if module_uses_call(module, "io.pollInput") {
        push_string_value(&mut values, err_msg("ErrInputFailed"));
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
            "canvas.presentLayers",
            "canvas.publishScene",
            "canvas.publishLayers",
            "canvas.blitSurface",
            "canvas.frameDone",
            "canvas.syncFrame",
            "canvas.setSyncMode",
            "canvas.surfaceWidth",
            "canvas.surfaceHeight",
            "canvas.setMetalMode",
            "canvas.useMetal",
            "canvas.metalAvailable",
            "canvas.vulkanAvailable",
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
            "canvas.imageRef",
            "canvas.getSize",
            "canvas.getBytes",
            "canvas.setBytes",
        ],
    ) {
        for value in [
            err_msg("ErrOutOfMemory"),
            err_msg("ErrResourceClosed"),
            err_msg("ErrBadPixelCount"),
            err_msg("ErrWrongMode"),
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
        // (both bits live in the offset-8 word, and the guard splits them only at
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
            // plan-55-B: `os.resourcePath` raises ErrUnsupported when the exe path
            // cannot be acquired (the same failure `executablePath` handles).
            "os.resourcePath",
        ],
    ) {
        push_string_value(&mut values, err_msg("ErrUnsupported"));
    }
    // plan-55-B: `os.resourcePath` additionally raises ErrInvalidPath when the
    // `relative` argument contains a `.`/`..` path component.
    if module_uses_call(module, "os.resourcePath") {
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
    // `term.terminalSize` raises ErrUnsupported when the console size cannot be
    // read; the `term::` draw helpers raise it in the Windows app/GDI backend,
    // which has no cell grid to stamp into (`win_x86_64::app::emit_app_term_helper`).
    // Either use needs the `_mfb_str_error_unsupported` data object present so the
    // error-return relocation resolves.
    if module_uses_any_call(
        module,
        &[
            "term.terminalSize",
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
    let process_calls = [
        "process.spawn",
        "process.spawnEnv",
        "process.shell",
        "process.pid",
        "process.isRunning",
        "process.waitFor",
        "process.close",
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
        || module_uses_call(module, "strings.graphemeAt")
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
    for value in [ENTRY_ERROR_PREFIX, ENTRY_ERROR_NEWLINE] {
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
            } else if value == ENTRY_ERROR_PREFIX {
                ENTRY_ERROR_PREFIX_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_NEWLINE {
                ENTRY_ERROR_NEWLINE_SYMBOL.to_string()
            } else if value == CLEANUP_FAILURE_PREFIX {
                CLEANUP_FAILURE_PREFIX_SYMBOL.to_string()
            } else {
                format!("_mfb_str_{index}")
            };
            (value, symbol)
        })
        .collect()
}

/// Error messages emitted by native `LINK` thunks and their initializer
/// (plan-linker.md §12): the allocation message is already covered by the
/// standard set, so only the two binding-specific messages are listed here.

fn collect_type_name_values(module: &NirModule, values: &mut Vec<String>) {
    for value in [
        "Boolean", "Byte", "Error", "Fixed", "Float", "Integer", "Money", "Nothing", "Scalar",
        "String",
    ] {
        push_string_value(values, value.to_string());
    }
    for type_ in &module.types {
        push_string_value(values, type_.name.clone());
        for field in &type_.fields {
            push_string_value(values, field.type_.name().into_owned());
        }
        for variant in &type_.variants {
            push_string_value(values, variant.name.clone());
            for field in &variant.fields {
                push_string_value(values, field.type_.name().into_owned());
            }
        }
    }
    for function in &module.functions {
        push_string_value(values, function.returns.name().into_owned());
        for param in &function.params {
            push_string_value(values, param.type_.name().into_owned());
        }
        collect_type_name_values_from_ops(&function.body, values);
    }
}

fn collect_type_name_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    use nir::visit::{walk_op, walk_value, NirVisitor};
    struct Collector<'a> {
        values: &'a mut Vec<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { type_, .. } => {
                    push_string_value(self.values, type_.name().into_owned())
                }
                NirOp::StoreGlobal { type_, .. } if !type_.name().is_empty() => {
                    push_string_value(self.values, type_.name().into_owned())
                }
                NirOp::For { type_, .. } | NirOp::ForEach { type_, .. } => {
                    push_string_value(self.values, type_.name().into_owned())
                }
                _ => {}
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Const { type_, .. }
                | NirValue::FunctionRef { type_, .. }
                | NirValue::Constructor { type_, .. }
                | NirValue::ListLiteral { type_, .. }
                | NirValue::SetLiteral { type_, .. }
                | NirValue::MapLiteral { type_, .. }
                | NirValue::UnionExtract { type_, .. }
                | NirValue::WithUpdate { type_, .. } => {
                    push_string_value(self.values, type_.name().into_owned())
                }
                NirValue::UnionWrap {
                    union_type,
                    member_type,
                    ..
                } => {
                    push_string_value(self.values, union_type.name().into_owned());
                    push_string_value(self.values, member_type.name().into_owned());
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }
    Collector { values }.visit_ops(ops);
}

pub(crate) fn unicode_string_call_is_static(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> bool {
    matches!(
        target,
        "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.graphemes"
            | "strings.displayWidth"
    ) && args.len() == 1
        && static_string_value_with_constants(&args[0], constants, types, fields).is_some()
}

/// The unicode runtime tables to emit. `referenced` is the set of
/// `_mfb_unicode_*` data symbols that some generated function actually relocates
/// against — the ground truth for which tables are live (plan-77 U5). Only those
/// tables are emitted, so e.g. a `strings::graphemes`-only program drops the six
/// case-mapping tables and the NFD/composition tables it never touches. `None`
/// emits every table (the coarse fallback for the — practically unreachable —
/// case where the NIR heuristic reports unicode use but no relocation names a
/// specific table, preserving the pre-split all-or-nothing behaviour rather than
/// risking an undefined symbol).
pub(crate) fn unicode_runtime_data_objects(
    referenced: Option<&std::collections::HashSet<&str>>,
) -> Vec<CodeDataObject> {
    let tables = crate::unicode::runtime_tables::tables();
    let keep = |symbol: &str| referenced.is_none_or(|set| set.contains(symbol));
    let mut objects = Vec::new();
    if keep(UNICODE_STAGE1_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_STAGE1_SYMBOL,
            "u16 utf8proc stage1 property index table",
            tables.stage1.len() * 2,
            crate::unicode::runtime_tables::stage1_hex(),
            2,
        ));
    }
    if keep(UNICODE_STAGE2_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_STAGE2_SYMBOL,
            "u16 utf8proc stage2 property index table",
            tables.stage2.len() * 2,
            crate::unicode::runtime_tables::stage2_hex(),
            2,
        ));
    }
    if keep(UNICODE_PROPERTIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_PROPERTIES_SYMBOL,
            "mfb.unicode.property.v1 records, 12 bytes each",
            tables.properties.len() * 12,
            crate::unicode::runtime_tables::properties_hex(),
            2,
        ));
    }
    if keep(UNICODE_COMBINATIONS_SECOND_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_COMBINATIONS_SECOND_SYMBOL,
            "u32 utf8proc composition second codepoint table",
            tables.combinations_second.len() * 4,
            crate::unicode::runtime_tables::combinations_second_hex(),
            4,
        ));
    }
    if keep(UNICODE_COMBINATIONS_COMBINED_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_COMBINATIONS_COMBINED_SYMBOL,
            "u32 utf8proc composition combined codepoint table",
            tables.combinations_combined.len() * 4,
            crate::unicode::runtime_tables::combinations_combined_hex(),
            4,
        ));
    }
    if keep(UNICODE_NFD_ENTRIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_NFD_ENTRIES_SYMBOL,
            "mfb.unicode.nfd_entry.v1 records, 16 bytes each",
            tables.nfd_entries.len() * 16,
            crate::unicode::runtime_tables::nfd_entries_hex(),
            4,
        ));
    }
    if keep(UNICODE_NFD_SEQUENCES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_NFD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode NFD sequence table",
            tables.nfd_sequences.len() * 4,
            crate::unicode::runtime_tables::nfd_sequences_hex(),
            4,
        ));
    }
    if keep(UNICODE_UPPERCASE_ENTRIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_UPPERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 uppercase records, 16 bytes each",
            tables.uppercase_entries.len() * 16,
            crate::unicode::runtime_tables::uppercase_entries_hex(),
            4,
        ));
    }
    if keep(UNICODE_UPPERCASE_SEQUENCES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_UPPERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode uppercase sequence table",
            tables.uppercase_sequences.len() * 4,
            crate::unicode::runtime_tables::uppercase_sequences_hex(),
            4,
        ));
    }
    if keep(UNICODE_LOWERCASE_ENTRIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_LOWERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 lowercase records, 16 bytes each",
            tables.lowercase_entries.len() * 16,
            crate::unicode::runtime_tables::lowercase_entries_hex(),
            4,
        ));
    }
    if keep(UNICODE_LOWERCASE_SEQUENCES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_LOWERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode lowercase sequence table",
            tables.lowercase_sequences.len() * 4,
            crate::unicode::runtime_tables::lowercase_sequences_hex(),
            4,
        ));
    }
    if keep(UNICODE_CASEFOLD_ENTRIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_CASEFOLD_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 casefold records, 16 bytes each",
            tables.casefold_entries.len() * 16,
            crate::unicode::runtime_tables::casefold_entries_hex(),
            4,
        ));
    }
    if keep(UNICODE_CASEFOLD_SEQUENCES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_CASEFOLD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode casefold sequence table",
            tables.casefold_sequences.len() * 4,
            crate::unicode::runtime_tables::casefold_sequences_hex(),
            4,
        ));
    }
    objects
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

/// Walk one function's body for string literals that need a data object.
///
/// The local-type map is seeded with the function's parameters. The code
/// builder records every parameter as a local carrying its declared type, so it
/// folds `typeName(param)` — and any `&` concatenation around it — to a literal.
/// Starting this pass with an empty map made its view of local types strictly
/// weaker than the builder's, so a fold the builder performed produced a literal
/// this pass had never seen and the build aborted with no data object for it
/// (bug-361B).
fn collect_string_values_from_function(
    function: &NirFunction,
    values: &mut Vec<String>,
    fields: &FieldTypes,
) {
    let mut constants = HashMap::new();
    let mut types: HashMap<String, ParameterType> = function
        .params
        .iter()
        .map(|param| (param.name.clone(), param.type_.clone()))
        .collect();
    collect_string_values_from_ops_with_constants(
        &function.body,
        values,
        &mut constants,
        &mut types,
        fields,
    );
}

fn collect_string_values_from_ops_with_constants(
    ops: &[NirOp],
    values: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, ParameterType>,
    fields: &FieldTypes,
) {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types, fields);
                    if let Some(constant) =
                        local_constant_value_with_constants(value, constants, types, fields)
                    {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types, fields);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types, fields);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_string_values_from_value(code, values, constants, types, fields);
            }
            NirOp::Fail { error } => {
                collect_string_values_from_value(error, values, constants, types, fields);
            }
            NirOp::StateAssign { value, .. } => {
                collect_string_values_from_value(value, values, constants, types, fields);
            }
            NirOp::Assign { name, value } => {
                collect_string_values_from_value(value, values, constants, types, fields);
                if let Some(constant) =
                    local_constant_value_with_constants(value, constants, types, fields)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Eval { value } => {
                collect_string_values_from_value(value, values, constants, types, fields);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_string_values_from_value(condition, values, constants, types, fields);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    then_body,
                    values,
                    &mut then_constants,
                    &mut then_types,
                    fields,
                );
                collect_string_values_from_ops_with_constants(
                    else_body,
                    values,
                    &mut else_constants,
                    &mut else_types,
                    fields,
                );
            }
            NirOp::Match { value, cases } => {
                collect_string_values_from_value(value, values, constants, types, fields);
                for case in cases {
                    // Exhaustive on purpose: an `if let` here silently skipped
                    // `OneOf`, so `CASE "B", "C"` reached codegen with no data
                    // object for either literal (bug-361A). Keeping the match
                    // exhaustive makes the next pattern variant a build error
                    // rather than another silent miss.
                    match &case.pattern {
                        NirMatchPattern::Value(value) => {
                            collect_string_values_from_value(
                                value, values, constants, types, fields,
                            );
                        }
                        NirMatchPattern::OneOf(patterns) => {
                            for pattern in patterns {
                                collect_string_values_from_value(
                                    pattern, values, constants, types, fields,
                                );
                            }
                        }
                        NirMatchPattern::Else => {}
                    }
                    // A guard is a value expression that may hold string
                    // literals; without walking it, `fs::exists("/tmp/x")` in a
                    // `WHEN` guard has no data object at codegen (bug-118).
                    if let Some(guard) = &case.guard {
                        collect_string_values_from_value(guard, values, constants, types, fields);
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    collect_string_values_from_ops_with_constants(
                        &case.body,
                        values,
                        &mut case_constants,
                        &mut case_types,
                        fields,
                    );
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_string_values_from_value(condition, values, constants, types, fields);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                );
            }
            NirOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_string_values_from_value(start, values, constants, types, fields);
                collect_string_values_from_value(end, values, constants, types, fields);
                collect_string_values_from_value(step, values, constants, types, fields);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                );
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                );
                collect_string_values_from_value(condition, values, constants, types, fields);
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                collect_string_values_from_value(iterable, values, constants, types, fields);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                );
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                let mut trap_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut trap_constants,
                    &mut trap_types,
                    fields,
                );
            }
        }
    }
}

fn collect_string_values_from_value(
    value: &NirValue,
    values: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) {
    if let Some(value) = static_string_value_with_constants(value, constants, types, fields) {
        push_string_value(values, value);
    }
    if let NirValue::Call { target, args, .. }
    | NirValue::CallResult { target, args, .. }
    | NirValue::RuntimeCall { target, args, .. } = value
    {
        if target == "strings.graphemes" && args.len() == 1 {
            if let Some(value) =
                static_string_value_with_constants(&args[0], constants, types, fields)
            {
                for grapheme in crate::unicode::backend::graphemes(&value) {
                    push_string_value(values, grapheme);
                }
            }
        }
        if target == "fs.pathJoin" && args.len() == 1 {
            push_string_value(values, "/".to_string());
        }
        if target == "fs.pathDirName" && args.len() == 1 {
            push_string_value(values, ".".to_string());
            push_string_value(values, "/".to_string());
        }
    }
    if value_may_return_invalid_format(value, constants, types, fields) {
        push_string_value(values, err_msg("ErrInvalidFormat"));
    }
    match value {
        NirValue::Const { type_, value } if matches!(type_, ParameterType::String) => {
            push_string_value(values, value.clone());
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_string_values_from_value(arg, values, constants, types, fields);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_string_values_from_value(value, values, constants, types, fields)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_string_values_from_value(target, values, constants, types, fields);
            for update in updates {
                collect_string_values_from_value(&update.value, values, constants, types, fields);
            }
        }
        NirValue::ListLiteral { values: items, .. }
        | NirValue::SetLiteral { values: items, .. } => {
            for item in items {
                collect_string_values_from_value(item, values, constants, types, fields);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_string_values_from_value(key, values, constants, types, fields);
                collect_string_values_from_value(value, values, constants, types, fields);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_string_values_from_value(target, values, constants, types, fields)
        }
        NirValue::Binary { left, right, .. } => {
            collect_string_values_from_value(left, values, constants, types, fields);
            collect_string_values_from_value(right, values, constants, types, fields);
        }
        NirValue::Unary { operand, .. } => {
            collect_string_values_from_value(operand, values, constants, types, fields)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_string_values_from_value(value, values, constants, types, fields);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
    }
}

fn push_string_value(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

pub(crate) fn static_string_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if matches!(type_, ParameterType::String) => {
            Some(value.clone())
        }
        NirValue::Local(name) => constants.get(name).and_then(|constant| {
            static_string_value_with_constants(constant, constants, types, fields)
        }),
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            // `typeName` yields the argument type's SPELLING — the rendered
            // name IS the program-visible string value here.
            static_type_name_for_fold_with_types(&args[0], types, fields)
                .map(|type_| type_.name().into_owned())
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            strings_package_static_string_value(target, args, constants, types, fields)
        }
        NirValue::Binary {
            op, left, right, ..
        } if op == "&" => {
            let left = static_string_value_with_constants(left, constants, types, fields)?;
            let right = static_string_value_with_constants(right, constants, types, fields)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

pub(crate) fn static_type_name_with_types(
    value: &NirValue,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<ParameterType> {
    match value {
        NirValue::Const { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => types.get(name).cloned(),
        NirValue::LocalRef { type_, .. } => Some(type_.clone()),
        NirValue::Global { type_, .. } if !is_unset_type(type_) => Some(type_.clone()),
        NirValue::Global { .. } => None,
        NirValue::FunctionRef { type_, .. }
        | NirValue::Closure { type_, .. }
        | NirValue::Capture { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::WithUpdate { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::SetLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::UnionWrap { union_type, .. } => Some(union_type.clone()),
        NirValue::UnionExtract { type_, .. } => Some(type_.clone()),
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => match target.as_str() {
            "typeName" | "toString" => Some(ParameterType::String),
            "len" | "toInt" => Some(ParameterType::Integer),
            // Migrated find/mid/replace: strings:: returns Integer/String; the
            // collections:: List overloads return the list type and are resolved
            // by the precise type path, so only `find` (always Integer) is mapped
            // here (plan-01-functions.md §5).
            "collections.find" | "strings.find" => Some(ParameterType::Integer),
            "strings.mid" | "strings.replace" => Some(ParameterType::String),
            "strings.trim"
            | "strings.trimStart"
            | "strings.trimEnd"
            | "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.join" => Some(ParameterType::String),
            "strings.graphemes" | "strings.split" => {
                Some(ParameterType::list_of(ParameterType::String))
            }
            "strings.startsWith" | "strings.endsWith" | "strings.contains" => {
                Some(ParameterType::Boolean)
            }
            "strings.byteLen" => Some(ParameterType::Integer),
            "toFloat" => Some(ParameterType::Float),
            "toFixed" => Some(ParameterType::Fixed),
            "toByte" => Some(ParameterType::Byte),
            "toMoney" => Some(ParameterType::Money),
            "toScalar" => Some(scalar_type()),
            "isNumeric" => Some(ParameterType::Boolean),
            _ => None,
        },
        NirValue::ResultIsOk { .. } => Some(ParameterType::Boolean),
        NirValue::ResultValue { value } => {
            match static_type_name_with_types(value, types, fields)? {
                // A non-`Result` operand answers with its own type, as the
                // `strip_prefix(…).or_else(…)` this replaces did.
                ParameterType::ResultOf(success) => Some(*success),
                other => Some(other),
            }
        }
        NirValue::ResultError { .. } => Some(error_type()),
        NirValue::Binary {
            op, left, right, ..
        } => {
            if matches!(
                op.as_str(),
                "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
            ) {
                return Some(ParameterType::Boolean);
            }
            if op == "&" {
                return Some(ParameterType::String);
            }
            let left = static_type_name_with_types(left, types, fields)?;
            let right = static_type_name_with_types(right, types, fields)?;
            Some(promoted_binary_type(op, &left, &right))
        }
        NirValue::Unary { op, operand, .. } => {
            if op == "NOT" {
                Some(ParameterType::Boolean)
            } else {
                static_type_name_with_types(operand, types, fields)
            }
        }
        NirValue::MemberAccess { target, member } => {
            let target_type = static_type_name_with_types(target, types, fields)?;
            if member == "result" {
                if let ParameterType::ThreadHandle {
                    worker: false, out, ..
                } = &target_type
                {
                    return Some(ParameterType::result_of((**out).clone()));
                }
            }
            // Record and union-variant fields, then the two `MapEntry` members —
            // the same sources `static_nir_value_type` consults. Without the
            // field table this arm answered `None` for every record field, which
            // silently under-reported in every predicate built on this seam:
            // `typeName(rec.field)` failed to lower at all, and the
            // ERR_INVALID_FORMAT gate missed a promoting Float operand (bug-366).
            // `FieldTypes` keys are nominal type NAMES, so the lookup renders
            // the (scalar-cheap) name — a name-keyed table probe.
            if let Some(field_type) = fields.get(&(target_type.name().into_owned(), member.clone()))
            {
                return Some(field_type.clone());
            }
            let (key_type, value_type) = typed_map_entry_type_parts(&target_type)?;
            match member.as_str() {
                "key" => Some(key_type.clone()),
                "value" => Some(value_type.clone()),
                _ => None,
            }
        }
    }
}

/// The pre-pass twin of [`crate::codegen::engine::builder::CodeBuilder::static_type_name_for_fold`]: static
/// type of `value`, resolving builtin calls that [`static_type_name_with_types`]'s
/// hand-written table misses via `builtins::resolve_call_return_type`.
///
/// Used **only** for the `typeName` compile-time fold (bug-354), where the pre-pass
/// interns the folded type-name string the builder later looks up — so this must
/// agree with the builder's `static_type_name_for_fold`, and both delegate to the
/// same resolver. It does NOT widen `static_type_name_with_types`, whose other
/// consumers (the float-numeric-error gate, module analysis, binary typing) must
/// keep their exact current answers.
pub(crate) fn static_type_name_for_fold_with_types(
    value: &NirValue,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<ParameterType> {
    if let Some(type_) = static_type_name_with_types(value, types, fields) {
        return Some(type_);
    }
    match value {
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            let arg_types = args
                .iter()
                .map(|arg| static_type_name_for_fold_with_types(arg, types, fields))
                .collect::<Option<Vec<_>>>()?;
            builtins::resolve_call_return_type_typed(target, &arg_types, false)
        }
        _ => None,
    }
}

pub(crate) fn builtin_function_symbol_for_type(
    name: &str,
    type_: &ParameterType,
) -> Option<String> {
    crate::codegen::builtins::general::builtin_function_id_for_type(name, type_)?;
    Some(format!(
        "_mfb_builtin_{}_{}",
        nir::symbol_fragment(name),
        nir::symbol_fragment(&type_.name())
    ))
}

pub(crate) fn builtin_function_refs(module: &NirModule) -> Vec<(String, ParameterType, String)> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for function in &module.functions {
        collect_builtin_function_refs_in_ops(&function.body, &mut refs, &mut seen);
    }
    refs
}

/// plan-86 K1: names of every top-level function referenced as a `FunctionRef`
/// (used as a callback / function value) ANYWHERE in the module. Such a function
/// is invoked through the generic callback ABI, which takes OWNERSHIP of the
/// callback's return value — e.g. `collections::groupBy` stores each per-element
/// result into a bucket and frees the callee's returned block after use. A
/// parameter-passthrough callback that returned a borrow of its argument would
/// hand that ABI a non-owned pointer to a per-iteration temporary, causing a
/// double-free / UAF (observed: grouped String values came back empty). So a
/// function used as a `FunctionRef` MUST keep returning a fresh owned copy;
/// `function_returns_param_borrow` excludes every name in this set.
pub(crate) fn collect_function_ref_names(module: &NirModule) -> HashSet<String> {
    use nir::visit::{walk_value, NirVisitor};
    struct Collector<'a> {
        out: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::FunctionRef { name, .. } = value {
                self.out.insert(name.clone());
            }
            walk_value(self, value);
        }
    }
    let mut out = HashSet::new();
    for function in &module.functions {
        Collector { out: &mut out }.visit_ops(&function.body);
    }
    out
}

fn collect_builtin_function_refs_in_ops(
    ops: &[NirOp],
    refs: &mut Vec<(String, ParameterType, String)>,
    seen: &mut HashSet<String>,
) {
    use nir::visit::{walk_value, NirVisitor};
    struct Collector<'a> {
        refs: &'a mut Vec<(String, ParameterType, String)>,
        seen: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::FunctionRef { name, type_ } = value {
                if let Some(symbol) = builtin_function_symbol_for_type(name, &type_) {
                    let key = format!("{name}\0{type_}");
                    if self.seen.insert(key) {
                        self.refs.push((name.clone(), type_.clone(), symbol));
                    }
                }
            }
            walk_value(self, value);
        }
    }
    Collector { refs, seen }.visit_ops(ops);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::{NirSourceLoc, NirValue};
    use std::collections::{HashMap, HashSet};

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
    /// (builder_value_semantics.rs) emits the fold, and this pre-pass's
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
    #[test]
    fn unicode_runtime_data_objects_emit_only_referenced_tables() {
        let all = unicode_runtime_data_objects(None);
        assert_eq!(all.len(), 13, "None must emit every unicode table");

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
