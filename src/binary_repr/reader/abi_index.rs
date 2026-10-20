//! plan-176-O: the `.mfp` reader's ABI index — reading and validating it, manifest counts and
//! export kinds. Container framing and the per-section table decoders live in
//! `binary_repr/reader/mod.rs`.

use super::*;

// === ABI index decode + validation + shared lookups =========================

pub(in crate::binary_repr) fn read_abi_index(bytes: &[u8]) -> Result<AbiIndex, String> {
    let mut offset = 0;
    let version = cursor_u16(bytes, &mut offset)?;
    if version != ABI_FORMAT_VERSION {
        return Err(format!("unsupported ABI_INDEX format version {version}"));
    }
    let _reserved = cursor_u16(bytes, &mut offset)?;

    let export_count = cursor_u32(bytes, &mut offset)? as usize;
    let mut exports = Vec::with_capacity(bounded_capacity(export_count, bytes.len() - offset, 38));
    for _ in 0..export_count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = match cursor_u16(bytes, &mut offset)? {
            kind => decode_export_kind(kind)?,
        };
        let sig_hash = cursor_hash(bytes, &mut offset)?;
        exports.push(AbiExport {
            name,
            kind,
            sig_hash,
        });
    }

    let edge_count = cursor_u32(bytes, &mut offset)? as usize;
    let mut dep_edges = Vec::with_capacity(bounded_capacity(edge_count, bytes.len() - offset, 17));
    for _ in 0..edge_count {
        let package_name = cursor_u32(bytes, &mut offset)?;
        let package_ident = cursor_u32(bytes, &mut offset)?;
        let version_request = cursor_u32(bytes, &mut offset)?;
        let pin = match cursor_u8(bytes, &mut offset)? {
            0 => false,
            1 => true,
            other => return Err(format!("unsupported ABI_INDEX dep pin value {other}")),
        };
        let used_count = cursor_u32(bytes, &mut offset)? as usize;
        let mut used_symbols =
            Vec::with_capacity(bounded_capacity(used_count, bytes.len() - offset, 36));
        for _ in 0..used_count {
            used_symbols.push(AbiUsedSymbol {
                name: cursor_u32(bytes, &mut offset)?,
                sig_hash: cursor_hash(bytes, &mut offset)?,
            });
        }
        dep_edges.push(AbiDepEdge {
            package_name,
            package_ident,
            version_request,
            pin,
            used_symbols,
        });
    }

    if offset != bytes.len() {
        return Err("invalid trailing bytes in ABI_INDEX".to_string());
    }

    Ok(AbiIndex { exports, dep_edges })
}

