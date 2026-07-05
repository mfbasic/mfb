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

    let binary_repr_length = read_u64(&bytes, offset)? as usize;
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
