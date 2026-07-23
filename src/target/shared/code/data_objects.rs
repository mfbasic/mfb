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
        // `ErrResourceMoved` rides the SAME closed-guard as `ErrResourceClosed`
        // (both bits live in the offset-8 word, and the guard splits them only at
        // the report), so wherever the closed message is registered the moved one
        // must be too — plan-52-B. Registering the string is what emits its
        // `_mfb_str_error_resource_moved` data object; miss one and the reference
        // the guard already emitted dangles at link time (the bug-256 class:
        // `net::` programs link no `_mfb_rt_fs_*`/`_mfb_rt_thread_*` symbol, so
        // they do not get the whole standard set for free and failed with
        // "relocation target '_mfb_str_error_resource_moved' is not a data object").
        push_string_value(&mut values, ERR_RESOURCE_MOVED_MESSAGE.to_string());
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
        &[
            "os.hostName",
            "os.userName",
            "os.executablePath",
            // plan-55-B: `os.resourcePath` raises ErrUnsupported when the exe path
            // cannot be acquired (the same failure `executablePath` handles).
            "os.resourcePath",
        ],
    ) {
        push_string_value(&mut values, ERR_UNSUPPORTED_MESSAGE.to_string());
    }
    // plan-55-B: `os.resourcePath` additionally raises ErrInvalidPath when the
    // `relative` argument contains a `.`/`..` path component.
    if module_uses_call(module, "os.resourcePath") {
        push_string_value(&mut values, ERR_INVALID_PATH_MESSAGE.to_string());
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
            ERR_RESOURCE_MOVED_MESSAGE,
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
            ERR_RESOURCE_MOVED_MESSAGE,
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
            ERR_RESOURCE_MOVED_MESSAGE,
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
    use nir::visit::{walk_op, walk_value, NirVisitor};
    struct Collector<'a> {
        values: &'a mut Vec<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { type_, .. } => push_string_value(self.values, type_.clone()),
                NirOp::StoreGlobal { type_, .. } if !type_.is_empty() => {
                    push_string_value(self.values, type_.clone())
                }
                NirOp::For { type_, .. } | NirOp::ForEach { type_, .. } => {
                    push_string_value(self.values, type_.clone())
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
                | NirValue::MapLiteral { type_, .. }
                | NirValue::UnionExtract { type_, .. }
                | NirValue::WithUpdate { type_, .. } => {
                    push_string_value(self.values, type_.clone())
                }
                NirValue::UnionWrap {
                    union_type,
                    member_type,
                    ..
                } => {
                    push_string_value(self.values, union_type.clone());
                    push_string_value(self.values, member_type.clone());
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }
    Collector { values }.visit_ops(ops);
}

pub(super) fn unicode_string_call_is_static(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
    fields: &FieldTypes,
) -> bool {
    matches!(
        target,
        "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.graphemes"
    ) && args.len() == 1
        && static_string_value_with_constants(&args[0], constants, types, fields).is_some()
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
    let mut types: HashMap<String, String> = function
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
    types: &mut HashMap<String, String>,
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
    types: &HashMap<String, String>,
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
    if value_may_return_invalid_format(value, constants, types, fields) {
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
        NirValue::ListLiteral { values: items, .. } => {
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

pub(super) fn static_string_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
    fields: &FieldTypes,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
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
            static_type_name_for_fold_with_types(&args[0], types, fields)
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

pub(super) fn static_type_name_with_types(
    value: &NirValue,
    types: &HashMap<String, String>,
    fields: &FieldTypes,
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
        NirValue::ResultValue { value } => static_type_name_with_types(value, types, fields)
            .and_then(|type_| type_.strip_prefix("Result OF ").map(str::to_string))
            .or_else(|| static_type_name_with_types(value, types, fields)),
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
            let left = static_type_name_with_types(left, types, fields)?;
            let right = static_type_name_with_types(right, types, fields)?;
            Some(numeric_binary_result_type(op, &left, &right).to_string())
        }
        NirValue::Unary { op, operand, .. } => {
            if op == "NOT" {
                Some("Boolean".to_string())
            } else {
                static_type_name_with_types(operand, types, fields)
            }
        }
        NirValue::MemberAccess { target, member } => {
            let target_type = static_type_name_with_types(target, types, fields)?;
            if member == "result" {
                if let Some(output_type) = builtins::thread::parent_thread_output(&target_type) {
                    return Some(format!("Result OF {output_type}"));
                }
            }
            // Record and union-variant fields, then the two `MapEntry` members —
            // the same sources `static_nir_value_type` consults. Without the
            // field table this arm answered `None` for every record field, which
            // silently under-reported in every predicate built on this seam:
            // `typeName(rec.field)` failed to lower at all, and the
            // ERR_INVALID_FORMAT gate missed a promoting Float operand (bug-366).
            if let Some(field_type) = fields.get(&(target_type.clone(), member.clone())) {
                return Some(field_type.clone());
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

/// The pre-pass twin of [`super::CodeBuilder::static_type_name_for_fold`]: static
/// type of `value`, resolving builtin calls that [`static_type_name_with_types`]'s
/// hand-written table misses via `builtins::resolve_call_return_type`.
///
/// Used **only** for the `typeName` compile-time fold (bug-354), where the pre-pass
/// interns the folded type-name string the builder later looks up — so this must
/// agree with the builder's `static_type_name_for_fold`, and both delegate to the
/// same resolver. It does NOT widen `static_type_name_with_types`, whose other
/// consumers (the float-numeric-error gate, module analysis, binary typing) must
/// keep their exact current answers.
pub(super) fn static_type_name_for_fold_with_types(
    value: &NirValue,
    types: &HashMap<String, String>,
    fields: &FieldTypes,
) -> Option<String> {
    if let Some(type_name) = static_type_name_with_types(value, types, fields) {
        return Some(type_name);
    }
    match value {
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            let arg_types = args
                .iter()
                .map(|arg| static_type_name_for_fold_with_types(arg, types, fields))
                .collect::<Option<Vec<_>>>()?;
            builtins::resolve_call_return_type(target, &arg_types)
        }
        _ => None,
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
    use nir::visit::{walk_value, NirVisitor};
    struct Collector<'a> {
        refs: &'a mut Vec<(String, String, String)>,
        seen: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::FunctionRef { name, type_ } = value {
                if let Some(symbol) = builtin_function_symbol_for_type(name, type_) {
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
    use std::collections::HashMap;

    fn const_of(type_: &str) -> NirValue {
        NirValue::Const {
            type_: type_.to_string(),
            value: String::new(),
        }
    }

    fn call(target: &str, arg_types: &[&str]) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args: arg_types.iter().map(|t| const_of(t)).collect(),
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
            ("collections.hasKey", &["Map OF String TO Integer", "String"]),
            // general.* contrast cases (already resolved before the fix).
            ("toString", &["Integer"]),
            ("toInt", &["String"]),
            ("toFloat", &["String"]),
            ("isNumeric", &["String"]),
            ("typeName", &["String"]),
        ];
        for (target, arg_types) in catalog {
            let want = builtins::resolve_call_return_type(
                target,
                &arg_types.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            );
            let got = static_type_name_for_fold_with_types(&call(target, arg_types), &types, &fields);
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
}
