use super::*;

pub(super) fn package_resource_exports(
    package: &PackageBinaryRepr,
) -> Result<Vec<BinaryReprResourceExport>, String> {
    let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
    let mut exports = Vec::with_capacity(package.project.resources.entries.len());
    for entry in &package.project.resources.entries {
        let type_name = type_name(&type_names, entry.type_id)?.to_string();
        // A native LINK resource (NATIVE set, STANDARD clear) stores its close op
        // name directly in the string pool (plan-link-update.md §10); built-ins
        // and source resources reference a function id / sentinel.
        let close_function = if entry.flags & RESOURCE_FLAG_NATIVE != 0
            && entry.flags & RESOURCE_FLAG_STANDARD == 0
        {
            Some(string_at(&package.project.strings.values, entry.close_function_id)?.to_string())
        } else {
            resolve_resource_close_name(package, entry.close_function_id)?
        };
        exports.push(BinaryReprResourceExport {
            type_name,
            close_function,
            sendable: entry.flags & RESOURCE_FLAG_SENDABLE != 0,
            close_may_fail: entry.flags & RESOURCE_FLAG_CLOSE_MAY_FAIL != 0,
            native: entry.flags & RESOURCE_FLAG_NATIVE != 0,
        });
    }
    Ok(exports)
}

/// Resolve a `RESOURCE_TABLE` close-function id to a call name. The two built-in
/// sentinels map to the standard `fs.close`/`net.close` ops; any other id is a
/// `functionId` index into the package's function table.
pub(super) fn resolve_resource_close_name(
    package: &PackageBinaryRepr,
    close_function_id: u32,
) -> Result<Option<String>, String> {
    match close_function_id {
        BUILTIN_FS_CLOSE_FUNCTION_ID => {
            Ok(builtins::resource_close_function(builtins::fs::FILE_TYPE).map(str::to_string))
        }
        BUILTIN_NET_CLOSE_FUNCTION_ID => {
            Ok(builtins::resource_close_function(builtins::net::SOCKET_TYPE).map(str::to_string))
        }
        id => match package.project.functions.get(id as usize) {
            Some(function) => Ok(Some(
                string_at(&package.project.strings.values, function.name)?.to_string(),
            )),
            None => Ok(None),
        },
    }
}

