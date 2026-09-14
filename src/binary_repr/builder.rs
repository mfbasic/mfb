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
            resolve_resource_close_name(package, entry.close_function_id, &type_name)?
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

/// Resolve a `RESOURCE_TABLE` close-function id to a call name. The two legacy
/// built-in sentinels map to the standard `fs.close`/`tcp.close` ops;
/// [`BUILTIN_RESOURCE_CLOSE_BY_TYPE`] resolves from the registry by the entry's
/// own `type_name`; any other id is a `functionId` index into the package's
/// function table.
pub(super) fn resolve_resource_close_name(
    package: &PackageBinaryRepr,
    close_function_id: u32,
    type_name: &str,
) -> Result<Option<String>, String> {
    match close_function_id {
        BUILTIN_FS_CLOSE_FUNCTION_ID => Ok(builtins::resource_close_function(
            &crate::types::ParameterType::named(crate::codegen::builtins::fs::FILE_TYPE_ID),
        )
        .map(str::to_string)),
        BUILTIN_STREAM_CLOSE_FUNCTION_ID => Ok(builtins::resource_close_function(
            &crate::types::ParameterType::named(crate::codegen::builtins::tcp::SOCKET_TYPE_ID),
        )
        .map(str::to_string)),
        // Every other built-in resource (bug-464 fallout). The entry already
        // carries the type, so the close op is derivable and no new sentinel is
        // needed per resource.
        BUILTIN_RESOURCE_CLOSE_BY_TYPE => Ok(builtins::resource_close_function(
            &crate::types::ParameterType::named(type_name),
        )
        .map(str::to_string)),
        id => match package.project.functions.get(id as usize) {
            Some(function) => Ok(Some(
                string_at(&package.project.strings.values, function.name)?.to_string(),
            )),
            None => Ok(None),
        },
    }
}

pub(super) fn package_exports(
    package: &PackageBinaryRepr,
) -> Result<Vec<BinaryReprExport>, String> {
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
                            type_: crate::types::ParameterType::parse(type_name(
                                &type_names,
                                param.type_id,
                            )?),
                            default: export_default(package, param)?,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                return_type: crate::types::ParameterType::parse(type_name(
                    &type_names,
                    function.return_type,
                )?),
            };
            Ok(built)
        })
        .collect()
}

/// Decode a parameter record's default for an importer (plan-136-B).
///
/// A default-function record names the parameter's hidden default function by its
/// package-local name; `read_binary_repr_package` has already refused one that is
/// not exactly what the writer produces (`validate_default_functions`). A literal
/// record decodes its `CONST_POOL` entry back to the `IrValue::Const` type and value
/// spelling — the inverse of `ConstPool::add`, kind by kind.
fn export_default(
    package: &PackageBinaryRepr,
    param: &Param,
) -> Result<BinaryReprExportDefault, String> {
    use crate::types::ParameterType;
    let strings = &package.project.strings.values;
    if param.flags & PARAM_FLAG_DEFAULT == 0 {
        return Ok(BinaryReprExportDefault::None);
    }
    if param.flags & PARAM_FLAG_DEFAULT_FUNCTION != 0 {
        let function = package
            .project
            .functions
            .get(param.default_const as usize)
            .ok_or_else(|| {
                format!(
                    "parameter default references missing function {}",
                    param.default_const
                )
            })?;
        return Ok(BinaryReprExportDefault::Function(
            string_at(strings, function.name)?.to_string(),
        ));
    }
    let constant = package
        .project
        .constants
        .entries
        .get(param.default_const as usize)
        .ok_or_else(|| {
            format!(
                "parameter default references missing constant {}",
                param.default_const
            )
        })?;
    let payload = &constant.payload;
    let (type_, value) = match constant.kind {
        1 => (ParameterType::Nothing, "NOTHING".to_string()),
        2 => (
            ParameterType::Boolean,
            (const_payload::<1>(payload)?[0] != 0).to_string(),
        ),
        3 => (
            ParameterType::Integer,
            i64::from_le_bytes(const_payload(payload)?).to_string(),
        ),
        4 => (
            ParameterType::Float,
            format!(
                "{:?}",
                f64::from_bits(u64::from_le_bytes(const_payload(payload)?))
            ),
        ),
        5 => (
            ParameterType::Fixed,
            crate::numeric::fixed_decimal_from_raw(i64::from_le_bytes(const_payload(payload)?)),
        ),
        6 => (
            ParameterType::String,
            string_at(strings, u32::from_le_bytes(const_payload(payload)?))?.to_string(),
        ),
        7 => (
            ParameterType::Byte,
            const_payload::<1>(payload)?[0].to_string(),
        ),
        kind if kind == TYPE_MONEY as u16 => (
            ParameterType::Money,
            crate::numeric::money_decimal_from_raw(i64::from_le_bytes(const_payload(payload)?)),
        ),
        kind if kind == TYPE_SCALAR as u16 => (
            ParameterType::named("Scalar"),
            u32::from_le_bytes(const_payload(payload)?).to_string(),
        ),
        kind => {
            return Err(format!(
                "parameter default references a constant of unknown kind {kind}"
            ))
        }
    };
    Ok(BinaryReprExportDefault::Literal { type_, value })
}

/// A constant's payload as exactly `N` bytes, or an error naming the mismatch.
fn const_payload<const N: usize>(payload: &[u8]) -> Result<[u8; N], String> {
    payload
        .try_into()
        .map_err(|_| format!("constant payload is {} bytes, expected {N}", payload.len()))
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
                1 => "public",
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

pub(super) fn package_type_exports(
    package: &PackageBinaryRepr,
) -> Result<Vec<BinaryReprTypeExport>, String> {
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
        if entry.kind == FOREIGN_TYPE_KIND {
            // bug-390: a re-exported dependency type carries no local field data;
            // emit a marker naming the owning package. `read_package_type_exports`
            // fills in the real definition from the owner's sibling `.mfp`.
            let owner =
                string_at(&package.project.strings.values, entry.owner_package)?.to_string();
            exports.push(BinaryReprTypeExport {
                name,
                kind: export.kind,
                fields: Vec::new(),
                variants: Vec::new(),
                members: Vec::new(),
                foreign_owner: Some(owner),
            });
            continue;
        }
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
