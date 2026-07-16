use super::*;

/// Materialize the address of an internal symbol (data or code) into `dst` via
/// the `adrp`/`add` page pair. The `data` binding is the internal-symbol-address
/// relocation regardless of the target's section — the linker resolves it through
/// `symbol_vmaddr` (the same pattern used for the thread-trampoline address).
pub(super) fn push_symbol_address(
    from: &str,
    symbol: &str,
    dst: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::load_page_address(dst, symbol));
    instructions.push(abi::add_page_offset(dst, dst, symbol));
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

pub(super) fn push_error_message_address(
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

pub(super) fn string_symbols(module: &NirModule) -> HashMap<String, String> {
    let mut values = Vec::new();
    if module_uses_type_name(module) {
        collect_type_name_values(module, &mut values);
    }
    for function in &module.functions {
        collect_string_values_from_ops(&function.body, &mut values);
    }
    // Source file paths back `ErrorLoc.filename` for errors that originate in
    // each function; emit them as string constants so the origin can load them.
    for function in &module.functions {
        if !function.file.is_empty() {
            push_string_value(&mut values, function.file.clone());
        }
    }
    for value in [
        ERR_INVALID_ARGUMENT_MESSAGE,
        ERR_OVERFLOW_MESSAGE,
        ERR_UNDERFLOW_MESSAGE,
        ERR_ALLOCATION_MESSAGE,
    ] {
        push_string_value(&mut values, value.to_string());
    }
    if module_may_emit_float_numeric_error(module) {
        for value in [
            ERR_FLOAT_DOMAIN_MESSAGE,
            ERR_FLOAT_NAN_MESSAGE,
            ERR_FLOAT_INF_MESSAGE,
            ERR_FLOAT_OVERFLOW_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
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
        push_string_value(&mut values, ERR_OUTPUT_MESSAGE.to_string());
    }
    if module_uses_any_call(
        module,
        &["io.input", "io.readLine", "io.readChar", "io.readByte"],
    ) {
        if module_uses_call(module, "io.input") {
            push_string_value(&mut values, String::new());
        }
        push_string_value(&mut values, ERR_EOF_MESSAGE.to_string());
        push_string_value(&mut values, ERR_INPUT_MESSAGE.to_string());
        push_string_value(&mut values, ERR_ENCODING_MESSAGE.to_string());
        // plan-15 D1: reading stdin from an unsubscribed thread traps ErrInvalidContext.
        push_string_value(&mut values, ERR_INVALID_CONTEXT_MESSAGE.to_string());
    }
    if module_uses_call(module, "io.pollInput") {
        push_string_value(&mut values, ERR_INPUT_MESSAGE.to_string());
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
        push_string_value(&mut values, ERR_RESOURCE_CLOSED_MESSAGE.to_string());
    }
    if module_uses_call(module, "fs.currentDirectory") {
        push_string_value(&mut values, ERR_READ_MESSAGE.to_string());
    }
    // `os::getEnv` raises `ErrNotFound` for an unset variable; `os::setEnv`
    // reuses the always-emitted `ErrInvalidArgument`/allocation messages
    // (plan-31-A).
    if module_uses_call(module, "os.getEnv") {
        push_string_value(&mut values, ERR_NOT_FOUND_MESSAGE.to_string());
    }
    // `os::hostName`/`userName`/`executablePath` raise ErrUnsupported when the
    // host lookup fails (no passwd entry, unreadable /proc/self/exe, …).
    if module_uses_any_call(
        module,
        &["os.hostName", "os.userName", "os.executablePath"],
    ) {
        push_string_value(&mut values, ERR_UNSUPPORTED_MESSAGE.to_string());
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
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_NOT_FOUND_MESSAGE,
            ERR_ACCESS_DENIED_MESSAGE,
            ERR_ALREADY_EXISTS_MESSAGE,
            ERR_DIRECTORY_NOT_EMPTY_MESSAGE,
            ERR_OUTPUT_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
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
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_NOT_FOUND_MESSAGE,
            ERR_ACCESS_DENIED_MESSAGE,
            ERR_ALREADY_EXISTS_MESSAGE,
            ERR_OUTPUT_MESSAGE,
            ERR_RESOURCE_CLOSED_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
        }
    }
    if module_uses_call(module, "term.terminalSize") {
        push_string_value(&mut values, ERR_UNSUPPORTED_MESSAGE.to_string());
    }
    if module_uses_any_call(
        module,
        &[
            "net.lookup",
            "net.connectTcp",
            "net.listenTcp",
            "net.accept",
            "net.poll",
            "net.read",
            "net.readText",
            "net.write",
            "net.writeText",
            "net.close",
            "net.localAddress",
            "net.remoteAddress",
            "net.setReadTimeout",
            "net.setWriteTimeout",
            "net.bindUdp",
            "net.receiveFrom",
            "net.receiveTextFrom",
            "net.sendTo",
            "net.sendTextTo",
        ],
    ) {
        for value in [
            ERR_ADDRESS_INVALID_MESSAGE,
            ERR_ADDRESS_NOT_FOUND_MESSAGE,
            ERR_NETWORK_FAILED_MESSAGE,
            ERR_CONNECTION_CLOSED_MESSAGE,
            ERR_READ_TIMEOUT_MESSAGE,
            ERR_WRITE_TIMEOUT_MESSAGE,
            ERR_MESSAGE_TOO_LARGE_MESSAGE,
            ERR_RESOURCE_CLOSED_MESSAGE,
            ERR_CLOSE_FAILED_MESSAGE,
            ERR_ENCODING_MESSAGE,
            ERR_TIMEOUT_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
        }
    }
    // `crypto::randomBytes` fails `ErrInvalidArgument` on a negative count and
    // `ErrUnknown` on an (essentially unreachable) OS-entropy failure
    // (plan-04-crypto.md §A.6).
    if module_uses_call(module, "crypto.randomBytes") {
        for value in [ERR_INVALID_ARGUMENT_MESSAGE, ERR_UNKNOWN_MESSAGE] {
            push_string_value(&mut values, value.to_string());
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
            "tls.readText",
            "tls.write",
            "tls.writeText",
            "tls.close",
            "tls.closeListener",
        ],
    ) {
        for value in [
            ERR_TLS_FAILED_MESSAGE,
            ERR_ADDRESS_INVALID_MESSAGE,
            ERR_ADDRESS_NOT_FOUND_MESSAGE,
            ERR_NETWORK_FAILED_MESSAGE,
            ERR_CONNECTION_CLOSED_MESSAGE,
            ERR_RESOURCE_CLOSED_MESSAGE,
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_ENCODING_MESSAGE,
            ERR_TIMEOUT_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
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
            ERR_AUDIO_UNAVAILABLE_MESSAGE,
            ERR_AUDIO_DEVICE_MESSAGE,
            ERR_INVALID_ARGUMENT_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
        }
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
        push_string_value(&mut values, ERR_INDEX_OUT_OF_RANGE_MESSAGE.to_string());
    }
    if module_uses_migrated(module, "find") || module_uses_migrated(module, "get") {
        push_string_value(&mut values, ERR_NOT_FOUND_MESSAGE.to_string());
    }
    if module_uses_call(module, "toString") {
        push_string_value(&mut values, "TRUE".to_string());
        push_string_value(&mut values, "FALSE".to_string());
        push_string_value(&mut values, ERR_ENCODING_MESSAGE.to_string());
    }
    for value in [ENTRY_ERROR_PREFIX, ENTRY_ERROR_NEWLINE] {
        if !values.contains(&value.to_string()) {
            values.push(value.to_string());
        }
    }
    if module_may_record_cleanup_failure(module) {
        if !values.contains(&CLEANUP_FAILURE_PREFIX.to_string()) {
            values.push(CLEANUP_FAILURE_PREFIX.to_string());
        }
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
            push_string_value(values, field.type_.clone());
        }
        for variant in &type_.variants {
            push_string_value(values, variant.name.clone());
            for field in &variant.fields {
                push_string_value(values, field.type_.clone());
            }
        }
    }
    for function in &module.functions {
        push_string_value(values, function.returns.clone());
        for param in &function.params {
            push_string_value(values, param.type_.clone());
        }
        collect_type_name_values_from_ops(&function.body, values);
    }
}