pub(super) fn package_exports(package: &PackageBinaryRepr) -> Result<Vec<BinaryReprExport>, String> {
    let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
    package
        .exports
        .iter()
        .map(|export| {
            let function = package
                .project
                .functions
                .get(export.function_id as usize)
                .ok_or_else(|| {
                    format!("export references missing function {}", export.function_id)
                })?;
            let built = BinaryReprExport {
                name: string_at(&package.project.strings.values, export.name)?.to_string(),
                kind: export.kind,
                isolated: function.flags & FUNCTION_FLAG_ISOLATED != 0,
                params: function
                    .params
                    .iter()
                    .map(|param| {
                        Ok::<BinaryReprExportParam, String>(BinaryReprExportParam {
                            name: string_at(&package.project.strings.values, param.name)?
                                .to_string(),
                            type_: type_name(&type_names, param.type_id)?.to_string(),
                            has_default: param.flags & 1 != 0,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                return_type: type_name(&type_names, function.return_type)?.to_string(),
            };
            Ok(built)
        })
        .collect()
}

pub(super) fn package_info(package: &PackageBinaryRepr) -> Result<BinaryReprPackageInfo, String> {
    let strings = &package.project.strings.values;
    let type_names = type_entry_names(&package.project.types, strings)?;
    let exports = package
        .project
        .abi
        .exports
        .iter()
        .map(|abi_export| {
            Ok(BinaryReprPackageInfoExport {
                name: string_at(strings, abi_export.name)?.to_string(),
                kind: abi_export.kind,
                sig_hash: hex_hash(&abi_export.sig_hash),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let globals = package
        .project
        .globals
        .iter()
        .map(|global| {
            let visibility = match (global.flags >> 1) & 0b11 {
                1 => "package",
                2 => "export",
                _ => "private",
            };
            Ok(BinaryReprPackageInfoGlobal {
                name: string_at(strings, global.name)?.to_string(),
                type_: type_name(&type_names, global.type_id)?.to_string(),
                mutable: global.flags & 1 != 0,
                visibility: visibility.to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut abi_edges = package
        .project
        .abi
        .dep_edges
        .iter()
        .map(|edge| {
            Ok((
                (
                    string_at(strings, edge.package_name)?.to_string(),
                    string_at(strings, edge.package_ident)?.to_string(),
                ),
                edge,
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;

    let imports = package
        .project
        .imports
        .entries
        .iter()
        .map(|entry| {
            let package_name = string_at(strings, entry.package_name)?.to_string();
            let package_ident = string_at(strings, entry.package_ident)?.to_string();
            let edge = abi_edges.remove(&(package_name.clone(), package_ident.clone()));
            let used_symbols = edge
                .map(|edge| {
                    edge.used_symbols
                        .iter()
                        .map(|symbol| {
                            Ok(BinaryReprPackageInfoUsedSymbol {
                                name: string_at(strings, symbol.name)?.to_string(),
                                sig_hash: hex_hash(&symbol.sig_hash),
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok(BinaryReprPackageInfoImport {
                package_name,
                package_ident,
                version: string_at(strings, entry.version)?.to_string(),
                pin: entry.pin,
                flags: entry.flags,
                used_symbols,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let cleanups = package
        .project
        .functions
        .iter()
        .flat_map(|function| {
            function
                .cleanups
                .iter()
                .map(move |cleanup| (function.name, cleanup))
        })
        .map(|(function_name, cleanup)| {
            Ok(BinaryReprPackageInfoCleanup {
                function: string_at(strings, function_name)?.to_string(),
                cleanup_id: cleanup.id,
                start_pc: cleanup.start_pc,
                end_pc: cleanup.end_pc,
                resource_register: cleanup.resource_register,
                close_function_id: cleanup.close_function_id,
                records_secondary_close_failure: cleanup.flags
                    & CLEANUP_FLAG_RECORD_SECONDARY_CLOSE_FAILURE
                    != 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(BinaryReprPackageInfo {
        manifest_name: string_at(strings, package.project.manifest.package_name)?.to_string(),
        manifest_ident: string_at(strings, package.project.manifest.package_ident)?.to_string(),
        manifest_version: string_at(strings, package.project.manifest.package_version)?.to_string(),
        manifest_ident_key: string_at(strings, package.project.manifest.ident_key)?.to_string(),
        manifest_ident_fingerprint: string_at(strings, package.project.manifest.ident_fingerprint)?
            .to_string(),
        manifest_signing_fingerprint: string_at(
            strings,
            package.project.manifest.signing_fingerprint,
        )?
        .to_string(),
        author: string_at(strings, package.project.manifest.author)?.to_string(),
        url: string_at(strings, package.project.manifest.url)?.to_string(),
        type_count: package.project.types.entries.len(),
        const_count: package.project.constants.entries.len(),
        resource_count: package.project.resources.entries.len(),
        function_count: package.project.functions.len(),
        global_count: package.project.globals.len(),
        export_count: package.project.abi.exports.len(),
        import_count: package.project.imports.entries.len(),
        cleanup_count: cleanups.len(),
        abi_format_version: ABI_FORMAT_VERSION,
        exports,
        globals,
        imports,
        cleanups,
    })
}

pub(super) fn package_type_exports(package: &PackageBinaryRepr) -> Result<Vec<BinaryReprTypeExport>, String> {
    let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
    let type_by_name = package
        .project
        .types
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let id = FIRST_TABLE_TYPE_ID + index as u32;
            type_name(&type_names, id)
                .ok()
                .map(|name| (name.to_string(), entry))
        })
        .collect::<HashMap<_, _>>();
    let mut exports = Vec::new();
    for export in &package.project.abi.exports {
        if !matches!(
            export.kind,
            BinaryReprExportKind::Type | BinaryReprExportKind::Union | BinaryReprExportKind::Enum
        ) {
            continue;
        }
        let name = string_at(&package.project.strings.values, export.name)?.to_string();
        let Some(entry) = type_by_name.get(&name) else {
            return Err(format!(
                "exported type `{name}` is missing from the type table"
            ));
        };
        exports.push(decode_type_export(
            &name,
            export.kind,
            entry,
            &type_names,
            &package.project.strings.values,
        )?);
    }
    Ok(exports)
}
