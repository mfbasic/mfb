use super::*;

pub(super) fn lower_project(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
) -> Result<BinaryReprProject, String> {
    lower_project_with_external_functions(
        ir,
        metadata,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
}

pub(super) fn lower_package_project(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    package_paths: &[PathBuf],
) -> Result<BinaryReprProject, String> {
    let packages = package_paths
        .iter()
        .map(|path| read_package_binary_repr(path))
        .collect::<Result<Vec<_>, _>>()?;
    let (external_function_ids, external_function_returns, external_function_abi_hashes) =
        external_function_metadata(ir.functions.len() as u32, &packages)?;
    lower_project_with_external_functions(
        ir,
        metadata,
        &external_function_ids,
        &external_function_returns,
        &external_function_abi_hashes,
    )
}

pub(super) fn external_function_metadata(
    base_function_id: u32,
    packages: &[PackageBinaryRepr],
) -> Result<
    (
        HashMap<String, u32>,
        HashMap<String, String>,
        HashMap<String, [u8; ABI_HASH_LEN]>,
    ),
    String,
> {
    let mut external_function_ids = HashMap::new();
    let mut external_function_returns = HashMap::new();
    let mut external_function_abi_hashes = HashMap::new();
    let mut next_function_id = base_function_id;
    for package in packages {
        let package_name = string_at(
            &package.project.strings.values,
            package.project.manifest.package_name,
        )?;
        let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
        for export in &package.exports {
            let function = package
                .project
                .functions
                .get(export.function_id as usize)
                .ok_or_else(|| {
                    format!("export references missing function {}", export.function_id)
                })?;
            let export_name = string_at(&package.project.strings.values, export.name)?;
            // `export.function_id` is decoded from an attacker-influenced `.mfp`;
            // use checked_add (like the `next_function_id` bump below) so an
            // overflow errors instead of silently wrapping (bug-215).
            let global_function_id = next_function_id
                .checked_add(export.function_id)
                .ok_or_else(|| "merged binary representation has too many functions".to_string())?;
            external_function_ids
                .insert(format!("{package_name}.{export_name}"), global_function_id);
            external_function_returns.insert(
                format!("{package_name}.{export_name}"),
                type_name(&type_names, function.return_type)?.to_string(),
            );
            let abi_export =
                abi_export_for_decoded(&package.project.abi, export).ok_or_else(|| {
                    format!("ABI_INDEX is missing EXPORT_TABLE entry `{export_name}`")
                })?;
            external_function_abi_hashes
                .insert(format!("{package_name}.{export_name}"), abi_export.sig_hash);
        }
        next_function_id = next_function_id
            .checked_add(package.project.functions.len() as u32)
            .ok_or_else(|| "merged binary representation has too many functions".to_string())?;
    }
    Ok((
        external_function_ids,
        external_function_returns,
        external_function_abi_hashes,
    ))
}

pub(super) fn lower_project_with_external_functions(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    external_function_ids: &HashMap<String, u32>,
    external_function_returns: &HashMap<String, String>,
    external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
) -> Result<BinaryReprProject, String> {
    let mut strings = StringPool::new();
    let ident = if metadata.ident.is_empty() {
        &metadata.name
    } else {
        &metadata.ident
    };
    let manifest = BinaryReprManifest {
        package_name: strings.intern(&metadata.name),
        package_ident: strings.intern(ident),
        package_version: strings.intern(&metadata.version),
        ident_key: strings.intern(&metadata.ident_key),
        ident_fingerprint: strings.intern(&metadata.ident_fingerprint),
        signing_fingerprint: strings.intern(&metadata.signing_fingerprint),
        author: strings.intern(&metadata.author),
        url: strings.intern(&metadata.url),
        // Derived at encode time from the tables themselves (`encode_manifest`),
        // not from these fields; they exist so the decoder can cross-check a
        // manifest against its own tables (bug-282 B4).
        dependency_count: 0,
        export_count: 0,
    };
    let mut imports = ImportTable::from_metadata(&mut strings, metadata);

    let mut types = TypeTable::new();
    for ir_type in &ir.types {
        types.reserve_source_type(&mut strings, &metadata.name, ir_type);
    }
    types.populate_source_payloads(&mut strings, &ir.types)?;
    let mut resources = ResourceTable::new();
    if ir_uses_resource_type(ir) {
        let mut used = HashSet::new();
        collect_resource_type_names(ir, &mut used);
        if used.contains("File") {
            resources.add_standard_file(&mut types, &mut strings);
        }
        if used.contains("Socket") {
            resources.add_standard_socket(&mut types, &mut strings);
        }
        if used.contains("Listener") {
            resources.add_standard_listener(&mut types, &mut strings);
        }
    }
    // Native LINK resources (plan-link-update.md §10): each becomes an opaque
    // type (exported when the declaration is `EXPORT`) plus a RESOURCE_TABLE
    // entry whose close op is referenced by name.
    for native in &ir.native_resources {
        // An opaque native resource has no fields; encode it as a zero-field
        // record so the type table round-trips. Its resource-ness (which blocks
        // construction and field access) comes from the RESOURCE_TABLE.
        let mut payload = Vec::new();
        put_u32(&mut payload, 0);
        let type_id = types.add_entry(&mut strings, &metadata.name, &native.name, 1, payload);
        if native.visibility == "export" {
            let index = (type_id - FIRST_TABLE_TYPE_ID) as usize;
            types.entries[index].abi_export_kind = Some(BinaryReprExportKind::Type);
        }
        resources.add_native(&mut strings, type_id, native);
    }
    let globals = ir
        .bindings
        .iter()
        .map(|binding| {
            let mut flags = 0;
            if binding.mutable {
                flags |= 1;
            }
            flags |= match binding.visibility.as_str() {
                "private" => 0 << 1,
                "public" => 1 << 1,
                "export" => 2 << 1,
                _ => 0,
            };
            GlobalEntry {
                name: strings.intern(&binding.name),
                type_id: types.type_id(&mut strings, &binding.type_),
                flags,
            }
        })
        .collect::<Vec<_>>();

    let mut constants = ConstPool::new();
    let mut function_ids = HashMap::new();
    for (index, function) in ir.functions.iter().enumerate() {
        function_ids.insert(function.name.clone(), index as u32);
        // The return type is interned even though the map that once held it was
        // never read (bug-100): the interning still fixes the string/type table
        // order, so the emitted bytes stay byte-identical.
        let _ = types.type_id(&mut strings, &function.returns);
    }
    for (name, id) in external_function_ids {
        function_ids.insert(name.clone(), *id);
    }
    for return_type_name in external_function_returns.values() {
        // Interning kept for table-order stability; the result is unused (bug-100).
        let _ = types.type_id(&mut strings, return_type_name);
    }

    let mut functions = Vec::new();
    let mut used_imported_functions = HashSet::new();
    for function in &ir.functions {
        functions.push(lower_function(
            function,
            &mut strings,
            &mut types,
            &mut constants,
            external_function_abi_hashes,
            &mut used_imported_functions,
        )?);
    }
    imports.record_used_imports(
        &mut strings,
        &used_imported_functions,
        external_function_abi_hashes,
    );
    let abi = AbiIndex::from_project(&strings, &types, &constants, &imports, &functions)?;

    let (entry_function, entry_flags) = if let Some(entry) = &ir.entry {
        let function_id = *function_ids.get(&entry.name).ok_or_else(|| {
            format!(
                "entry function `{}` was not lowered to binary representation",
                entry.name
            )
        })?;
        let mut flags = 1;
        if entry.accepts_args {
            flags |= 1 << 1;
        }
        if entry.returns == "Integer" {
            flags |= 1 << 2;
        }
        (function_id, flags)
    } else {
        (u32::MAX, 0)
    };

    Ok(BinaryReprProject {
        strings,
        types,
        constants,
        resources,
        globals,
        manifest,
        imports,
        abi,
        entry_function,
        entry_flags,
        functions,
        binary_repr: crate::ir::encode_binary_repr(ir),
        docs: docs_from_ir(&ir.docs),
        // Assembled by the build path from the manifest's `libraries` section and
        // the IR's `LINK` names (plan-46-B §4.3); empty for a non-binding package.
        native_libraries: metadata.native_libraries.clone(),
    })
}

pub(super) fn ir_uses_resource_type(ir: &IrProject) -> bool {
    ir.functions.iter().any(|function| {
        function
            .params
            .iter()
            .any(|param| is_resource_type_name(&param.type_))
            || is_resource_type_name(&function.returns)
            || ops_use_resource_type(&function.body)
    })
}

pub(super) fn ops_use_resource_type(ops: &[IrOp]) -> bool {
    ops.iter().any(|op| match op {
        IrOp::Bind { type_, value, .. } => {
            is_resource_type_name(type_) || value.as_ref().is_some_and(value_uses_resource_type)
        }
        IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value, .. } => value_uses_resource_type(value),
        IrOp::Return { value, .. } => value.as_ref().is_some_and(value_uses_resource_type),
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => false,
        IrOp::ExitProgram { code, .. } => value_uses_resource_type(code),
        IrOp::Fail { error, .. } => value_uses_resource_type(error),
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            value_uses_resource_type(condition)
                || ops_use_resource_type(then_body)
                || ops_use_resource_type(else_body)
        }
        IrOp::Match { value, cases, .. } => {
            value_uses_resource_type(value)
                || cases.iter().any(|case| ops_use_resource_type(&case.body))
        }
        IrOp::While {
            condition, body, ..
        } => value_uses_resource_type(condition) || ops_use_resource_type(body),
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_uses_resource_type(start)
                || value_uses_resource_type(end)
                || value_uses_resource_type(step)
                || ops_use_resource_type(body)
        }
        IrOp::DoUntil {
            body, condition, ..
        } => ops_use_resource_type(body) || value_uses_resource_type(condition),
        IrOp::ForEach {
            type_,
            iterable,
            body,
            ..
        } => {
            is_resource_type_name(type_)
                || value_uses_resource_type(iterable)
                || ops_use_resource_type(body)
        }
        IrOp::Trap { body, .. } => ops_use_resource_type(body),
    })
}

