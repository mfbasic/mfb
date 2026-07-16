use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

use crate::binary_repr;
use crate::ir;
use crate::json_string;

const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];

/// Parsed container v1.0 `.mfp` header (plan-23 §4). The reader is hard
/// v1.0: `containerMajor.containerMinor` must be exactly `1.0`.
pub(crate) struct MfpHeader {
    pub(crate) name: String,
    pub(crate) ident: String,
    pub(crate) version: String,
    pub(crate) author: String,
    pub(crate) url: String,
    pub(crate) ident_key: String,
    pub(crate) signing_key: String,
    pub(crate) proof: String,
    pub(crate) attestation: String,
    pub(crate) package_binary_hash: [u8; 32],
    pub(crate) container_major: u16,
    pub(crate) container_minor: u16,
    pub(crate) binary_repr_major: u16,
    pub(crate) binary_repr_minor: u16,
    pub(crate) flags: u32,
    pub(crate) signature_type: u16,
    pub(crate) signature_length: usize,
    pub(crate) binary_repr_length: usize,
}

pub(crate) struct ProjectPackageDependency {
    pub(crate) name: String,
    pub(crate) ident: String,
    pub(crate) version: String,
    pub(crate) pin: bool,
    pub(crate) source: String,
    /// The pinned owner identKey (plan-23 §3.5 trust anchor), captured on
    /// first `pkg add` of a signed package (trust-on-first-use). Empty for
    /// unsigned dependencies.
    pub(crate) ident_key: String,
}

pub(crate) fn package_file_url_path(url: &str) -> Result<PathBuf, String> {
    let Some(path) = url.strip_prefix("file://") else {
        return Err("mfb pkg add currently supports only file:// URLs ending in .mfp".to_string());
    };

    if path.is_empty() {
        return Err("file:// URL must include an absolute package path".to_string());
    }
    if path.contains('?') || path.contains('#') {
        return Err("file:// package URLs must not include query strings or fragments".to_string());
    }

    let path = PathBuf::from(percent_decode_path(path)?);
    if !path.is_absolute() {
        return Err("file:// package URL must resolve to an absolute path".to_string());
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("mfp") {
        return Err("file:// package URL must point to a .mfp file".to_string());
    }
    if !path.is_file() {
        return Err(format!("package file '{}' does not exist", path.display()));
    }

    Ok(path)
}

fn percent_decode_path(path: &str) -> Result<String, String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("file:// URL contains an incomplete percent escape".to_string());
            }
            let high = hex_value(bytes[index + 1])
                .ok_or_else(|| "file:// URL contains an invalid percent escape".to_string())?;
            let low = hex_value(bytes[index + 2])
                .ok_or_else(|| "file:// URL contains an invalid percent escape".to_string())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded).map_err(|_| "file:// URL path is not valid UTF-8".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Rejects a package name that cannot be used as a single path component.
///
/// A `.mfp` header name and an `mfb.lock` name are untrusted: both are turned
/// into `packages/<name>.mfp`. Without this guard a name of `../../x` escapes the
/// project, and a name beginning with `.` hides the file. Legitimate names are
/// identifier-like, so the charset is deliberately narrow.
pub(crate) fn validate_package_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let leading_ok = chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    let rest_ok = chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    if !leading_ok || !rest_ok {
        return Err(format!(
            "package name `{name}` is not a valid path component (expected [A-Za-z0-9_][A-Za-z0-9_.-]*)"
        ));
    }
    Ok(())
}

pub(crate) fn read_mfp_header(path: &Path) -> Result<MfpHeader, String> {
    let bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    if bytes.len() < 20 {
        return Err(format!(
            "'{}' is too small to be a valid .mfp package",
            path.display()
        ));
    }
    if bytes[0..8] != MFP_MAGIC {
        return Err(format!(
            "'{}' does not have the MFP package magic",
            path.display()
        ));
    }

    let container_major = read_u16(&bytes, 8)?;
    let container_minor = read_u16(&bytes, 10)?;
    if container_major != 1 || container_minor != 0 {
        return Err(format!(
            "'{}' uses unsupported MFP container version {container_major}.{container_minor} (expected 1.0)",
            path.display()
        ));
    }
    let binary_repr_major = read_u16(&bytes, 12)?;
    let binary_repr_minor = read_u16(&bytes, 14)?;
    let flags = read_u32(&bytes, 16)?;

    let mut offset = 20usize;
    let name = read_mfp_string(&bytes, &mut offset, "name", 255, true)?;
    validate_package_name(&name)?;
    let ident = read_mfp_string(&bytes, &mut offset, "ident", 255, false)?;
    let version = read_mfp_string(&bytes, &mut offset, "version", 64, true)?;
    let author = read_mfp_string(&bytes, &mut offset, "author", 512, false)?;
    let url = read_mfp_string(&bytes, &mut offset, "url", 2048, false)?;
    let ident_key = read_mfp_string(&bytes, &mut offset, "identKey", 255, false)?;
    let signing_key = read_mfp_string(&bytes, &mut offset, "signingKey", 255, false)?;
    let proof = read_mfp_string(&bytes, &mut offset, "proof", 4096, false)?;
    let _proof_sig = read_mfp_bytes(&bytes, &mut offset, "proofSig", 64)?;
    let attestation = read_mfp_string(&bytes, &mut offset, "attestation", 4096, false)?;
    let _attestation_sig = read_mfp_bytes(&bytes, &mut offset, "attestationSig", 64)?;

    let hash_end = offset
        .checked_add(32)
        .ok_or_else(|| "truncated .mfp packageBinaryHash".to_string())?;
    if hash_end > bytes.len() {
        return Err("truncated .mfp packageBinaryHash".to_string());
    }
    let mut package_binary_hash = [0u8; 32];
    package_binary_hash.copy_from_slice(&bytes[offset..hash_end]);
    offset = hash_end;

    // `as usize` would truncate a hostile 64-bit length on a 32-bit target, and
    // the structural `offset != bytes.len()` check below would then validate the
    // truncated value instead of the declared one.
    let binary_repr_length = usize::try_from(read_u64(&bytes, offset)?)
        .map_err(|_| "invalid .mfp binary representation length".to_string())?;
    offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;

    let signature_type = read_u16(&bytes, offset)?;
    let signature_length = read_u32(&bytes, offset + 2)? as usize;
    match (signature_type, signature_length) {
        (0, 0) | (1, 64) => {}
        (0, _) => return Err("unsigned .mfp package must have zero signature length".to_string()),
        (1, _) => return Err("Ed25519 .mfp package must have a 64 byte signature".to_string()),
        _ => return Err(format!("unsupported .mfp signature type {signature_type}")),
    }
    offset = offset
        .checked_add(6)
        .and_then(|offset| offset.checked_add(signature_length))
        .and_then(|offset| offset.checked_add(binary_repr_length))
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;
    if offset != bytes.len() {
        return Err("invalid .mfp binary representation length".to_string());
    }

    Ok(MfpHeader {
        name,
        ident,
        version,
        author,
        url,
        ident_key,
        signing_key,
        proof,
        attestation,
        package_binary_hash,
        container_major,
        container_minor,
        binary_repr_major,
        binary_repr_minor,
        flags,
        signature_type,
        signature_length,
        binary_repr_length,
    })
}