fn collect_type_name_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    for op in ops {
        match op {
            NirOp::Bind { type_, value, .. } => {
                push_string_value(values, type_.clone());
                if let Some(value) = value {
                    collect_type_name_values_from_value(value, values);
                }
            }
            NirOp::StoreGlobal { type_, value, .. } => {
                if !type_.is_empty() {
                    push_string_value(values, type_.clone());
                }
                if let Some(value) = value {
                    collect_type_name_values_from_value(value, values);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_type_name_values_from_value(value, values);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => collect_type_name_values_from_value(code, values),
            NirOp::Fail { error } => collect_type_name_values_from_value(error, values),
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value } => {
                collect_type_name_values_from_value(value, values);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_type_name_values_from_value(condition, values);
                collect_type_name_values_from_ops(then_body, values);
                collect_type_name_values_from_ops(else_body, values);
            }
            NirOp::Match { value, cases } => {
                collect_type_name_values_from_value(value, values);
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        collect_type_name_values_from_value(value, values);
                    }
                    if let Some(guard) = &case.guard {
                        collect_type_name_values_from_value(guard, values);
                    }
                    collect_type_name_values_from_ops(&case.body, values);
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_type_name_values_from_value(condition, values);
                collect_type_name_values_from_ops(body, values);
            }
            NirOp::For {
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                push_string_value(values, type_.clone());
                collect_type_name_values_from_value(start, values);
                collect_type_name_values_from_value(end, values);
                collect_type_name_values_from_value(step, values);
                collect_type_name_values_from_ops(body, values);
            }
            NirOp::DoUntil { body, condition } => {
                collect_type_name_values_from_ops(body, values);
                collect_type_name_values_from_value(condition, values);
            }
            NirOp::ForEach {
                type_,
                iterable,
                body,
                ..
            } => {
                push_string_value(values, type_.clone());
                collect_type_name_values_from_value(iterable, values);
                collect_type_name_values_from_ops(body, values);
            }
            NirOp::Trap { body, .. } => {
                collect_type_name_values_from_ops(body, values);
            }
        }
    }
}