pub(super) fn value_uses_resource_type(value: &IrValue) -> bool {
    match value {
        IrValue::Const { type_, .. }
        | IrValue::LocalRef { type_, .. }
        | IrValue::FunctionRef { type_, .. }
        | IrValue::Closure { type_, .. }
        | IrValue::Capture { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. } => is_resource_type_name(type_),
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            builtins::call_return_type_name(target).is_some_and(is_resource_type_name)
                || args.iter().any(value_uses_resource_type)
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value } => value_uses_resource_type(value),
        IrValue::MemberAccess { target, .. } => value_uses_resource_type(target),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            value_uses_resource_type(target)
                || updates
                    .iter()
                    .any(|update| value_uses_resource_type(&update.value))
        }
        IrValue::Binary { left, right, .. } => {
            value_uses_resource_type(left) || value_uses_resource_type(right)
        }
        IrValue::Unary { operand, .. } => value_uses_resource_type(operand),
        IrValue::Local(_) | IrValue::Global(_) => false,
    }
}

pub(super) fn is_resource_type_name(type_name: &str) -> bool {
    builtins::is_resource_type(type_name)
}

/// Collect the bare resource type names (`File`, `Socket`, `Listener`) actually
/// referenced by the project so only the resource tables that are used get
/// emitted. Resource handles cannot appear inside collections, so resource type
/// strings are always bare names.
pub(super) fn collect_resource_type_names(ir: &IrProject, names: &mut HashSet<String>) {
    let mut record = |type_: &str, names: &mut HashSet<String>| {
        if is_resource_type_name(type_) {
            names.insert(type_.to_string());
        }
    };
    for function in &ir.functions {
        for param in &function.params {
            record(&param.type_, names);
        }
        record(&function.returns, names);
        collect_resource_names_in_ops(&function.body, names, &mut record);
    }
}