fn read_mfp_string(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
    limit: usize,
    required: bool,
) -> Result<String, String> {
    let raw = read_mfp_bytes(bytes, offset, field, limit)?;
    let value = String::from_utf8(raw).map_err(|_| format!(".mfp {field} is not valid UTF-8"))?;
    if required && value.is_empty() {
        return Err(format!(".mfp {field} must not be empty"));
    }
    Ok(value)
}

fn read_mfp_bytes(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let length = read_u32(bytes, *offset)? as usize;
    *offset = offset
        .checked_add(4)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;

    if length > limit {
        return Err(format!(".mfp {field} exceeds the {limit} byte limit"));
    }

    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    if end > bytes.len() {
        return Err(format!("truncated .mfp {field}"));
    }

    let value = bytes[*offset..end].to_vec();
    *offset = end;
    Ok(value)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

pub(crate) fn installed_package_files(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Result<Vec<PathBuf>, String> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Ok(Vec::new());
    };

    packages
        .iter()
        .filter_map(project_package_dependency)
        .map(|dependency| {
            let package_file = project_dir
                .join("packages")
                .join(format!("{}.mfp", dependency.name));
            if package_file.is_file() {
                let header = read_mfp_header(&package_file)?;
                if dependency.pin && header.version != dependency.version {
                    return Err(format!(
                        "package `{}` is pinned to version {}, but installed package is version {}",
                        dependency.name, dependency.version, header.version
                    ));
                }
                Ok(package_file)
            } else {
                Err(format!(
                    "package `{}` must be installed as '{}' before binary representation merging",
                    dependency.name,
                    package_file.display()
                ))
            }
        })
        .collect()
}