pub(in crate::binary_repr) fn validate_abi_index(
    abi: &AbiIndex,
    exports: &[DecodedExport],
    imports: &ImportTable,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
    functions: &[Function],
) -> Result<(), String> {
    for export in exports {
        let name = string_at(strings, export.name).unwrap_or("<invalid>");
        let Some(abi_export) = abi_export_for_decoded(abi, export) else {
            return Err(format!("ABI_INDEX is missing EXPORT_TABLE entry `{name}`"));
        };
        let Some(function) = functions.get(export.function_id as usize) else {
            return Err(format!(
                "export references missing function {}",
                export.function_id
            ));
        };
        let expected = function_sig_hash(function, export.kind, strings, types, constants)?;
        if abi_export.sig_hash != expected {
            return Err(format!(
                "ABI_INDEX export `{name}` sigHash disagrees with binary representation (required {}, provided {})",
                hex_hash(&expected),
                hex_hash(&abi_export.sig_hash)
            ));
        }
    }

    // Type/union/enum exports carry a sigHash too, but they have no EXPORT_TABLE
    // entry to key off, so the loop above never reaches them. Re-derive each one
    // from the decoded TYPE_TABLE the same way the writer does, so no ABI surface
    // is trusted unverified.
    for abi_export in &abi.exports {
        let entry_kind = match abi_export.kind {
            // bug-282 B1: the loop above is driven by EXPORT_TABLE, so a callable
            // ABI entry naming no EXPORT_TABLE row was never reached and its
            // sigHash never recomputed. Such an entry flows into
            // `package_info().exports`, `repo check-abi` and the registry
            // `abi_index`, and can satisfy an importer's used-symbol pin at
            // resolve time for a function that does not exist -- failing much
            // later at merge with a far more confusing error. Require the
            // counterpart here; the hash itself is then verified above.
            BinaryReprExportKind::Func => {
                if !exports
                    .iter()
                    .any(|export| export.name == abi_export.name && export.kind == abi_export.kind)
                {
                    let name = string_at(strings, abi_export.name)?;
                    return Err(format!(
                        "ABI_INDEX callable export `{name}` is missing from the export table"
                    ));
                }
                continue;
            }
            BinaryReprExportKind::Type => 1u16,
            BinaryReprExportKind::Union => 2,
            BinaryReprExportKind::Enum => 3,
        };
        let name = string_at(strings, abi_export.name)?;
        // A name is interned once, so `entry.name` compares by string id. Several
        // entries may still share it (an imported type of the same name lives in
        // the table too), so the export is valid when *some* candidate definition
        // reproduces its hash.
        let candidates = types
            .entries
            .iter()
            .enumerate()
            // bug-390: a re-exported foreign type (FOREIGN_TYPE_KIND) surfaces under
            // the owning package's Type/Union/Enum export kind, so accept it here
            // too — its `type_sig_hash` reproduces the stored hash from the payload's
            // owning ABI hash, exactly as the writer computed it.
            .filter(|(_, entry)| {
                (entry.kind == entry_kind || entry.kind == FOREIGN_TYPE_KIND)
                    && entry.name == abi_export.name
            })
            .map(|(index, _)| FIRST_TABLE_TYPE_ID + index as u32)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Err(format!(
                "ABI_INDEX type export `{name}` is missing from the type table"
            ));
        }
        let mut expected = None;
        for type_id in candidates {
            let hash = type_sig_hash(type_id, abi_export.kind, strings, types, constants)?;
            if hash == abi_export.sig_hash {
                expected = None;
                break;
            }
            expected.get_or_insert(hash);
        }
        if let Some(expected) = expected {
            return Err(format!(
                "ABI_INDEX type export `{name}` sigHash disagrees with binary representation (required {}, provided {})",
                hex_hash(&expected),
                hex_hash(&abi_export.sig_hash)
            ));
        }
    }

    let import_names = imports
        .entries
        .iter()
        .map(|entry| {
            Ok::<(String, String), String>((
                string_at(strings, entry.package_name)?.to_string(),
                string_at(strings, entry.package_ident)?.to_string(),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let edge_names = abi
        .dep_edges
        .iter()
        .map(|edge| {
            Ok::<(String, String), String>((
                string_at(strings, edge.package_name)?.to_string(),
                string_at(strings, edge.package_ident)?.to_string(),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if sorted_pairs(import_names) != sorted_pairs(edge_names) {
        return Err("ABI_INDEX dependency edges disagree with IMPORT_TABLE entries".to_string());
    }

    for import in &imports.entries {
        let Some(edge) = abi.dep_edges.iter().find(|edge| {
            edge.package_name == import.package_name && edge.package_ident == import.package_ident
        }) else {
            continue;
        };
        if edge.version_request != import.version || edge.pin != import.pin {
            let name = string_at(strings, import.package_name).unwrap_or("<invalid>");
            return Err(format!(
                "ABI_INDEX dependency edge `{name}` disagrees with IMPORT_TABLE request"
            ));
        }
        if edge.used_symbols.len() != import.used_symbols.len()
            || edge
                .used_symbols
                .iter()
                .zip(import.used_symbols.iter())
                .any(|(a, b)| a.name != b.name || a.sig_hash != b.sig_hash)
        {
            let name = string_at(strings, import.package_name).unwrap_or("<invalid>");
            return Err(format!(
                "ABI_INDEX dependency edge `{name}` disagrees with IMPORT_TABLE used symbols"
            ));
        }
    }

    Ok(())
}

/// Cross-check the counts the manifest repeats against the tables they describe
/// (bug-282 B4). Both were previously decoded into `_`-prefixed locals and
/// discarded, so a crafted manifest could claim counts its own tables
/// contradicted and no reader would notice. A legitimate writer derives both from
/// the tables it is about to emit, so any disagreement means the file is
/// internally inconsistent.
pub(in crate::binary_repr) fn validate_manifest_counts(
    manifest: &BinaryReprManifest,
    imports: &ImportTable,
    exports: &[DecodedExport],
) -> Result<(), String> {
    if manifest.dependency_count as usize != imports.entries.len() {
        return Err(format!(
            "manifest claims {} dependencies but the import table holds {}",
            manifest.dependency_count,
            imports.entries.len()
        ));
    }
    if manifest.export_count as usize != exports.len() {
        return Err(format!(
            "manifest claims {} exports but the export table holds {}",
            manifest.export_count,
            exports.len()
        ));
    }
    Ok(())
}

pub(in crate::binary_repr) fn abi_export_for_decoded<'a>(
    abi: &'a AbiIndex,
    export: &DecodedExport,
) -> Option<&'a AbiExport> {
    abi.exports
        .iter()
        .find(|abi_export| abi_export.name == export.name && abi_export.kind == export.kind)
}

pub(in crate::binary_repr) fn decode_callable_export_kind(
    value: u16,
) -> Result<BinaryReprExportKind, String> {
    match decode_export_kind(value)? {
        BinaryReprExportKind::Func => Ok(BinaryReprExportKind::Func),
        other => Err(format!(
            "unsupported callable export kind {}",
            encode_export_kind(other)
        )),
    }
}

/// A package built before plan-159-D can carry a retired value-less-callable
/// export (kind `2`, function flag bit 3). There is one package format and no
/// compatibility path: the reader names the cause and the fix instead of guessing
/// what the export meant.
pub(in crate::binary_repr) fn retired_sub_error() -> String {
    "package was built by an older compiler and exports a callable of a retired kind \
     (export kind 2); rebuild the package with this compiler"
        .to_string()
}

pub(in crate::binary_repr) fn decode_export_kind(
    value: u16,
) -> Result<BinaryReprExportKind, String> {
    match value {
        1 => Ok(BinaryReprExportKind::Func),
        RETIRED_EXPORT_KIND_SUB => Err(retired_sub_error()),
        3 => Ok(BinaryReprExportKind::Type),
        4 => Ok(BinaryReprExportKind::Union),
        5 => Ok(BinaryReprExportKind::Enum),
        other => Err(format!("unsupported export kind {other}")),
    }
}

pub(in crate::binary_repr) fn type_name(
    types: &HashMap<u32, String>,
    id: u32,
) -> Result<&str, String> {
    if let Some(name) = primitive_type_name(id) {
        return Ok(name);
    }
    types
        .get(&id)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown type id {id}"))
}

pub(in crate::binary_repr) fn string_at(strings: &[String], id: u32) -> Result<&str, String> {
    strings
        .get(id as usize)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown string id {id}"))
}