pub(super) fn collect_resource_names_in_ops(
    ops: &[IrOp],
    names: &mut HashSet<String>,
    record: &mut impl FnMut(&str, &mut HashSet<String>),
) {
    for op in ops {
        match op {
            IrOp::Bind { type_, value, .. } => {
                record(type_, names);
                if let Some(value) = value {
                    collect_resource_names_in_value(value, names, record);
                }
            }
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value, .. } => collect_resource_names_in_value(value, names, record),
            IrOp::Return { value, .. } => {
                if let Some(value) = value {
                    collect_resource_names_in_value(value, names, record);
                }
            }
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
            IrOp::ExitProgram { code, .. } => collect_resource_names_in_value(code, names, record),
            IrOp::Fail { error, .. } => collect_resource_names_in_value(error, names, record),
            IrOp::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                collect_resource_names_in_value(condition, names, record);
                collect_resource_names_in_ops(then_body, names, record);
                collect_resource_names_in_ops(else_body, names, record);
            }
            IrOp::Match { value, cases, .. } => {
                collect_resource_names_in_value(value, names, record);
                for case in cases {
                    collect_resource_names_in_ops(&case.body, names, record);
                }
            }
            IrOp::While {
                condition, body, ..
            } => {
                collect_resource_names_in_value(condition, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_resource_names_in_value(start, names, record);
                collect_resource_names_in_value(end, names, record);
                collect_resource_names_in_value(step, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::DoUntil {
                body, condition, ..
            } => {
                collect_resource_names_in_ops(body, names, record);
                collect_resource_names_in_value(condition, names, record);
            }
            IrOp::ForEach {
                type_,
                iterable,
                body,
                ..
            } => {
                record(type_, names);
                collect_resource_names_in_value(iterable, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::Trap { body, .. } => collect_resource_names_in_ops(body, names, record),
        }
    }
}

pub(super) fn collect_resource_names_in_value(
    value: &IrValue,
    names: &mut HashSet<String>,
    record: &mut impl FnMut(&str, &mut HashSet<String>),
) {
    match value {
        IrValue::Const { type_, .. }
        | IrValue::LocalRef { type_, .. }
        | IrValue::FunctionRef { type_, .. }
        | IrValue::Closure { type_, .. }
        | IrValue::Capture { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. } => record(type_, names),
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            if let Some(returns) = builtins::call_return_type_name(target) {
                record(returns, names);
            }
            for arg in args {
                collect_resource_names_in_value(arg, names, record);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value } => collect_resource_names_in_value(value, names, record),
        IrValue::MemberAccess { target, .. } => {
            collect_resource_names_in_value(target, names, record)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_resource_names_in_value(target, names, record);
            for update in updates {
                collect_resource_names_in_value(&update.value, names, record);
            }
        }
        IrValue::Binary { left, right, .. } => {
            collect_resource_names_in_value(left, names, record);
            collect_resource_names_in_value(right, names, record);
        }
        IrValue::Unary { operand, .. } => collect_resource_names_in_value(operand, names, record),
        IrValue::Local(_) | IrValue::Global(_) => {}
    }
}

/// Lower an `IrFunction` to its container *metadata* (`Function`): name, kind,
/// flags, return type, and parameter signatures. Function *bodies* are no longer
/// flattened to opcodes here — they are carried verbatim in the structured
/// Binary Representation payload (`SECTION_BINARY_REPR`). The flat `code`/`registers`/`cleanups`
/// fields are therefore empty; only the signature-level tables (function table,
/// export table, ABI index, import table) consume this metadata.
pub(super) fn lower_function(
    function: &IrFunction,
    strings: &mut StringPool,
    types: &mut TypeTable,
    constants: &mut ConstPool,
    external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used_imported_functions: &mut HashSet<String>,
) -> Result<Function, String> {
    let mut params = Vec::new();
    for param in &function.params {
        let type_id = types.type_id(strings, &param.type_);
        params.push(Param {
            name: strings.intern(&param.name),
            type_id,
            flags: if param.default.is_some() { 1 } else { 0 },
            default_const: match &param.default {
                Some(default) => constants.add(strings, default)?,
                None => u32::MAX,
            },
        });
    }

    // Record which imported (cross-package) functions this body references so the
    // import table can pin the exact used symbols. Imported targets are exactly
    // the qualified names present in `external_function_abi_hashes`.
    for op in &function.body {
        collect_imported_calls_op(op, external_function_abi_hashes, used_imported_functions);
    }

    let mut flags = if function.visibility == "export" {
        0
    } else {
        FUNCTION_FLAG_PRIVATE
    };
    if function.kind == "sub" {
        flags |= FUNCTION_FLAG_SUB | FUNCTION_FLAG_RETURNS_NOTHING;
    }
    if function.returns == "Nothing" {
        flags |= FUNCTION_FLAG_RETURNS_NOTHING;
    }
    if function.isolated {
        flags |= FUNCTION_FLAG_ISOLATED;
    }

    Ok(Function {
        name: strings.intern(&function.name),
        kind: FUNCTION_BINARY_REPR,
        flags,
        return_type: types.type_id(strings, &function.returns),
        params,
        registers: Vec::new(),
        cleanups: Vec::new(),
    })
}

/// Walk an `IrOp`, recording any call/reference target that names an imported
/// (cross-package) function into `used`.
pub(super) fn collect_imported_calls_op(
    op: &IrOp,
    imported: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used: &mut HashSet<String>,
) {
    match op {
        IrOp::Bind { value, .. } => {
            if let Some(v) = value {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value, .. }
        | IrOp::Fail { error: value, .. } => collect_imported_calls_value(value, imported, used),
        IrOp::Return { value, .. } => {
            if let Some(v) = value {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
        IrOp::ExitProgram { code, .. } => collect_imported_calls_value(code, imported, used),
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            collect_imported_calls_value(condition, imported, used);
            for op in then_body.iter().chain(else_body) {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::Match { value, cases, .. } => {
            collect_imported_calls_value(value, imported, used);
            for case in cases {
                if let Some(guard) = &case.guard {
                    collect_imported_calls_value(guard, imported, used);
                }
                for op in &case.body {
                    collect_imported_calls_op(op, imported, used);
                }
            }
        }
        IrOp::While {
            condition, body, ..
        } => {
            collect_imported_calls_value(condition, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            collect_imported_calls_value(start, imported, used);
            collect_imported_calls_value(end, imported, used);
            collect_imported_calls_value(step, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::DoUntil {
            body, condition, ..
        } => {
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
            collect_imported_calls_value(condition, imported, used);
        }
        IrOp::ForEach { iterable, body, .. } => {
            collect_imported_calls_value(iterable, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::Trap { body, .. } => {
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
    }
}

/// Walk an `IrValue`, recording imported function references into `used`.
pub(super) fn collect_imported_calls_value(
    value: &IrValue,
    imported: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used: &mut HashSet<String>,
) {
    let note = |target: &str, used: &mut HashSet<String>| {
        if imported.contains_key(target) {
            used.insert(target.to_string());
        }
    };
    match value {
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            note(target, used);
            for arg in args {
                collect_imported_calls_value(arg, imported, used);
            }
        }
        IrValue::FunctionRef { name, .. } => note(name, used),
        IrValue::Closure { name, captures, .. } => {
            note(name, used);
            for capture in captures {
                collect_imported_calls_value(capture, imported, used);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                collect_imported_calls_value(arg, imported, used);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            collect_imported_calls_value(value, imported, used)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_imported_calls_value(target, imported, used);
            for update in updates {
                collect_imported_calls_value(&update.value, imported, used);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_imported_calls_value(k, imported, used);
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrValue::Binary { left, right, .. } => {
            collect_imported_calls_value(left, imported, used);
            collect_imported_calls_value(right, imported, used);
        }
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::LocalRef { .. }
        | IrValue::Global(_)
        | IrValue::Capture { .. } => {}
    }
}

#[derive(Clone)]
pub(super) struct FunctionTypeSignature {
    pub(super) isolated: bool,
    pub(super) params: Vec<String>,
    pub(super) returns: String,
}

pub(super) fn parse_function_type(type_name: &str) -> Option<FunctionTypeSignature> {
    let (isolated, rest) = if let Some(rest) = type_name.strip_prefix("ISOLATED FUNC(") {
        (true, rest)
    } else {
        (false, type_name.strip_prefix("FUNC(")?)
    };
    let (params, returns) = split_function_type_rest(rest)?;
    Some(FunctionTypeSignature {
        isolated,
        params: split_top_level_types(params),
        returns: returns.to_string(),
    })
}

pub(super) fn split_function_type_rest(rest: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let bytes = rest.as_bytes();
    for index in 0..bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' if depth == 0 && rest[index..].starts_with(") AS ") => {
                return Some((&rest[..index], &rest[index + 5..]));
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

pub(super) fn split_top_level_types(params: &str) -> Vec<String> {
    if params.trim().is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(params[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(params[start..].trim().to_string());
    result
}

pub(super) fn source_type_payload(
    strings: &mut StringPool,
    types: &mut TypeTable,
    source_types: &HashMap<&str, &IrType>,
    ir_type: &IrType,
) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    match ir_type.kind.as_str() {
        "type" => {
            put_u32(&mut payload, ir_type.fields.len() as u32);
            for field in &ir_type.fields {
                put_field_payload(strings, types, &mut payload, field);
            }
        }
        "union" => {
            let variants = concrete_union_variants(source_types, ir_type)?;
            put_u32(&mut payload, variants.len() as u32);
            for variant in variants {
                put_u32(&mut payload, strings.intern(&variant.name));
                put_u32(&mut payload, variant.fields.len() as u32);
                for field in &variant.fields {
                    put_u32(&mut payload, strings.intern(&field.name));
                    put_u32(&mut payload, types.type_id(strings, &field.type_));
                }
            }
        }
        "enum" => {
            put_u32(&mut payload, ir_type.members.len() as u32);
            for (ordinal, member) in ir_type.members.iter().enumerate() {
                put_u32(&mut payload, strings.intern(&member.name));
                put_u32(&mut payload, ordinal as u32);
            }
        }
        _ => {}
    }
    Ok(payload)
}

pub(super) fn concrete_union_variants<'a>(
    source_types: &HashMap<&str, &'a IrType>,
    ir_type: &'a IrType,
) -> Result<Vec<&'a crate::ir::IrVariant>, String> {
    let mut variants = Vec::new();
    for include in &ir_type.includes {
        let included = source_types.get(include.as_str()).ok_or_else(|| {
            format!(
                "union `{}` includes unknown union `{include}`",
                ir_type.name
            )
        })?;
        variants.extend(concrete_union_variants(source_types, included)?);
    }
    variants.extend(ir_type.variants.iter());
    Ok(variants)
}

pub(super) fn put_field_payload(
    strings: &mut StringPool,
    types: &mut TypeTable,
    payload: &mut Vec<u8>,
    field: &crate::ir::IrField,
) {
    put_u32(payload, strings.intern(&field.name));
    put_u32(payload, types.type_id(strings, &field.type_));
    put_u32(
        payload,
        match field.visibility.as_deref() {
            Some("private") => 1,
            Some("package") => 2,
            Some("export") => 3,
            _ => 0,
        },
    );
}

pub(super) fn fixed_raw_from_decimal(value: &str) -> Result<i64, String> {
    const SCALE: i128 = 1_i128 << 32;

    let (negative, digits) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let (whole, fractional) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fractional.is_empty() {
        return Err(format!("invalid Fixed constant `{value}`"));
    }
    let mut whole_value = if whole.is_empty() {
        0_i128
    } else {
        whole
            .parse::<i128>()
            .map_err(|_| format!("invalid Fixed constant `{value}`"))?
    };
    let mut fractional_value = 0_i128;
    if !fractional.is_empty() {
        // Cap fractional accumulation at 28 digits — the 32.32 layout resolves
        // only 2^-32, so digits past that sit below one ULP and cannot change the
        // round-half-up result. This keeps `fractional_value * SCALE` inside i128
        // instead of rejecting a long literal (bug-91). Mirrors
        // `numeric::fixed_raw_from_decimal`.
        const MAX_FRACTIONAL_DIGITS: usize = 28;
        let mut denominator = 1_i128;
        for digit in fractional.bytes().take(MAX_FRACTIONAL_DIGITS) {
            if !digit.is_ascii_digit() {
                return Err(format!("invalid Fixed constant `{value}`"));
            }
            fractional_value = fractional_value * 10 + (digit - b'0') as i128;
            denominator *= 10;
        }
        for digit in fractional.bytes().skip(MAX_FRACTIONAL_DIGITS) {
            if !digit.is_ascii_digit() {
                return Err(format!("invalid Fixed constant `{value}`"));
            }
        }
        let scaled = fractional_value * SCALE;
        fractional_value = scaled / denominator;
        if (scaled % denominator) * 2 >= denominator {
            fractional_value += 1;
        }
        if fractional_value == SCALE {
            whole_value += 1;
            fractional_value = 0;
        }
    }
    let raw = whole_value
        .checked_mul(SCALE)
        .and_then(|current| current.checked_add(fractional_value))
        .ok_or_else(|| format!("Fixed constant `{value}` is out of range"))?;
    let raw = if negative { -raw } else { raw };
    i64::try_from(raw).map_err(|_| format!("Fixed constant `{value}` is out of range"))
}

/// The `RESOURCE_TABLE` flags for a standard built-in resource, including the
/// "sendable to thread" bit (bit 2) when the registry marks the type sendable.
pub(super) fn standard_resource_flags(type_name: &str) -> u32 {
    let mut flags = RESOURCE_FLAG_NATIVE | RESOURCE_FLAG_STANDARD | RESOURCE_FLAG_CLOSE_MAY_FAIL;
    if builtins::resource::is_builtin_sendable_resource_type(type_name) {
        flags |= RESOURCE_FLAG_SENDABLE;
    }
    flags
}

impl BinaryReprProject {
    pub(super) fn encode(&self) -> Vec<u8> {
        // Function bodies live in the structured Binary Representation payload, not a flat
        // code stream. The function table still records signatures/metadata, so
        // every per-function code region is zero-length.
        let code_offsets: Vec<(u64, u64)> = self.functions.iter().map(|_| (0, 0)).collect();

        // The native library table (plan-46-B §4.1) is encoded first because
        // interning its strings grows the pool, and the pool section below must be
        // encoded from the final pool. It is emitted only for a binding package
        // that declares a `LINK` block, so every non-binding package's `.mfp` —
        // and its string pool — stays byte-identical to a pre-plan-46 build.
        let mut strings = self.strings.clone();
        let native_libraries = if self.native_libraries.is_empty() {
            None
        } else {
            Some(encode_native_library_table(
                &mut strings,
                &self.native_libraries,
            ))
        };

        let mut sections = vec![
            Section::new(SECTION_MANIFEST, self.encode_manifest()),
            Section::new(SECTION_STRING_POOL, strings.encode()),
            Section::new(SECTION_TYPE_TABLE, self.types.encode()),
            Section::new(SECTION_CONST_POOL, self.constants.encode()),
            Section::new(SECTION_IMPORT_TABLE, self.imports.encode()),
            Section::new(SECTION_EXPORT_TABLE, self.encode_exports()),
            Section::new(SECTION_GLOBAL_TABLE, self.encode_globals()),
            Section::new(SECTION_FUNCTION_TABLE, self.encode_functions(&code_offsets)),
            Section::new(SECTION_BINARY_REPR, self.binary_repr.clone()),
            Section::new(SECTION_ABI_INDEX, self.abi.encode()),
        ];
        if !self.resources.entries.is_empty() {
            sections.push(Section::new(
                SECTION_RESOURCE_TABLE,
                self.resources.encode(),
            ));
        }
        // The doc section is emitted only when the package has documentation
        // (plan-09-doc.md §5); it does not affect execution or the ABI.
        if !self.docs.is_empty() {
            sections.push(Section::new(
                SECTION_DOC_TABLE,
                encode_doc_table(&self.docs),
            ));
        }
        if let Some(table) = native_libraries {
            sections.push(Section::new(SECTION_NATIVE_LIBRARY_TABLE, table));
        }

        encode_sections(&sections)
    }

    pub(super) fn encode_manifest(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.manifest.package_name);
        put_u32(&mut bytes, self.manifest.package_ident);
        put_u32(&mut bytes, self.manifest.package_version);
        put_u32(&mut bytes, self.manifest.ident_key);
        put_u32(&mut bytes, self.manifest.ident_fingerprint);
        put_u32(&mut bytes, self.manifest.signing_fingerprint);
        put_u32(&mut bytes, self.manifest.author);
        put_u32(&mut bytes, self.manifest.url);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, self.imports.entries.len() as u32);
        put_u32(&mut bytes, 0);
        put_u32(&mut bytes, self.export_count());
        put_u32(&mut bytes, self.entry_function);
        put_u32(&mut bytes, self.entry_flags);
        bytes
    }

    pub(super) fn encode_exports(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.export_count());
        for (index, function) in self.functions.iter().enumerate() {
            if !is_exported_function(function) {
                continue;
            }
            put_u32(&mut bytes, function.name);
            put_u16(
                &mut bytes,
                if function.flags & FUNCTION_FLAG_SUB != 0 {
                    2
                } else {
                    1
                },
            );
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, index as u32);
        }
        bytes
    }

    pub(super) fn encode_globals(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.globals.len() as u32);
        for global in &self.globals {
            put_u32(&mut bytes, global.name);
            put_u32(&mut bytes, global.type_id);
            put_u32(&mut bytes, global.flags);
        }
        bytes
    }

    pub(super) fn export_count(&self) -> u32 {
        self.functions
            .iter()
            .filter(|function| is_exported_function(function))
            .count() as u32
    }

    pub(super) fn encode_functions(&self, code_offsets: &[(u64, u64)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.functions.len() as u32);
        for (index, function) in self.functions.iter().enumerate() {
            let (code_offset, code_length) = code_offsets[index];
            put_u32(&mut bytes, function.name);
            put_u16(&mut bytes, function.kind);
            put_u16(&mut bytes, function.flags);
            put_u32(&mut bytes, function.params.len() as u32);
            put_u32(&mut bytes, function.return_type);
            put_u32(&mut bytes, function.registers.len() as u32);
            put_u64(&mut bytes, code_offset);
            put_u64(&mut bytes, code_length);
            put_u32(&mut bytes, u32::MAX);
            put_u32(&mut bytes, function.cleanups.len() as u32);
            let cleanup_offset =
                bytes.len() + 8 + function.params.len() * 16 + function.registers.len() * 8;
            put_u64(
                &mut bytes,
                if function.cleanups.is_empty() {
                    0
                } else {
                    cleanup_offset as u64
                },
            );

            for param in &function.params {
                put_u32(&mut bytes, param.name);
                put_u32(&mut bytes, param.type_id);
                put_u32(&mut bytes, param.flags);
                put_u32(&mut bytes, param.default_const);
            }

            for register in &function.registers {
                put_u32(&mut bytes, register.type_id);
                put_u32(&mut bytes, register.flags);
            }

            for cleanup in &function.cleanups {
                put_u32(&mut bytes, cleanup.id);
                put_u32(&mut bytes, cleanup.start_pc);
                put_u32(&mut bytes, cleanup.end_pc);
                put_u32(&mut bytes, cleanup.resource_register);
                put_u32(&mut bytes, cleanup.close_function_id);
                put_u32(&mut bytes, cleanup.flags);
            }
        }
        bytes
    }
}