pub(crate) fn external_package_function_types(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> (
    HashMap<String, String>,
    HashMap<String, Vec<ir::ExternalFunctionParam>>,
) {
    let Ok(packages) = installed_package_files(project_dir, manifest) else {
        return (HashMap::new(), HashMap::new());
    };
    external_package_function_types_from_files_lossy(&packages)
}

pub(crate) fn external_package_function_types_from_files(
    packages: &[PathBuf],
) -> Result<
    (
        HashMap<String, String>,
        HashMap<String, Vec<ir::ExternalFunctionParam>>,
    ),
    String,
> {
    let mut functions = HashMap::new();
    let mut params = HashMap::new();
    for package in packages {
        let header = read_mfp_header(package)?;
        for export in binary_repr::read_package_exports(package)? {
            let name = format!("{}.{}", header.name, export.name);
            functions.insert(name.clone(), package_export_function_type(&export));
            params.insert(
                name,
                export
                    .params
                    .iter()
                    .map(|param| ir::ExternalFunctionParam {
                        name: param.name.clone(),
                        type_: param.type_.clone(),
                    })
                    .collect(),
            );
        }
    }
    Ok((functions, params))
}

fn external_package_function_types_from_files_lossy(
    packages: &[PathBuf],
) -> (
    HashMap<String, String>,
    HashMap<String, Vec<ir::ExternalFunctionParam>>,
) {
    let mut functions = HashMap::new();
    let mut params = HashMap::new();
    for package in packages {
        let Ok(header) = read_mfp_header(package) else {
            continue;
        };
        let Ok(exports) = binary_repr::read_package_exports(package) else {
            continue;
        };
        for export in exports {
            let name = format!("{}.{}", header.name, export.name);
            functions.insert(name.clone(), package_export_function_type(&export));
            params.insert(
                name,
                export
                    .params
                    .iter()
                    .map(|param| ir::ExternalFunctionParam {
                        name: param.name.clone(),
                        type_: param.type_.clone(),
                    })
                    .collect(),
            );
        }
    }
    (functions, params)
}

fn package_export_function_type(export: &binary_repr::BinaryReprExport) -> String {
    let params = export
        .params
        .iter()
        .map(|param| param.type_.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let prefix = if export.isolated { "ISOLATED " } else { "" };
    format!("{prefix}FUNC({params}) AS {}", export.return_type)
}

pub(crate) fn package_metadata(
    manifest: &HashMap<String, JsonValue>,
) -> binary_repr::BinaryReprMetadata {
    let name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name")
        .clone();
    let version = manifest
        .get("version")
        .and_then(|value| value.get::<String>())
        .expect("validated project version")
        .clone();
    let mut metadata = binary_repr::BinaryReprMetadata::new(name, version);
    metadata.ident = manifest
        .get("ident")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    // Identity-chain fields (identKey and the key fingerprints) are outputs
    // of `--sign` (plan-23 §3.3), stamped by apply_signing_metadata — never
    // read from the manifest. An unsigned package carries no identity chain,
    // and the file-embedded key is never a trust root.
    metadata.author = manifest
        .get("author")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    metadata.url = manifest
        .get("url")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    metadata.dependencies = package_dependencies(manifest);
    metadata
}

fn package_dependencies(
    manifest: &HashMap<String, JsonValue>,
) -> Vec<binary_repr::BinaryReprDependency> {
    manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(|package| package.get::<HashMap<String, JsonValue>>())
        .filter_map(|package| {
            let name = package.get("name")?.get::<String>()?.clone();
            let ident = package
                .get("ident")
                .and_then(|value| value.get::<String>())
                .cloned()
                .unwrap_or_else(|| name.clone());
            let version = package
                .get("version")
                .and_then(|value| value.get::<String>())
                .cloned()
                .unwrap_or_default();
            let pin = package
                .get("pin")
                .and_then(|value| value.get::<bool>())
                .copied()
                .unwrap_or(false);
            Some(binary_repr::BinaryReprDependency {
                name,
                ident,
                version,
                pin,
                flags: 0,
            })
        })
        .collect()
}

pub(crate) fn project_package_dependency(value: &JsonValue) -> Option<ProjectPackageDependency> {
    let package = value.get::<HashMap<String, JsonValue>>()?;
    let name = package.get("name")?.get::<String>()?.clone();
    let ident = package
        .get("ident")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_else(|| name.clone());
    let version = package
        .get("version")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    let source = package
        .get("source")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    let pin = package
        .get("pin")
        .and_then(|value| value.get::<bool>())
        .copied()
        .unwrap_or(false);
    let ident_key = package
        .get("identKey")
        .or_else(|| package.get("ident_key"))
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();

    if name.trim().is_empty() {
        return None;
    }

    // The dependency `name` is interpolated into `packages/<name>.mfp` and
    // read/merged by `mfb audit`/`build`/`pkg`. Validate it as a single path
    // component (same guard the header-stored name uses) so a `../…` or absolute
    // name cannot escape `packages/` and probe/merge arbitrary host `.mfp` files
    // (bug-195). Reject the dependency on failure, mirroring the blank-name case.
    if validate_package_name(&name).is_err() {
        return None;
    }

    Some(ProjectPackageDependency {
        name,
        ident,
        version,
        pin,
        source,
        ident_key,
    })
}

pub(crate) fn project_json_with_package(
    contents: &str,
    manifest: &HashMap<String, JsonValue>,
    dependency: &ProjectPackageDependency,
) -> Result<String, String> {
    let packages = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>());

    if packages.is_some_and(|packages| {
        packages.iter().any(|package| {
            package
                .get::<HashMap<String, JsonValue>>()
                .and_then(|package| package.get("name"))
                .and_then(|name| name.get::<String>())
                == Some(&dependency.name)
        })
    }) {
        return Err(format!(
            "project.json already declares package `{}`",
            dependency.name
        ));
    }

    let entry = package_dependency_json(dependency, 4);
    if packages.is_some() {
        insert_package_dependency(contents, &entry)
    } else {
        insert_packages_array(contents, &entry)
    }
}

fn package_dependency_json(dependency: &ProjectPackageDependency, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let field_pad = " ".repeat(indent + 2);
    let ident_key = if dependency.ident_key.is_empty() {
        String::new()
    } else {
        format!(
            ",\n{field_pad}\"identKey\": {}",
            json_string(&dependency.ident_key)
        )
    };
    format!(
        "{pad}{{\n{field_pad}\"name\": {},\n{field_pad}\"ident\": {},\n{field_pad}\"version\": {},\n{field_pad}\"pin\": {},\n{field_pad}\"source\": {}{ident_key}\n{pad}}}",
        json_string(&dependency.name),
        json_string(&dependency.ident),
        json_string(&dependency.version),
        dependency.pin,
        json_string(&dependency.source),
        pad = pad,
        field_pad = field_pad,
    )
}

fn insert_package_dependency(contents: &str, entry: &str) -> Result<String, String> {
    let Some((array_start, array_end)) = json_array_bounds(contents, "packages") else {
        return Err("could not locate project.json `packages` array".to_string());
    };
    let inner = &contents[array_start + 1..array_end];
    let has_entries = !inner.trim().is_empty();
    let before_entry = contents[..array_end].trim_end_matches([' ', '\t', '\r', '\n']);
    let closing_indent = &contents[before_entry.len()..array_end];

    let mut updated = String::new();
    updated.push_str(before_entry);
    if has_entries {
        updated.push(',');
    }
    updated.push('\n');
    updated.push_str(entry);
    updated.push_str(closing_indent);
    updated.push_str(&contents[array_end..]);
    Ok(updated)
}

fn insert_packages_array(contents: &str, entry: &str) -> Result<String, String> {
    let Some(root_end) = root_object_end(contents) else {
        return Err("could not locate end of project.json object".to_string());
    };
    let before = contents[..root_end].trim_end_matches([' ', '\t', '\r', '\n']);
    let between = &contents[before.len()..root_end];
    let needs_comma = before.as_bytes().last().is_some_and(|byte| *byte != b'{');

    let mut updated = String::new();
    updated.push_str(before);
    if needs_comma {
        updated.push(',');
    }
    updated.push_str("\n  \"packages\": [\n");
    updated.push_str(entry);
    updated.push_str("\n  ]");
    updated.push_str(between);
    updated.push_str(&contents[root_end..]);
    Ok(updated)
}

/// Rewrite (or insert) the pinned `identKey` of the dependency named `name`
/// in `project.json`, preserving the file's formatting (plan-23-B2
/// pin-follow after an ident rotation).
pub(crate) fn project_json_with_updated_ident_key(
    contents: &str,
    name: &str,
    new_key: &str,
) -> Result<String, String> {
    let Some((array_start, array_end)) = json_array_bounds(contents, "packages") else {
        return Err("could not locate project.json `packages` array".to_string());
    };
    let mut cursor = array_start + 1;
    while cursor < array_end {
        let Some(object_start) = contents[cursor..array_end].find('{').map(|at| cursor + at) else {
            break;
        };
        let Some(object_end) = matching_json_delimiter(contents, object_start, b'{', b'}') else {
            return Err("malformed project.json `packages` entry".to_string());
        };
        let object = &contents[object_start..=object_end];
        let is_target = object
            .parse::<JsonValue>()
            .ok()
            .and_then(|value| {
                value
                    .get::<HashMap<String, JsonValue>>()
                    .and_then(|entry| entry.get("name"))
                    .and_then(|value| value.get::<String>())
                    .cloned()
            })
            .is_some_and(|entry_name| entry_name == name);
        if !is_target {
            cursor = object_end + 1;
            continue;
        }
        let mut updated = String::new();
        if let Some(field_at) = json_field_name_position(object, "identKey")
            .or_else(|| json_field_name_position(object, "ident_key"))
        {
            let field_len = if object[field_at..].starts_with("\"identKey\"") {
                "\"identKey\"".len()
            } else {
                "\"ident_key\"".len()
            };
            let colon = find_json_punct(object, field_at + field_len, b':')
                .ok_or_else(|| "malformed identKey field".to_string())?;
            let value_start = next_json_string_start(object, colon + 1)
                .ok_or_else(|| "malformed identKey value".to_string())?;
            let value_end = json_string_end(object, value_start)
                .ok_or_else(|| "malformed identKey value".to_string())?;
            updated.push_str(&contents[..object_start + value_start]);
            updated.push_str(&json_string(new_key));
            updated.push_str(&contents[object_start + value_end..]);
        } else {
            // No pin recorded yet: append the field before the closing brace.
            let before_close = object[..object.len() - 1].trim_end_matches([' ', '\t', '\r', '\n']);
            let closing = &object[before_close.len()..];
            updated.push_str(&contents[..object_start]);
            updated.push_str(before_close);
            updated.push_str(",\n      \"identKey\": ");
            updated.push_str(&json_string(new_key));
            updated.push_str(closing);
            updated.push_str(&contents[object_end + 1..]);
        }
        return Ok(updated);
    }
    Err(format!("project.json does not declare package `{name}`"))
}

fn json_array_bounds(contents: &str, field: &str) -> Option<(usize, usize)> {
    let field_start = json_field_name_position(contents, field)?;
    let colon = find_json_punct(contents, field_start + field.len() + 2, b':')?;
    let array_start = find_json_punct(contents, colon + 1, b'[')?;
    let array_end = matching_json_delimiter(contents, array_start, b'[', b']')?;
    Some((array_start, array_end))
}

fn json_field_name_position(contents: &str, field: &str) -> Option<usize> {
    let needle = format!("\"{field}\"");
    let mut index = 0;

    loop {
        index = next_json_string_start(contents, index)?;
        let end = json_string_end(contents, index)?;
        if &contents[index..end] == needle {
            return Some(index);
        }
        index = end;
    }
}

fn root_object_end(contents: &str) -> Option<usize> {
    let start = find_json_punct(contents, 0, b'{')?;
    matching_json_delimiter(contents, start, b'{', b'}')
}

fn find_json_punct(contents: &str, start: usize, punct: u8) -> Option<usize> {
    let bytes = contents.as_bytes();
    let mut index = start;
    let mut in_string = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == punct {
            return Some(index);
        } else if !byte.is_ascii_whitespace() {
            return None;
        }
        index += 1;
    }

    None
}