fn collect_type_name_values_from_value(value: &NirValue, values: &mut Vec<String>) {
    match value {
        NirValue::Const { type_, .. }
        | NirValue::FunctionRef { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => {
            push_string_value(values, type_.clone());
        }
        NirValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => {
            push_string_value(values, union_type.clone());
            push_string_value(values, member_type.clone());
            collect_type_name_values_from_value(value, values);
        }
        NirValue::UnionExtract { type_, value } => {
            push_string_value(values, type_.clone());
            collect_type_name_values_from_value(value, values);
        }
        _ => {}
    }
    match value {
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_type_name_values_from_value(arg, values);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => collect_type_name_values_from_value(value, values),
        NirValue::WithUpdate {
            type_,
            target,
            updates,
        } => {
            push_string_value(values, type_.clone());
            collect_type_name_values_from_value(target, values);
            for update in updates {
                collect_type_name_values_from_value(&update.value, values);
            }
        }
        NirValue::ListLiteral { values: items, .. } => {
            for item in items {
                collect_type_name_values_from_value(item, values);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_type_name_values_from_value(key, values);
                collect_type_name_values_from_value(value, values);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_type_name_values_from_value(target, values)
        }
        NirValue::Binary { left, right, .. } => {
            collect_type_name_values_from_value(left, values);
            collect_type_name_values_from_value(right, values);
        }
        NirValue::Unary { operand, .. } => collect_type_name_values_from_value(operand, values),
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_type_name_values_from_value(value, values);
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

pub(super) fn unicode_string_call_is_static(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    matches!(
        target,
        "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.graphemes"
    ) && args.len() == 1
        && static_string_value_with_constants(&args[0], constants, types).is_some()
}

pub(super) fn unicode_runtime_data_objects() -> Vec<CodeDataObject> {
    let tables = crate::unicode_runtime_tables::tables();
    vec![
        raw_data_object(
            UNICODE_STAGE1_SYMBOL,
            "u16 utf8proc stage1 property index table",
            tables.stage1.len() * 2,
            crate::unicode_runtime_tables::stage1_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_STAGE2_SYMBOL,
            "u16 utf8proc stage2 property index table",
            tables.stage2.len() * 2,
            crate::unicode_runtime_tables::stage2_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_PROPERTIES_SYMBOL,
            "mfb.unicode.property.v1 records, 24 bytes each",
            tables.properties.len() * 24,
            crate::unicode_runtime_tables::properties_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_SEQUENCES_SYMBOL,
            "u16 utf8proc sequence table",
            tables.sequences.len() * 2,
            crate::unicode_runtime_tables::sequences_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_COMBINATIONS_SECOND_SYMBOL,
            "u32 utf8proc composition second codepoint table",
            tables.combinations_second.len() * 4,
            crate::unicode_runtime_tables::combinations_second_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_COMBINATIONS_COMBINED_SYMBOL,
            "u32 utf8proc composition combined codepoint table",
            tables.combinations_combined.len() * 4,
            crate::unicode_runtime_tables::combinations_combined_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_NFD_ENTRIES_SYMBOL,
            "mfb.unicode.nfd_entry.v1 records, 16 bytes each",
            tables.nfd_entries.len() * 16,
            crate::unicode_runtime_tables::nfd_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_NFD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode NFD sequence table",
            tables.nfd_sequences.len() * 4,
            crate::unicode_runtime_tables::nfd_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_UPPERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 uppercase records, 16 bytes each",
            tables.uppercase_entries.len() * 16,
            crate::unicode_runtime_tables::uppercase_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_UPPERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode uppercase sequence table",
            tables.uppercase_sequences.len() * 4,
            crate::unicode_runtime_tables::uppercase_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_LOWERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 lowercase records, 16 bytes each",
            tables.lowercase_entries.len() * 16,
            crate::unicode_runtime_tables::lowercase_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_LOWERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode lowercase sequence table",
            tables.lowercase_sequences.len() * 4,
            crate::unicode_runtime_tables::lowercase_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_CASEFOLD_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 casefold records, 16 bytes each",
            tables.casefold_entries.len() * 16,
            crate::unicode_runtime_tables::casefold_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_CASEFOLD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode casefold sequence table",
            tables.casefold_sequences.len() * 4,
            crate::unicode_runtime_tables::casefold_sequences_hex(),
            4,
        ),
    ]
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

fn collect_string_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    let mut constants = HashMap::new();
    let mut types = HashMap::new();
    collect_string_values_from_ops_with_constants(ops, values, &mut constants, &mut types);
}

fn collect_string_values_from_ops_with_constants(
    ops: &[NirOp],
    values: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, String>,
) {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types);
                    if let Some(constant) =
                        local_constant_value_with_constants(value, constants, types)
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
                    collect_string_values_from_value(value, values, constants, types);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_string_values_from_value(code, values, constants, types);
            }
            NirOp::Fail { error } => {
                collect_string_values_from_value(error, values, constants, types);
            }
            NirOp::StateAssign { value, .. } => {
                collect_string_values_from_value(value, values, constants, types);
            }
            NirOp::Assign { name, value } => {
                collect_string_values_from_value(value, values, constants, types);
                if let Some(constant) = local_constant_value_with_constants(value, constants, types)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Eval { value } => {
                collect_string_values_from_value(value, values, constants, types);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_string_values_from_value(condition, values, constants, types);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    then_body,
                    values,
                    &mut then_constants,
                    &mut then_types,
                );
                collect_string_values_from_ops_with_constants(
                    else_body,
                    values,
                    &mut else_constants,
                    &mut else_types,
                );
            }
            NirOp::Match { value, cases } => {
                collect_string_values_from_value(value, values, constants, types);
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        collect_string_values_from_value(value, values, constants, types);
                    }
                    // A guard is a value expression that may hold string
                    // literals; without walking it, `fs::exists("/tmp/x")` in a
                    // `WHEN` guard has no data object at codegen (bug-118).
                    if let Some(guard) = &case.guard {
                        collect_string_values_from_value(guard, values, constants, types);
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    collect_string_values_from_ops_with_constants(
                        &case.body,
                        values,
                        &mut case_constants,
                        &mut case_types,
                    );
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_string_values_from_value(condition, values, constants, types);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
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
                collect_string_values_from_value(start, values, constants, types);
                collect_string_values_from_value(end, values, constants, types);
                collect_string_values_from_value(step, values, constants, types);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
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
                );
                collect_string_values_from_value(condition, values, constants, types);
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                collect_string_values_from_value(iterable, values, constants, types);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
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
                );
            }
        }
    }
}