fn matching_json_delimiter(contents: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = contents.as_bytes();
    let mut index = start;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == open {
            depth = depth.checked_add(1)?;
        } else if byte == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }

    None
}

fn next_json_string_start(contents: &str, start: usize) -> Option<usize> {
    contents[start..].find('"').map(|offset| start + offset)
}

fn json_string_end(contents: &str, start: usize) -> Option<usize> {
    let bytes = contents.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }

    let mut index = start + 1;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinyjson::JsonValue;

    // --- .mfp header byte builder -----------------------------------------

    /// Append a length-prefixed (u32 LE) byte field to a buffer.
    fn push_field(buf: &mut Vec<u8>, value: &[u8]) {
        buf.extend_from_slice(&(value.len() as u32).to_le_bytes());
        buf.extend_from_slice(value);
    }

    /// Build a minimal, structurally-valid unsigned v1.0 `.mfp` header with an
    /// empty binary-representation body. Overridable pieces let each test tweak
    /// one aspect and assert the corresponding error.
    fn build_mfp(name: &str, version: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MFP_MAGIC);
        buf.extend_from_slice(&1u16.to_le_bytes()); // container major
        buf.extend_from_slice(&0u16.to_le_bytes()); // container minor
        buf.extend_from_slice(&1u16.to_le_bytes()); // binary_repr major
        buf.extend_from_slice(&0u16.to_le_bytes()); // binary_repr minor
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags
        push_field(&mut buf, name.as_bytes()); // name (required)
        push_field(&mut buf, b""); // ident
        push_field(&mut buf, version.as_bytes()); // version (required)
        push_field(&mut buf, b""); // author
        push_field(&mut buf, b""); // url
        push_field(&mut buf, b""); // identKey
        push_field(&mut buf, b""); // signingKey
        push_field(&mut buf, b""); // proof
        push_field(&mut buf, &[0u8; 64]); // proofSig
        push_field(&mut buf, b""); // attestation
        push_field(&mut buf, &[0u8; 64]); // attestationSig
        buf.extend_from_slice(&[0u8; 32]); // packageBinaryHash
        buf.extend_from_slice(&0u64.to_le_bytes()); // binary_repr length = 0
        buf.extend_from_slice(&0u16.to_le_bytes()); // signature type = unsigned
        buf.extend_from_slice(&0u32.to_le_bytes()); // signature length = 0
        buf
    }

    /// Read a header expecting failure, returning the error string. `MfpHeader`
    /// is not `Debug`, so `unwrap_err` is unavailable.
    fn header_err(path: &Path) -> String {
        match read_mfp_header(path) {
            Ok(_) => panic!("expected read_mfp_header to fail for {}", path.display()),
            Err(err) => err,
        }
    }

    fn write_temp(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pkg.mfp");
        fs::write(&path, bytes).expect("write mfp");
        (dir, path)
    }

    #[test]
    fn read_mfp_header_parses_valid_unsigned_package() {
        let (_dir, path) = write_temp(&build_mfp("mypkg", "1.2.3"));
        let header = read_mfp_header(&path).expect("valid header");
        assert_eq!(header.name, "mypkg");
        assert_eq!(header.version, "1.2.3");
        assert_eq!(header.container_major, 1);
        assert_eq!(header.container_minor, 0);
        assert_eq!(header.signature_type, 0);
        assert_eq!(header.signature_length, 0);
        assert_eq!(header.binary_repr_length, 0);
    }

    #[test]
    fn read_mfp_header_rejects_a_name_that_is_not_a_path_component() {
        // A consumer installs this as `packages/<name>.mfp`; a traversing name
        // would escape the project directory.
        for name in [
            "../../../../home/victim/.config/autostart/x",
            "..",
            ".",
            "a/b",
            "a\\b",
            ".hidden",
            "-rf",
            "na me",
        ] {
            let (_dir, path) = write_temp(&build_mfp(name, "1.0.0"));
            let err = header_err(&path);
            assert!(
                err.contains("not a valid path component"),
                "name `{name}` gave `{err}`"
            );
        }
        // Legitimate names still parse.
        for name in ["mypkg", "my_pkg", "my-pkg", "pkg.v2", "_x", "9lives"] {
            let (_dir, path) = write_temp(&build_mfp(name, "1.0.0"));
            assert!(read_mfp_header(&path).is_ok(), "name `{name}` must parse");
        }
    }

    #[test]
    fn read_mfp_header_rejects_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = header_err(&dir.path().join("absent.mfp"));
        assert!(err.contains("failed to read"));
    }

    #[test]
    fn read_mfp_header_rejects_too_small() {
        let (_dir, path) = write_temp(&[0u8; 4]);
        let err = header_err(&path);
        assert!(err.contains("too small"));
    }

    #[test]
    fn read_mfp_header_rejects_bad_magic() {
        let mut bytes = build_mfp("p", "1");
        bytes[0] = 0xff;
        let (_dir, path) = write_temp(&bytes);
        let err = header_err(&path);
        assert!(err.contains("magic"));
    }

    #[test]
    fn read_mfp_header_rejects_bad_container_version() {
        let mut bytes = build_mfp("p", "1");
        bytes[8] = 2; // container major 2.0
        let (_dir, path) = write_temp(&bytes);
        let err = header_err(&path);
        assert!(err.contains("unsupported MFP container version"));
    }

    #[test]
    fn read_mfp_header_rejects_empty_required_name() {
        let (_dir, path) = write_temp(&build_mfp("", "1"));
        let err = header_err(&path);
        assert!(err.contains("name must not be empty"));
    }

    #[test]
    fn read_mfp_header_rejects_signature_length_mismatch() {
        // Unsigned (type 0) with a non-zero signature length is rejected.
        let mut bytes = build_mfp("p", "1");
        let len = bytes.len();
        // The signature-length u32 is the last 4 bytes we appended.
        bytes[len - 4..].copy_from_slice(&5u32.to_le_bytes());
        let (_dir, path) = write_temp(&bytes);
        let err = header_err(&path);
        assert!(err.contains("unsigned .mfp package must have zero signature length"));
    }

    #[test]
    fn read_mfp_header_rejects_unknown_signature_type() {
        let mut bytes = build_mfp("p", "1");
        let len = bytes.len();
        // signature type u16 sits 6 bytes before the end (2 type + 4 length).
        bytes[len - 6..len - 4].copy_from_slice(&9u16.to_le_bytes());
        let (_dir, path) = write_temp(&bytes);
        let err = header_err(&path);
        assert!(err.contains("unsupported .mfp signature type"));
    }

    #[test]
    fn read_mfp_header_rejects_trailing_length_mismatch() {
        // Claim a non-zero binary_repr length but supply no body -> the final
        // `offset != bytes.len()` check fails.
        let mut bytes = build_mfp("p", "1");
        // binary_repr length u64 sits 14 bytes before the end (8 + 2 + 4).
        let at = bytes.len() - 14;
        bytes[at..at + 8].copy_from_slice(&100u64.to_le_bytes());
        let (_dir, path) = write_temp(&bytes);
        let err = header_err(&path);
        assert!(err.contains("invalid .mfp binary representation length"));
    }

    /// bug-37: a 64-bit body length is narrowed to `usize`. `as` would truncate a
    /// hostile `0x1_0000_0000` to `0` on a 32-bit target and the structural
    /// `offset != bytes.len()` check would then validate the truncated value. Any
    /// oversized length must be rejected — here it fails the trailing check on a
    /// 64-bit host and the `try_from` guard on a narrower one.
    #[test]
    fn read_mfp_header_rejects_a_length_that_cannot_address_the_body() {
        let mut bytes = build_mfp("p", "1");
        let at = bytes.len() - 14;
        bytes[at..at + 8].copy_from_slice(&0x1_0000_0000u64.to_le_bytes());
        let (_dir, path) = write_temp(&bytes);
        let err = header_err(&path);
        assert!(err.contains("invalid .mfp binary representation length"), "{err}");

        // The maximum u64 cannot describe any body on any target.
        let mut bytes = build_mfp("p", "1");
        bytes[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        let (_dir, path) = write_temp(&bytes);
        let err = header_err(&path);
        assert!(err.contains("invalid .mfp binary representation length"), "{err}");
    }

    #[test]
    fn read_mfp_header_rejects_truncated_field() {
        // Truncate mid-header (after the fixed 20-byte prefix) so a length-
        // prefixed string read runs off the end.
        let bytes = build_mfp("longishname", "1")[..24].to_vec();
        let (_dir, path) = write_temp(&bytes);
        assert!(read_mfp_header(&path).is_err());
    }

    #[test]
    fn read_mfp_header_rejects_oversized_field() {
        // Name length claims 300 bytes (> 255 limit) with matching payload.
        let mut buf = Vec::new();
        buf.extend_from_slice(&MFP_MAGIC);
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&300u32.to_le_bytes()); // name length exceeds limit
        buf.extend_from_slice(&vec![b'x'; 300]);
        let (_dir, path) = write_temp(&buf);
        let err = header_err(&path);
        assert!(err.contains("exceeds the 255 byte limit"));
    }

    // --- percent decoding / file:// URLs ----------------------------------

    #[test]
    fn package_file_url_path_validates_scheme_and_extension() {
        assert!(package_file_url_path("https://x/y.mfp")
            .unwrap_err()
            .contains("file://"));
        assert!(package_file_url_path("file://")
            .unwrap_err()
            .contains("absolute package path"));
        assert!(package_file_url_path("file:///a/b.mfp?x=1")
            .unwrap_err()
            .contains("query strings or fragments"));
        assert!(package_file_url_path("file://relative/path.mfp")
            .unwrap_err()
            .contains("absolute path"));
        assert!(package_file_url_path("file:///abs/path.txt")
            .unwrap_err()
            .contains("must point to a .mfp file"));
        assert!(package_file_url_path("file:///does/not/exist.mfp")
            .unwrap_err()
            .contains("does not exist"));
    }

    #[test]
    fn package_file_url_path_accepts_existing_file_and_decodes_percent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("my pkg.mfp");
        fs::write(&file, b"x").unwrap();
        // Percent-encode the space in the path.
        let encoded = file.display().to_string().replace(' ', "%20");
        let url = format!("file://{encoded}");
        let resolved = package_file_url_path(&url).expect("resolves");
        assert_eq!(resolved, file);
    }

    #[test]
    fn percent_decode_rejects_malformed_escapes() {
        assert!(percent_decode_path("abc%")
            .unwrap_err()
            .contains("incomplete"));
        assert!(percent_decode_path("abc%zz")
            .unwrap_err()
            .contains("invalid percent escape"));
        assert_eq!(percent_decode_path("a%2Fb").unwrap(), "a/b");
    }

    #[test]
    fn hex_value_covers_all_cases() {
        assert_eq!(hex_value(b'0'), Some(0));
        assert_eq!(hex_value(b'9'), Some(9));
        assert_eq!(hex_value(b'a'), Some(10));
        assert_eq!(hex_value(b'F'), Some(15));
        assert_eq!(hex_value(b'g'), None);
    }

    // --- project.json package dependency parsing --------------------------

    fn json(src: &str) -> JsonValue {
        src.parse::<JsonValue>().expect("json")
    }

    #[test]
    fn project_package_dependency_reads_all_fields() {
        let value = json(
            r#"{"name":"pkg","ident":"pkg.id","version":"2.0","source":"file:///p.mfp","pin":true,"identKey":"KEY"}"#,
        );
        let dep = project_package_dependency(&value).expect("dependency");
        assert_eq!(dep.name, "pkg");
        assert_eq!(dep.ident, "pkg.id");
        assert_eq!(dep.version, "2.0");
        assert!(dep.pin);
        assert_eq!(dep.source, "file:///p.mfp");
        assert_eq!(dep.ident_key, "KEY");
    }

    #[test]
    fn project_package_dependency_defaults_and_ident_key_alias() {
        // Missing ident defaults to name; ident_key snake-case alias is honored.
        let value = json(r#"{"name":"pkg","ident_key":"K2"}"#);
        let dep = project_package_dependency(&value).expect("dependency");
        assert_eq!(dep.ident, "pkg");
        assert_eq!(dep.version, "");
        assert!(!dep.pin);
        assert_eq!(dep.ident_key, "K2");
    }

    #[test]
    fn project_package_dependency_rejects_non_object_and_blank_name() {
        assert!(project_package_dependency(&json("42")).is_none());
        assert!(project_package_dependency(&json(r#"{"version":"1"}"#)).is_none());
        assert!(project_package_dependency(&json(r#"{"name":"   "}"#)).is_none());
    }

    #[test]
    fn project_package_dependency_rejects_path_traversal_name() {
        // A `name` that is not a single path component would escape `packages/`
        // when interpolated into `packages/<name>.mfp` (bug-195). Reject it.
        assert!(
            project_package_dependency(&json(r#"{"name":"../../../../etc/passwd"}"#)).is_none()
        );
        assert!(project_package_dependency(&json(r#"{"name":"/etc/passwd"}"#)).is_none());
        assert!(project_package_dependency(&json(r#"{"name":"sub/dep"}"#)).is_none());
        assert!(project_package_dependency(&json(r#"{"name":".hidden"}"#)).is_none());
        // A legitimate single-component name is still accepted.
        assert!(project_package_dependency(&json(r#"{"name":"legit_pkg"}"#)).is_some());
    }

    #[test]
    fn package_metadata_and_dependencies_from_manifest() {
        let value = json(
            r#"{"name":"proj","version":"1.0","ident":"proj.id","author":"me","url":"http://x",
                "packages":[{"name":"dep","version":"3.1","pin":true},{"noname":true}]}"#,
        );
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let meta = package_metadata(manifest);
        assert_eq!(meta.name, "proj");
        assert_eq!(meta.version, "1.0");
        assert_eq!(meta.ident, "proj.id");
        assert_eq!(meta.author, "me");
        assert_eq!(meta.url, "http://x");
        // The malformed second package (no name) is filtered out.
        assert_eq!(meta.dependencies.len(), 1);
        assert_eq!(meta.dependencies[0].name, "dep");
        assert_eq!(meta.dependencies[0].version, "3.1");
        assert!(meta.dependencies[0].pin);
        // Its ident defaults to its name.
        assert_eq!(meta.dependencies[0].ident, "dep");
    }

    #[test]
    fn package_metadata_defaults_when_optional_fields_absent() {
        let value = json(r#"{"name":"p","version":"1"}"#);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let meta = package_metadata(manifest);
        assert_eq!(meta.ident, "");
        assert_eq!(meta.author, "");
        assert_eq!(meta.url, "");
        assert!(meta.dependencies.is_empty());
    }

    #[test]
    fn installed_package_files_empty_when_no_packages() {
        let value = json(r#"{"name":"p","version":"1"}"#);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let files = installed_package_files(dir.path(), manifest).expect("ok");
        assert!(files.is_empty());
    }

    #[test]
    fn installed_package_files_reports_missing_and_pin_mismatch() {
        // A declared dependency with no installed file -> "must be installed".
        let value = json(r#"{"name":"p","version":"1","packages":[{"name":"dep","version":"1"}]}"#);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let err = installed_package_files(dir.path(), manifest).unwrap_err();
        assert!(err.contains("must be installed"));

        // Install a package at the wrong version with pin -> version mismatch.
        let value = json(
            r#"{"name":"p","version":"1","packages":[{"name":"dep","version":"9.9","pin":true}]}"#,
        );
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let pkg_dir = dir.path().join("packages");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("dep.mfp"), build_mfp("dep", "1.0")).unwrap();
        let err = installed_package_files(dir.path(), manifest).unwrap_err();
        assert!(err.contains("is pinned to version"));
    }

    #[test]
    fn installed_package_files_ok_when_present_and_unpinned() {
        let value = json(r#"{"name":"p","version":"1","packages":[{"name":"dep","version":"1"}]}"#);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let pkg_dir = dir.path().join("packages");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("dep.mfp"), build_mfp("dep", "2.0")).unwrap();
        let files = installed_package_files(dir.path(), manifest).expect("ok");
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("dep.mfp"));
    }

    #[test]
    fn external_package_function_types_lossy_swallows_errors() {
        // Missing packages array -> empty maps, no error.
        let value = json(r#"{"name":"p","version":"1"}"#);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (functions, params) = external_package_function_types(dir.path(), manifest);
        assert!(functions.is_empty());
        assert!(params.is_empty());
    }

    #[test]
    fn package_export_function_type_formats_signature() {
        let export = binary_repr::BinaryReprExport {
            name: "f".to_string(),
            kind: binary_repr::BinaryReprExportKind::Func,
            isolated: false,
            params: vec![
                binary_repr::BinaryReprExportParam {
                    name: "a".to_string(),
                    type_: "Integer".to_string(),
                    has_default: false,
                },
                binary_repr::BinaryReprExportParam {
                    name: "b".to_string(),
                    type_: "String".to_string(),
                    has_default: false,
                },
            ],
            return_type: "Boolean".to_string(),
        };
        assert_eq!(
            package_export_function_type(&export),
            "FUNC(Integer, String) AS Boolean"
        );
        let isolated = binary_repr::BinaryReprExport {
            isolated: true,
            params: Vec::new(),
            ..export
        };
        assert_eq!(
            package_export_function_type(&isolated),
            "ISOLATED FUNC() AS Boolean"
        );
    }

    // --- project.json rewriting -------------------------------------------

    fn dependency(name: &str, ident_key: &str) -> ProjectPackageDependency {
        ProjectPackageDependency {
            name: name.to_string(),
            ident: format!("{name}.id"),
            version: "1.0".to_string(),
            pin: true,
            source: "file:///p.mfp".to_string(),
            ident_key: ident_key.to_string(),
        }
    }

    #[test]
    fn project_json_with_package_inserts_new_array() {
        let contents = "{\n  \"name\": \"proj\",\n  \"version\": \"1\"\n}\n";
        let value = json(contents);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let out = project_json_with_package(contents, manifest, &dependency("dep", "KEY")).unwrap();
        assert!(out.contains("\"packages\": ["));
        assert!(out.contains("\"name\": \"dep\""));
        assert!(out.contains("\"identKey\": \"KEY\""));
        // Result is still valid JSON declaring the dependency.
        let reparsed = out.parse::<JsonValue>().expect("valid json");
        assert!(reparsed
            .get::<HashMap<String, JsonValue>>()
            .unwrap()
            .contains_key("packages"));
    }

    #[test]
    fn project_json_with_package_appends_to_existing_array() {
        let contents =
            "{\n  \"name\": \"proj\",\n  \"packages\": [\n    { \"name\": \"a\" }\n  ]\n}\n";
        let value = json(contents);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        // No identKey -> the field is omitted.
        let out = project_json_with_package(contents, manifest, &dependency("b", "")).unwrap();
        assert!(out.contains("\"name\": \"a\""));
        assert!(out.contains("\"name\": \"b\""));
        assert!(!out.contains("identKey"));
        out.parse::<JsonValue>().expect("valid json");
    }

    #[test]
    fn project_json_with_package_appends_to_empty_array() {
        let contents = "{\n  \"name\": \"proj\",\n  \"packages\": []\n}\n";
        let value = json(contents);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let out = project_json_with_package(contents, manifest, &dependency("b", "")).unwrap();
        assert!(out.contains("\"name\": \"b\""));
        out.parse::<JsonValue>().expect("valid json");
    }

    #[test]
    fn project_json_with_package_rejects_duplicate() {
        let contents = "{\n  \"packages\": [\n    { \"name\": \"dep\" }\n  ]\n}\n";
        let value = json(contents);
        let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
        let err =
            project_json_with_package(contents, manifest, &dependency("dep", "")).unwrap_err();
        assert!(err.contains("already declares package"));
    }

    #[test]
    fn project_json_with_updated_ident_key_rewrites_existing() {
        let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"dep\",\n      \"identKey\": \"OLD\"\n    }\n  ]\n}\n";
        let out = project_json_with_updated_ident_key(contents, "dep", "NEW").unwrap();
        assert!(out.contains("\"identKey\": \"NEW\""));
        assert!(!out.contains("OLD"));
        out.parse::<JsonValue>().expect("valid json");
    }

    #[test]
    fn project_json_with_updated_ident_key_inserts_when_absent() {
        let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"dep\"\n    }\n  ]\n}\n";
        let out = project_json_with_updated_ident_key(contents, "dep", "NEW").unwrap();
        assert!(out.contains("\"identKey\": \"NEW\""));
        out.parse::<JsonValue>().expect("valid json");
    }

    #[test]
    fn project_json_with_updated_ident_key_errors() {
        // No packages array at all.
        let err = project_json_with_updated_ident_key("{}", "dep", "K").unwrap_err();
        assert!(err.contains("could not locate project.json `packages` array"));
        // Array present but package not declared.
        let contents = "{\n  \"packages\": [\n    { \"name\": \"other\" }\n  ]\n}\n";
        let err = project_json_with_updated_ident_key(contents, "dep", "K").unwrap_err();
        assert!(err.contains("does not declare package"));
    }

    #[test]
    fn insert_helpers_report_missing_structures() {
        // insert_package_dependency with no packages array.
        assert!(insert_package_dependency("{}", "entry")
            .unwrap_err()
            .contains("could not locate project.json `packages` array"));
        // insert_packages_array with no root object.
        assert!(insert_packages_array("", "entry")
            .unwrap_err()
            .contains("could not locate end of project.json object"));
    }

    #[test]
    fn json_scanning_primitives() {
        let src = r#"{"a": "x\"y", "b": [1, 2]}"#;
        // Field-name lookup skips string contents (the escaped quote in "x\"y").
        let a = json_field_name_position(src, "a").unwrap();
        assert_eq!(&src[a..a + 3], "\"a\"");
        let b = json_field_name_position(src, "b").unwrap();
        assert!(b > a);
        assert!(json_field_name_position(src, "missing").is_none());
        // Array bounds around "b".
        let (start, end) = json_array_bounds(src, "b").unwrap();
        assert_eq!(src.as_bytes()[start], b'[');
        assert_eq!(src.as_bytes()[end], b']');
        // find_json_punct returns None when a non-space non-target byte appears.
        assert!(find_json_punct("x:", 0, b':').is_none());
        // root_object_end and matching delimiter.
        assert!(root_object_end("  { }").is_some());
        assert!(root_object_end("no object").is_none());
        // Unbalanced braces -> None.
        assert!(matching_json_delimiter("{", 0, b'{', b'}').is_none());
        // json_string_end on a non-string start.
        assert!(json_string_end("abc", 0).is_none());
        // Unterminated string.
        assert!(json_string_end("\"abc", 0).is_none());
    }

    #[test]
    fn read_int_helpers_reject_truncation() {
        assert!(read_u16(&[0], 0).is_err());
        assert!(read_u32(&[0, 0], 0).is_err());
        assert!(read_u64(&[0, 0, 0], 0).is_err());
        assert_eq!(read_u16(&[1, 0], 0).unwrap(), 1);
        assert_eq!(read_u32(&[1, 0, 0, 0], 0).unwrap(), 1);
        assert_eq!(read_u64(&[1, 0, 0, 0, 0, 0, 0, 0], 0).unwrap(), 1);
    }

    // --- real compiled .mfp fixture ---------------------------------------

    /// A committed, structurally-valid compiled package fixture.
    fn fixture_mfp() -> PathBuf {
        crate::testutil::fixture_dir("package-simple")
            .join("golden")
            .join("package_simple.mfp")
    }

    #[test]
    fn read_mfp_header_reads_real_package_fixture() {
        let header = read_mfp_header(&fixture_mfp()).expect("valid fixture header");
        assert_eq!(header.name, "package_simple");
        assert_eq!(header.container_major, 1);
        assert_eq!(header.container_minor, 0);
        assert!(header.binary_repr_length > 0);
    }

    #[test]
    fn external_package_function_types_from_files_reads_exports() {
        // The strict variant reads the real fixture's exported function types.
        let (functions, params) =
            external_package_function_types_from_files(&[fixture_mfp()]).expect("reads exports");
        assert!(
            !functions.is_empty(),
            "fixture should export at least one function"
        );
        // Every exported function name is package-qualified and has a params entry.
        for name in functions.keys() {
            assert!(name.starts_with("package_simple."), "{name}");
            assert!(params.contains_key(name), "missing params for {name}");
        }
        // Signatures are FUNC(...) AS ... shaped.
        assert!(functions.values().all(|sig| sig.contains("FUNC(")));
    }

    #[test]
    fn external_package_function_types_lossy_matches_strict_for_valid_files() {
        let strict = external_package_function_types_from_files(&[fixture_mfp()]).unwrap();
        let lossy = external_package_function_types_from_files_lossy(&[fixture_mfp()]);
        assert_eq!(strict.0.len(), lossy.0.len());
        assert_eq!(strict.1.len(), lossy.1.len());
    }

    #[test]
    fn external_package_function_types_lossy_skips_bad_files() {
        // A path that is not a valid .mfp is silently skipped by the lossy path.
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.mfp");
        fs::write(&bad, b"not an mfp").unwrap();
        let (functions, params) = external_package_function_types_from_files_lossy(&[bad]);
        assert!(functions.is_empty());
        assert!(params.is_empty());
    }

    #[test]
    fn external_package_function_types_from_files_propagates_error() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.mfp");
        fs::write(&bad, b"not an mfp").unwrap();
        assert!(external_package_function_types_from_files(&[bad]).is_err());
    }

    #[test]
    fn read_mfp_header_rejects_ed25519_wrong_signature_length() {
        // signature type 1 (Ed25519) but a length other than 64 is rejected.
        let mut bytes = build_mfp("p", "1");
        let len = bytes.len();
        bytes[len - 6..len - 4].copy_from_slice(&1u16.to_le_bytes()); // type = 1
        bytes[len - 4..].copy_from_slice(&10u32.to_le_bytes()); // length = 10
        let (_dir, path) = write_temp(&bytes);
        let err = header_err(&path);
        assert!(err.contains("Ed25519 .mfp package must have a 64 byte signature"));
    }

    #[test]
    fn read_mfp_header_rejects_truncated_binary_hash() {
        // Cut the buffer right after the last variable-length field so the fixed
        // 32-byte packageBinaryHash read runs past the end.
        let full = build_mfp("p", "1");
        // The trailing fixed section is 32 (hash) + 8 (len) + 2 + 4 = 46 bytes.
        let bytes = full[..full.len() - 46 + 4].to_vec();
        let (_dir, path) = write_temp(&bytes);
        assert!(read_mfp_header(&path).is_err());
    }

    #[test]
    fn project_json_with_updated_ident_key_rewrites_snake_case_field() {
        // A pre-existing snake_case `ident_key` field is rewritten in place.
        let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"dep\",\n      \"ident_key\": \"OLD\"\n    }\n  ]\n}\n";
        let out = project_json_with_updated_ident_key(contents, "dep", "NEW").unwrap();
        assert!(out.contains("\"NEW\""));
        assert!(!out.contains("OLD"));
        out.parse::<JsonValue>().expect("valid json");
    }

    #[test]
    fn project_json_with_updated_ident_key_rejects_malformed_entry() {
        // The packages array bounds resolve, but the first `{` inside never
        // closes (no `}` anywhere after it), so matching_json_delimiter returns
        // None -> "malformed project.json `packages` entry".
        let contents = "{ \"packages\": [ { \"name\": \"dep\" ] ";
        let err = project_json_with_updated_ident_key(contents, "dep", "K").unwrap_err();
        assert!(
            err.contains("malformed project.json `packages` entry"),
            "got: {err}"
        );
    }

    #[test]
    fn read_mfp_string_rejects_invalid_utf8() {
        // A name field with invalid UTF-8 bytes is rejected.
        let mut buf = Vec::new();
        buf.extend_from_slice(&MFP_MAGIC);
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        push_field(&mut buf, &[0xff, 0xfe]); // invalid UTF-8 name
        let (_dir, path) = write_temp(&buf);
        let err = header_err(&path);
        assert!(err.contains("not valid UTF-8"));
    }
}