fn collect_string_values_from_value(
    value: &NirValue,
    values: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) {
    if let Some(value) = static_string_value_with_constants(value, constants, types) {
        push_string_value(values, value);
    }
    if let NirValue::Call { target, args, .. }
    | NirValue::CallResult { target, args, .. }
    | NirValue::RuntimeCall { target, args, .. } = value
    {
        if target == "strings.graphemes" && args.len() == 1 {
            if let Some(value) = static_string_value_with_constants(&args[0], constants, types) {
                for grapheme in crate::unicode_backend::graphemes(&value) {
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
    if value_may_return_invalid_format(value, constants, types) {
        push_string_value(values, ERR_INVALID_FORMAT_MESSAGE.to_string());
    }
    match value {
        NirValue::Const { type_, value } if type_ == "String" => {
            push_string_value(values, value.clone());
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_string_values_from_value(arg, values, constants, types);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_string_values_from_value(value, values, constants, types)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_string_values_from_value(target, values, constants, types);
            for update in updates {
                collect_string_values_from_value(&update.value, values, constants, types);
            }
        }
        NirValue::ListLiteral { values: items, .. } => {
            for item in items {
                collect_string_values_from_value(item, values, constants, types);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_string_values_from_value(key, values, constants, types);
                collect_string_values_from_value(value, values, constants, types);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_string_values_from_value(target, values, constants, types)
        }
        NirValue::Binary { left, right, .. } => {
            collect_string_values_from_value(left, values, constants, types);
            collect_string_values_from_value(right, values, constants, types);
        }
        NirValue::Unary { operand, .. } => {
            collect_string_values_from_value(operand, values, constants, types)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_string_values_from_value(value, values, constants, types);
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

pub(super) fn static_string_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| static_string_value_with_constants(constant, constants, types)),
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
            static_type_name_with_types(&args[0], types)
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            strings_package_static_string_value(target, args, constants, types)
        }
        NirValue::Binary {
            op, left, right, ..
        } if op == "&" => {
            let left = static_string_value_with_constants(left, constants, types)?;
            let right = static_string_value_with_constants(right, constants, types)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

pub(super) fn static_type_name_with_types(
    value: &NirValue,
    types: &HashMap<String, String>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => types.get(name).cloned(),
        NirValue::LocalRef { type_, .. } => Some(type_.clone()),
        NirValue::Global { type_, .. } if !type_.is_empty() => Some(type_.clone()),
        NirValue::Global { .. } => None,
        NirValue::FunctionRef { type_, .. }
        | NirValue::Closure { type_, .. }
        | NirValue::Capture { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::WithUpdate { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::UnionWrap { union_type, .. } => Some(union_type.clone()),
        NirValue::UnionExtract { type_, .. } => Some(type_.clone()),
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => match target.as_str() {
            "typeName" | "toString" => Some("String".to_string()),
            "len" | "toInt" => Some("Integer".to_string()),
            // Migrated find/mid/replace: strings:: returns Integer/String; the
            // collections:: List overloads return the list type and are resolved
            // by the precise type path, so only `find` (always Integer) is mapped
            // here (plan-01-functions.md §5).
            "collections.find" | "strings.find" => Some("Integer".to_string()),
            "strings.mid" | "strings.replace" => Some("String".to_string()),
            "strings.trim"
            | "strings.trimStart"
            | "strings.trimEnd"
            | "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.join" => Some("String".to_string()),
            "strings.graphemes" | "strings.split" => Some("List OF String".to_string()),
            "strings.startsWith" | "strings.endsWith" | "strings.contains" => {
                Some("Boolean".to_string())
            }
            "strings.byteLen" => Some("Integer".to_string()),
            "toFloat" => Some("Float".to_string()),
            "toFixed" => Some("Fixed".to_string()),
            "toByte" => Some("Byte".to_string()),
            "toMoney" => Some("Money".to_string()),
            "toScalar" => Some("Scalar".to_string()),
            "isNumeric" => Some("Boolean".to_string()),
            _ => None,
        },
        NirValue::ResultIsOk { .. } => Some("Boolean".to_string()),
        NirValue::ResultValue { value } => static_type_name_with_types(value, types)
            .and_then(|type_| type_.strip_prefix("Result OF ").map(str::to_string))
            .or_else(|| static_type_name_with_types(value, types)),
        NirValue::ResultError { .. } => Some("Error".to_string()),
        NirValue::Binary {
            op, left, right, ..
        } => {
            if matches!(
                op.as_str(),
                "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
            ) {
                return Some("Boolean".to_string());
            }
            if op == "&" {
                return Some("String".to_string());
            }
            let left = static_type_name_with_types(left, types)?;
            let right = static_type_name_with_types(right, types)?;
            Some(numeric_binary_result_type(op, &left, &right).to_string())
        }
        NirValue::Unary { op, operand, .. } => {
            if op == "NOT" {
                Some("Boolean".to_string())
            } else {
                static_type_name_with_types(operand, types)
            }
        }
        NirValue::MemberAccess { target, member } => {
            let target_type = static_type_name_with_types(target, types)?;
            if member == "result" {
                if let Some(output_type) = builtins::thread::parent_thread_output(&target_type) {
                    return Some(format!("Result OF {output_type}"));
                }
            }
            let (key_type, value_type) = parse_map_entry_type(&target_type)?;
            match member.as_str() {
                "key" => Some(key_type),
                "value" => Some(value_type),
                _ => None,
            }
        }
    }
}

pub(super) fn builtin_function_symbol_for_type(name: &str, type_: &str) -> Option<String> {
    builtins::general::builtin_function_id_for_type(name, type_)?;
    Some(format!(
        "_mfb_builtin_{}_{}",
        nir::symbol_fragment(name),
        nir::symbol_fragment(type_)
    ))
}

pub(super) fn builtin_function_refs(module: &NirModule) -> Vec<(String, String, String)> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for function in &module.functions {
        collect_builtin_function_refs_in_ops(&function.body, &mut refs, &mut seen);
    }
    refs
}

fn collect_builtin_function_refs_in_ops(
    ops: &[NirOp],
    refs: &mut Vec<(String, String, String)>,
    seen: &mut HashSet<String>,
) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_builtin_function_refs_in_value(value, refs, seen);
                }
            }
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value }
            | NirOp::Fail { error: value } => {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_builtin_function_refs_in_value(value, refs, seen);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_builtin_function_refs_in_value(code, refs, seen);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_builtin_function_refs_in_value(condition, refs, seen);
                collect_builtin_function_refs_in_ops(then_body, refs, seen);
                collect_builtin_function_refs_in_ops(else_body, refs, seen);
            }
            NirOp::Match { value, cases } => {
                collect_builtin_function_refs_in_value(value, refs, seen);
                for case in cases {
                    if let NirMatchPattern::Value(pattern) = &case.pattern {
                        collect_builtin_function_refs_in_value(pattern, refs, seen);
                    }
                    if let Some(guard) = &case.guard {
                        collect_builtin_function_refs_in_value(guard, refs, seen);
                    }
                    collect_builtin_function_refs_in_ops(&case.body, refs, seen);
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_builtin_function_refs_in_value(condition, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_builtin_function_refs_in_value(start, refs, seen);
                collect_builtin_function_refs_in_value(end, refs, seen);
                collect_builtin_function_refs_in_value(step, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
            NirOp::DoUntil { body, condition } => {
                collect_builtin_function_refs_in_ops(body, refs, seen);
                collect_builtin_function_refs_in_value(condition, refs, seen);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_builtin_function_refs_in_value(iterable, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
            NirOp::Trap { body, .. } => {
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
        }
    }
}

fn collect_builtin_function_refs_in_value(
    value: &NirValue,
    refs: &mut Vec<(String, String, String)>,
    seen: &mut HashSet<String>,
) {
    match value {
        NirValue::FunctionRef { name, type_ } => {
            if let Some(symbol) = builtin_function_symbol_for_type(name, type_) {
                let key = format!("{name}\0{type_}");
                if seen.insert(key) {
                    refs.push((name.clone(), type_.clone(), symbol));
                }
            }
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. } => {
            for arg in args {
                collect_builtin_function_refs_in_value(arg, refs, seen);
            }
        }
        NirValue::Constructor { args, .. } => {
            for value in args {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_builtin_function_refs_in_value(value, refs, seen);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_builtin_function_refs_in_value(target, refs, seen);
            for update in updates {
                collect_builtin_function_refs_in_value(&update.value, refs, seen);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_builtin_function_refs_in_value(key, refs, seen);
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::Binary { left, right, .. } => {
            collect_builtin_function_refs_in_value(left, refs, seen);
            collect_builtin_function_refs_in_value(right, refs, seen);
        }
        NirValue::Unary { operand, .. } => {
            collect_builtin_function_refs_in_value(operand, refs, seen);
        }
        NirValue::MemberAccess { target, .. } => {
            collect_builtin_function_refs_in_value(target, refs, seen);
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. } => {}
    }
}
