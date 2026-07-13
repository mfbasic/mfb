use super::*;

/// Maximum depth of the composite type-reference graph an untrusted `.mfp` may
/// encode. Real type graphs are shallow; a crafted long *linear* chain of
/// distinct composite types (List OF id → List OF id → …) would otherwise recurse
/// one native stack frame per link and overflow the stack before any signature is
/// trusted (bug-153). The cycle guards (`in_progress` / `type_refs`) only reject
/// *repeated* ids, not a deep acyclic chain, so a separate depth cap is required.
pub(super) const MAX_TYPE_GRAPH_DEPTH: usize = 256;

pub(super) fn doc_kind_name(kind: u16) -> &'static str {
    match kind {
        DOC_KIND_SUB => "sub",
        DOC_KIND_TYPE => "type",
        DOC_KIND_UNION => "union",
        DOC_KIND_ENUM => "enum",
        _ => "func",
    }
}

pub(super) fn encode_doc_table(docs: &PackageDocs) -> Vec<u8> {
    let mut bytes = Vec::new();
    match &docs.package {
        Some(package) => {
            bytes.push(1);
            put_bytes(&mut bytes, package.name.as_bytes());
            put_prose_list(&mut bytes, &package.desc);
            put_optional_str(&mut bytes, &package.deprecated);
        }
        None => bytes.push(0),
    }
    put_u32(&mut bytes, docs.decls.len() as u32);
    for decl in &docs.decls {
        let kind = match decl.kind.as_str() {
            "sub" => DOC_KIND_SUB,
            "type" => DOC_KIND_TYPE,
            "union" => DOC_KIND_UNION,
            "enum" => DOC_KIND_ENUM,
            _ => DOC_KIND_FUNC,
        };
        put_u16(&mut bytes, kind);
        put_bytes(&mut bytes, decl.name.as_bytes());
        put_bytes(&mut bytes, decl.signature.as_bytes());
        put_bytes(&mut bytes, decl.group.as_bytes());
        put_prose_list(&mut bytes, &decl.desc);
        put_pair_list(&mut bytes, &decl.args);
        put_pair_list(&mut bytes, &decl.props);
        put_bytes(&mut bytes, decl.ret.as_bytes());
        put_pair_list(&mut bytes, &decl.errors);
        put_bytes(&mut bytes, decl.example.as_bytes());
        bytes.push(u8::from(decl.internal));
        put_optional_str(&mut bytes, &decl.deprecated);
    }
    bytes
}

pub(super) fn read_doc_table(bytes: &[u8]) -> Result<PackageDocs, String> {
    let mut offset = 0;
    let has_package = *bytes
        .get(offset)
        .ok_or_else(|| "truncated doc table".to_string())?;
    offset += 1;
    let package = if has_package == 0 {
        None
    } else {
        let name = cursor_string(bytes, &mut offset)?;
        let desc = cursor_prose_list(bytes, &mut offset)?;
        let deprecated = cursor_optional_str(bytes, &mut offset)?;
        Some(PackageDocEntry {
            name,
            desc,
            deprecated,
        })
    };
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut decls = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 2));
    for _ in 0..count {
        let kind = doc_kind_name(cursor_u16(bytes, &mut offset)?).to_string();
        let name = cursor_string(bytes, &mut offset)?;
        let signature = cursor_string(bytes, &mut offset)?;
        let group = cursor_string(bytes, &mut offset)?;
        let desc = cursor_prose_list(bytes, &mut offset)?;
        let args = cursor_pair_list(bytes, &mut offset)?;
        let props = cursor_pair_list(bytes, &mut offset)?;
        let ret = cursor_string(bytes, &mut offset)?;
        let errors = cursor_pair_list(bytes, &mut offset)?;
        let example = cursor_string(bytes, &mut offset)?;
        let internal = *bytes
            .get(offset)
            .ok_or_else(|| "truncated doc entry".to_string())?
            != 0;
        offset += 1;
        let deprecated = cursor_optional_str(bytes, &mut offset)?;
        decls.push(DeclDocEntry {
            kind,
            name,
            signature,
            group,
            desc,
            args,
            props,
            ret,
            errors,
            example,
            internal,
            deprecated,
        });
    }
    Ok(PackageDocs { package, decls })
}

pub(super) fn docs_from_ir(docs: &crate::ir::ProjectDocs) -> PackageDocs {
    use crate::ir::IrDocKind;
    let package = docs.package.as_ref().map(|package| PackageDocEntry {
        name: package.name.clone(),
        desc: package.desc.clone(),
        deprecated: package.deprecated.clone(),
    });
    let decls = docs
        .decls
        .iter()
        .map(|decl| DeclDocEntry {
            kind: match decl.kind {
                IrDocKind::Func => "func",
                IrDocKind::Sub => "sub",
                IrDocKind::Type => "type",
                IrDocKind::Union => "union",
                IrDocKind::Enum => "enum",
            }
            .to_string(),
            name: decl.name.clone(),
            signature: decl.signature.clone(),
            group: decl.group.clone(),
            desc: decl.desc.clone(),
            args: decl.args.clone(),
            props: decl.props.clone(),
            ret: decl.ret.clone(),
            errors: decl.errors.clone(),
            example: decl.example.clone(),
            internal: decl.internal,
            deprecated: decl.deprecated.clone(),
        })
        .collect();
    PackageDocs { package, decls }
}

/// Deterministic per-package identity prefix segment (`<id>` in
/// `<id>.package.symbol`): a 16-hex-char content hash over the package's
/// manifest identity (name, version, ident) and its inner binary_repr payload.
///
/// Being a pure content hash, the same package always yields the same id —
/// giving reproducible builds and letting a diamond dependency de-duplicate to
/// a single copy — while differing content yields a differing id, keeping two
/// distinct packages (e.g. a version conflict) from colliding at merge time.
pub(super) fn package_identity_id(identity: &MfpIdentity, payload: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    for field in [&identity.name, &identity.version, &identity.ident] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.update(payload);
    let digest = hasher.finalize();
    let mut id = String::with_capacity(16);
    for byte in &digest[..8] {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

pub(super) fn read_package_binary_repr(path: &Path) -> Result<PackageBinaryRepr, String> {
    let package =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let container = mfp_binary_repr_payload(&package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let package = read_binary_repr_package(container.binary_repr)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    validate_container_manifest_identity(&container.identity, &package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    Ok(package)
}

pub(super) fn mfp_binary_repr_payload(bytes: &[u8]) -> Result<MfpContainer<'_>, String> {
    const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
    if bytes.len() < 20 {
        return Err("package is too small to be a valid .mfp package".to_string());
    }
    if bytes[0..8] != MFP_MAGIC {
        return Err("package does not have the MFP package magic".to_string());
    }
    // Container v1.0 (plan-23 §4), hard: the reader accepts exactly 1.0.
    let container_major = checked_u16_at(bytes, 8)?;
    let container_minor = checked_u16_at(bytes, 10)?;
    if container_major != 1 || container_minor != 0 {
        return Err(format!(
            "unsupported MFP container version {container_major}.{container_minor} (expected 1.0)"
        ));
    }

    let mut offset = 20usize;
    let name = read_length_prefixed(bytes, &mut offset, "name")?;
    let ident = read_length_prefixed(bytes, &mut offset, "ident")?;
    let version = read_length_prefixed(bytes, &mut offset, "version")?;
    skip_length_prefixed(bytes, &mut offset, "author")?;
    skip_length_prefixed(bytes, &mut offset, "url")?;
    let ident_key = read_length_prefixed(bytes, &mut offset, "identKey")?;
    let signing_key = read_length_prefixed(bytes, &mut offset, "signingKey")?;
    skip_length_prefixed(bytes, &mut offset, "proof")?;
    skip_length_prefixed(bytes, &mut offset, "proofSig")?;
    skip_length_prefixed(bytes, &mut offset, "attestation")?;
    skip_length_prefixed(bytes, &mut offset, "attestationSig")?;
    // packageBinaryHash: 32 raw bytes, no length prefix.
    offset = offset
        .checked_add(32)
        .ok_or_else(|| "truncated .mfp packageBinaryHash".to_string())?;
    if offset > bytes.len() {
        return Err("truncated .mfp packageBinaryHash".to_string());
    }
    let binary_repr_length = checked_usize(
        checked_u64_at(bytes, offset)?,
        ".mfp binary representation length",
    )?;
    offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;
    let signature_type = checked_u16_at(bytes, offset)?;
    let signature_length = checked_u32_at(bytes, offset + 2)? as usize;
    validate_mfp_signature_header(signature_type, signature_length)?;
    offset = offset
        .checked_add(6)
        .and_then(|offset| offset.checked_add(signature_length))
        .ok_or_else(|| "invalid .mfp signature length".to_string())?;
    if offset > bytes.len() {
        return Err("truncated .mfp signature".to_string());
    }
    let end = offset
        .checked_add(binary_repr_length)
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;
    if end != bytes.len() {
        return Err("invalid .mfp binary representation length".to_string());
    }
    Ok(MfpContainer {
        identity: MfpIdentity {
            name,
            ident,
            version,
            ident_key,
            signing_key,
        },
        binary_repr: &bytes[offset..end],
    })
}

pub(super) fn validate_mfp_signature_header(
    signature_type: u16,
    signature_length: usize,
) -> Result<(), String> {
    match (signature_type, signature_length) {
        (0, 0) | (1, 64) => Ok(()),
        (0, _) => Err("unsigned .mfp package must have zero signature length".to_string()),
        (1, _) => Err("Ed25519 .mfp package must have a 64 byte signature".to_string()),
        _ => Err(format!("unsupported .mfp signature type {signature_type}")),
    }
}

pub(super) fn validate_container_manifest_identity(
    identity: &MfpIdentity,
    package: &PackageBinaryRepr,
) -> Result<(), String> {
    let strings = &package.project.strings.values;
    let manifest = &package.project.manifest;
    let manifest_name = string_at(strings, manifest.package_name)?;
    let manifest_ident = string_at(strings, manifest.package_ident)?;
    let manifest_version = string_at(strings, manifest.package_version)?;
    let manifest_ident_key = string_at(strings, manifest.ident_key)?;
    let manifest_ident_fingerprint = string_at(strings, manifest.ident_fingerprint)?;
    let manifest_signing_fingerprint = string_at(strings, manifest.signing_fingerprint)?;
    // The manifest repeats the header identity (plan-23 §4): the full ident
    // key string plus the SHA-256 fingerprints of the header's identKey and
    // signingKey (fingerprints are derived, no longer header fields).
    let header_ident_fingerprint =
        mfb_repository::package::metadata_key_fingerprint(&identity.ident_key, "identKey")?;
    let header_signing_fingerprint =
        mfb_repository::package::metadata_key_fingerprint(&identity.signing_key, "signingKey")?;
    if identity.name != manifest_name
        || identity.ident != manifest_ident
        || identity.version != manifest_version
        || identity.ident_key != manifest_ident_key
        || header_ident_fingerprint != manifest_ident_fingerprint
        || header_signing_fingerprint != manifest_signing_fingerprint
    {
        return Err(
            "MFP header identity does not match binary representation manifest identity"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn read_binary_repr_package(bytes: &[u8]) -> Result<PackageBinaryRepr, String> {
    if bytes.len() < 16 || &bytes[0..4] != b"MFPC" {
        return Err(
            "package payload does not have the binary representation container magic".to_string(),
        );
    }
    let major = checked_u16_at(bytes, 4)?;
    if major != MFPC_MAJOR_VERSION {
        return Err(format!(
            "unsupported MFPC major version {major} (expected {MFPC_MAJOR_VERSION}); \
             this package predates the structured Binary Representation format and must be rebuilt"
        ));
    }
    let section_count = checked_u32_at(bytes, 12)? as usize;
    let table_end = 16usize
        .checked_add(
            section_count
                .checked_mul(24)
                .ok_or_else(|| "invalid MFPC section table length".to_string())?,
        )
        .ok_or_else(|| "invalid MFPC section table length".to_string())?;
    if table_end > bytes.len() {
        return Err("truncated MFPC section table".to_string());
    }

    let mut sections = HashMap::new();
    for index in 0..section_count {
        let entry = 16 + index * 24;
        let id = checked_u16_at(bytes, entry)?;
        let offset = checked_usize(checked_u64_at(bytes, entry + 8)?, "MFPC section offset")?;
        let length = checked_usize(checked_u64_at(bytes, entry + 16)?, "MFPC section length")?;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid MFPC section length".to_string())?;
        if end > bytes.len() {
            return Err("truncated MFPC section".to_string());
        }
        // Reject duplicate section ids (PKG-06). A `HashMap::insert` silently
        // keeps the last copy, letting a crafted package ship two views of a
        // singleton section (e.g. two BINARY_REPR/ABI_INDEX) — one to satisfy a
        // cheap inspector, the other to be decoded and lowered. Every MFPC
        // section is a singleton, so a repeated id is always tampering.
        if sections.insert(id, &bytes[offset..end]).is_some() {
            return Err(format!("duplicate MFPC section id {id}"));
        }
    }

    let string_values = read_string_pool(
        sections
            .get(&SECTION_STRING_POOL)
            .copied()
            .ok_or_else(|| "MFPC is missing the string pool section".to_string())?,
    )?;
    let strings = StringPool {
        values: string_values,
    };
    let types = read_type_entries(
        sections
            .get(&SECTION_TYPE_TABLE)
            .copied()
            .ok_or_else(|| "MFPC is missing the type table section".to_string())?,
        &strings.values,
    )?;
    let type_names = type_entry_names(&types, &strings.values)?;
    let constants = read_const_pool(
        sections
            .get(&SECTION_CONST_POOL)
            .copied()
            .ok_or_else(|| "MFPC is missing the const pool section".to_string())?,
    )?;
    let functions = read_function_table(
        sections
            .get(&SECTION_FUNCTION_TABLE)
            .copied()
            .ok_or_else(|| "MFPC is missing the function table section".to_string())?,
        // Function bodies are carried by SECTION_BINARY_REPR (structured Binary Representation), not a
        // flat code section; the function table records zero-length code regions.
        &[],
        &strings.values,
        &type_names,
    )?;
    let binary_repr = sections
        .get(&SECTION_BINARY_REPR)
        .copied()
        .ok_or_else(|| "MFPC is missing the Binary Representation section".to_string())?
        .to_vec();
    let exports = read_export_table(
        sections
            .get(&SECTION_EXPORT_TABLE)
            .copied()
            .ok_or_else(|| "MFPC is missing the export table section".to_string())?,
    )?;
    let resources = match sections.get(&SECTION_RESOURCE_TABLE).copied() {
        Some(section) => read_resource_table(section)?,
        None => ResourceTable::new(),
    };
    let globals = match sections.get(&SECTION_GLOBAL_TABLE).copied() {
        Some(section) => read_global_table(section)?,
        None => Vec::new(),
    };
    let docs = match sections.get(&SECTION_DOC_TABLE).copied() {
        Some(section) => read_doc_table(section)?,
        None => PackageDocs::default(),
    };
    let manifest = read_manifest(
        sections
            .get(&SECTION_MANIFEST)
            .copied()
            .ok_or_else(|| "MFPC is missing the manifest section".to_string())?,
    )?;
    let imports = read_import_table(
        sections
            .get(&SECTION_IMPORT_TABLE)
            .copied()
            .ok_or_else(|| "MFPC is missing the import table section".to_string())?,
    )?;
    let abi = read_abi_index(
        sections
            .get(&SECTION_ABI_INDEX)
            .copied()
            .ok_or_else(|| "MFPC is missing the ABI_INDEX section".to_string())?,
    )?;
    validate_abi_index(
        &abi,
        &exports,
        &imports,
        &strings.values,
        &types,
        &constants,
        &functions,
    )?;

    Ok(PackageBinaryRepr {
        project: BinaryReprProject {
            strings,
            types,
            constants,
            resources,
            globals,
            manifest,
            imports,
            abi,
            entry_function: u32::MAX,
            entry_flags: 0,
            functions,
            binary_repr,
            docs,
        },
        exports,
    })
}

pub(super) fn decode_type_export(
    name: &str,
    kind: BinaryReprExportKind,
    entry: &TypeEntry,
    type_names: &HashMap<u32, String>,
    strings: &[String],
) -> Result<BinaryReprTypeExport, String> {
    let mut offset = 0usize;
    let (fields, variants, members) = match kind {
        BinaryReprExportKind::Type => {
            let field_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut fields = Vec::with_capacity(bounded_capacity(
                field_count,
                entry.payload.len() - offset,
                12,
            ));
            for _ in 0..field_count {
                fields.push(decode_type_field(
                    &entry.payload,
                    &mut offset,
                    type_names,
                    strings,
                )?);
            }
            (fields, Vec::new(), Vec::new())
        }
        BinaryReprExportKind::Union => {
            let variant_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut variants = Vec::with_capacity(bounded_capacity(
                variant_count,
                entry.payload.len() - offset,
                8,
            ));
            for _ in 0..variant_count {
                let variant_name =
                    string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string();
                let field_count = cursor_u32(&entry.payload, &mut offset)? as usize;
                let mut fields = Vec::with_capacity(bounded_capacity(
                    field_count,
                    entry.payload.len() - offset,
                    8,
                ));
                for _ in 0..field_count {
                    let field_name =
                        string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string();
                    let field_type =
                        type_name(type_names, cursor_u32(&entry.payload, &mut offset)?)?
                            .to_string();
                    fields.push(BinaryReprTypeField {
                        name: field_name,
                        type_: field_type,
                        visibility: BinaryReprTypeVisibility::Export,
                    });
                }
                variants.push(BinaryReprTypeVariant {
                    name: variant_name,
                    fields,
                });
            }
            (Vec::new(), variants, Vec::new())
        }
        BinaryReprExportKind::Enum => {
            let member_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut members = Vec::with_capacity(bounded_capacity(
                member_count,
                entry.payload.len() - offset,
                8,
            ));
            for _ in 0..member_count {
                members.push(
                    string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string(),
                );
                let _ordinal = cursor_u32(&entry.payload, &mut offset)?;
            }
            (Vec::new(), Vec::new(), members)
        }
        BinaryReprExportKind::Func | BinaryReprExportKind::Sub => {
            return Err(format!("export `{name}` is not a type export"));
        }
    };
    if offset != entry.payload.len() {
        return Err(format!("exported type `{name}` has trailing payload bytes"));
    }
    Ok(BinaryReprTypeExport {
        name: name.to_string(),
        kind,
        fields,
        variants,
        members,
    })
}

pub(super) fn decode_type_field(
    payload: &[u8],
    offset: &mut usize,
    type_names: &HashMap<u32, String>,
    strings: &[String],
) -> Result<BinaryReprTypeField, String> {
    let name = string_at(strings, cursor_u32(payload, offset)?)?.to_string();
    let type_ = type_name(type_names, cursor_u32(payload, offset)?)?.to_string();
    let visibility = match cursor_u32(payload, offset)? {
        0 => BinaryReprTypeVisibility::Export,
        1 => BinaryReprTypeVisibility::Private,
        2 => BinaryReprTypeVisibility::Public,
        3 => BinaryReprTypeVisibility::Export,
        other => return Err(format!("unsupported type field visibility {other}")),
    };
    Ok(BinaryReprTypeField {
        name,
        type_,
        visibility,
    })
}

pub(super) fn read_string_pool(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut strings = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 4));
    for _ in 0..count {
        let length = cursor_u32(bytes, &mut offset)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid string pool entry length".to_string())?;
        if end > bytes.len() {
            return Err("truncated string pool entry".to_string());
        }
        strings.push(
            std::str::from_utf8(&bytes[offset..end])
                .map_err(|_| "string pool entry is not valid UTF-8".to_string())?
                .to_string(),
        );
        offset = end;
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in string pool".to_string());
    }
    Ok(strings)
}

pub(super) fn read_type_entries(bytes: &[u8], strings: &[String]) -> Result<TypeTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let entries_end = 4usize
        .checked_add(
            count
                .checked_mul(20)
                .ok_or_else(|| "invalid type table length".to_string())?,
        )
        .ok_or_else(|| "invalid type table length".to_string())?;
    if entries_end > bytes.len() {
        return Err("truncated type table".to_string());
    }

    let mut entries = Vec::with_capacity(count);
    let mut ids = HashMap::new();
    for index in 0..count {
        let kind = cursor_u16(bytes, &mut offset)?;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let name = cursor_u32(bytes, &mut offset)?;
        let owner_package = cursor_u32(bytes, &mut offset)?;
        let payload_offset = cursor_u32(bytes, &mut offset)? as usize;
        let payload_length = cursor_u32(bytes, &mut offset)? as usize;
        let payload_end = payload_offset
            .checked_add(payload_length)
            .ok_or_else(|| "invalid type payload length".to_string())?;
        if payload_offset < entries_end || payload_end > bytes.len() {
            return Err("invalid type payload bounds".to_string());
        }
        let id = FIRST_TABLE_TYPE_ID + index as u32;
        ids.insert(string_at(strings, name)?.to_string(), id);
        entries.push(TypeEntry {
            kind,
            name,
            owner_package,
            abi_export_kind: None,
            payload: bytes[payload_offset..payload_end].to_vec(),
        });
    }

    Ok(TypeTable { entries, ids })
}

pub(super) fn type_entry_names(
    types: &TypeTable,
    strings: &[String],
) -> Result<HashMap<u32, String>, String> {
    let raw = types
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            (
                FIRST_TABLE_TYPE_ID + index as u32,
                (entry.kind, entry.name, entry.payload.clone()),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut decoded = HashMap::new();
    let mut in_progress = HashSet::new();
    for id in raw.keys().copied().collect::<Vec<_>>() {
        let name = decode_type_name(id, &raw, strings, &mut decoded, &mut in_progress)?;
        decoded.insert(id, name);
    }
    Ok(decoded)
}

pub(super) fn decode_type_name(
    id: u32,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
    in_progress: &mut HashSet<u32>,
) -> Result<String, String> {
    if let Some(name) = primitive_type_name(id) {
        return Ok(name.to_string());
    }
    if let Some(name) = decoded.get(&id) {
        return Ok(name.clone());
    }
    // Cycle guard (PKG-04): a composite type whose payload references its own id
    // (directly or via a mutual reference) would otherwise recurse forever until
    // the stack overflows. Mark the id in-progress *before* recursing — mirroring
    // `AbiSerializer::serialize_type`'s `type_refs` guard — and reject re-entry.
    if !in_progress.insert(id) {
        return Err(format!("cyclic type id {id}"));
    }
    // Depth cap (bug-153): `in_progress` holds exactly the ids on the active
    // recursion path (each is removed as its subtree unwinds), so its size is the
    // current depth. A deep-but-acyclic chain passes the cycle guard above but
    // must still be rejected before it overflows the native stack.
    if in_progress.len() > MAX_TYPE_GRAPH_DEPTH {
        in_progress.remove(&id);
        return Err(format!(
            "type graph too deep (exceeds {MAX_TYPE_GRAPH_DEPTH})"
        ));
    }
    let result = decode_type_name_body(id, raw, strings, decoded, in_progress);
    in_progress.remove(&id);
    let decoded_name = result?;
    decoded.insert(id, decoded_name.clone());
    Ok(decoded_name)
}

fn decode_type_name_body(
    id: u32,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
    in_progress: &mut HashSet<u32>,
) -> Result<String, String> {
    let Some((kind, name, payload)) = raw.get(&id) else {
        return Err(format!("unknown type id {id}"));
    };
    let decoded_name = match *kind {
        4 => {
            let element = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            format!("List OF {element}")
        }
        5 => {
            let key = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let value = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            format!("Map OF {key} TO {value}")
        }
        6 => {
            let success = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            format!("Result OF {success}")
        }
        7 => {
            let message = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let output = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            let resource = if payload.len() >= 12 {
                Some(read_payload_type(
                    payload,
                    8,
                    raw,
                    strings,
                    decoded,
                    in_progress,
                )?)
            } else {
                None
            };
            builtins::thread::format_thread_type("Thread", &message, resource.as_deref(), &output)
        }
        8 => decode_function_type(payload, raw, strings, decoded, in_progress)?,
        9 => {
            let key = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let value = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            format!("MapEntry OF {key} TO {value}")
        }
        10 => {
            let message = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let output = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            let resource = if payload.len() >= 12 {
                Some(read_payload_type(
                    payload,
                    8,
                    raw,
                    strings,
                    decoded,
                    in_progress,
                )?)
            } else {
                None
            };
            builtins::thread::format_thread_type(
                "ThreadWorker",
                &message,
                resource.as_deref(),
                &output,
            )
        }
        _ => string_at(strings, *name)?.to_string(),
    };
    Ok(decoded_name)
}

pub(super) fn decode_function_type(
    payload: &[u8],
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
    in_progress: &mut HashSet<u32>,
) -> Result<String, String> {
    let mut offset = 0;
    let isolated = cursor_u32(payload, &mut offset)? != 0;
    let param_count = cursor_u32(payload, &mut offset)? as usize;
    let return_type = cursor_u32(payload, &mut offset)?;
    let returns = decode_type_name(return_type, raw, strings, decoded, in_progress)?;
    let mut params = Vec::with_capacity(bounded_capacity(param_count, payload.len() - offset, 4));
    for _ in 0..param_count {
        let param = cursor_u32(payload, &mut offset)?;
        params.push(decode_type_name(param, raw, strings, decoded, in_progress)?);
    }
    let prefix = if isolated { "ISOLATED FUNC" } else { "FUNC" };
    Ok(format!("{prefix}({}) AS {returns}", params.join(", ")))
}

pub(super) fn read_payload_type(
    payload: &[u8],
    offset: usize,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
    in_progress: &mut HashSet<u32>,
) -> Result<String, String> {
    let id = checked_u32_at(payload, offset)?;
    decode_type_name(id, raw, strings, decoded, in_progress)
}

pub(super) fn primitive_type_name(id: u32) -> Option<&'static str> {
    match id {
        TYPE_NOTHING => Some("Nothing"),
        TYPE_BOOLEAN => Some("Boolean"),
        TYPE_INTEGER => Some("Integer"),
        TYPE_FLOAT => Some("Float"),
        TYPE_FIXED => Some("Fixed"),
        TYPE_MONEY => Some("Money"),
        TYPE_STRING => Some("String"),
        TYPE_BYTE => Some("Byte"),
        TYPE_ERROR => Some("Error"),
        TYPE_TERM_COLOR => Some("TermColor"),
        TYPE_TERM_SIZE => Some("TermSize"),
        TYPE_FILE_HANDLE => Some("File"),
        TYPE_SOCKET_HANDLE => Some("Socket"),
        TYPE_LISTENER_HANDLE => Some("Listener"),
        _ => None,
    }
}

pub(super) fn read_function_table(
    bytes: &[u8],
    code: &[u8],
    strings: &[String],
    _types: &HashMap<u32, String>,
) -> Result<Vec<Function>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut functions = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 4));
    for _ in 0..count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = cursor_u16(bytes, &mut offset)?;
        let flags = cursor_u16(bytes, &mut offset)?;
        let param_count = cursor_u32(bytes, &mut offset)? as usize;
        let return_type = cursor_u32(bytes, &mut offset)?;
        let register_count = cursor_u32(bytes, &mut offset)? as usize;
        let code_offset = checked_usize(cursor_u64(bytes, &mut offset)?, "function code offset")?;
        let code_length = checked_usize(cursor_u64(bytes, &mut offset)?, "function code length")?;
        let _source_map = cursor_u32(bytes, &mut offset)?;
        let cleanup_count = cursor_u32(bytes, &mut offset)? as usize;
        let _cleanup_offset = cursor_u64(bytes, &mut offset)?;

        let mut params =
            Vec::with_capacity(bounded_capacity(param_count, bytes.len() - offset, 16));
        for _ in 0..param_count {
            let param_name = cursor_u32(bytes, &mut offset)?;
            let _ = string_at(strings, param_name)?;
            let param_type = cursor_u32(bytes, &mut offset)?;
            let param_flags = cursor_u32(bytes, &mut offset)?;
            let default_const = cursor_u32(bytes, &mut offset)?;
            params.push(Param {
                name: param_name,
                type_id: param_type,
                flags: param_flags,
                default_const,
            });
        }
        let mut registers =
            Vec::with_capacity(bounded_capacity(register_count, bytes.len() - offset, 8));
        for _ in 0..register_count {
            registers.push(Register {
                type_id: cursor_u32(bytes, &mut offset)?,
                flags: cursor_u32(bytes, &mut offset)?,
            });
        }
        let mut cleanups =
            Vec::with_capacity(bounded_capacity(cleanup_count, bytes.len() - offset, 24));
        for _ in 0..cleanup_count {
            cleanups.push(Cleanup {
                id: cursor_u32(bytes, &mut offset)?,
                start_pc: cursor_u32(bytes, &mut offset)?,
                end_pc: cursor_u32(bytes, &mut offset)?,
                resource_register: cursor_u32(bytes, &mut offset)?,
                close_function_id: cursor_u32(bytes, &mut offset)?,
                flags: cursor_u32(bytes, &mut offset)?,
            });
        }

        let code_end = code_offset
            .checked_add(code_length)
            .ok_or_else(|| "invalid function code length".to_string())?;
        if code_end > code.len() {
            return Err("truncated function code".to_string());
        }
        // Function bodies live in SECTION_BINARY_REPR (structured Binary Representation), so the flat
        // code region is always empty here.
        if code_length != 0 {
            return Err("flat function code stream is no longer supported".to_string());
        }
        functions.push(Function {
            name,
            kind,
            flags,
            return_type,
            params,
            registers,
            cleanups,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in function table".to_string());
    }
    Ok(functions)
}

pub(super) fn read_const_pool(bytes: &[u8]) -> Result<ConstPool, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 8));
    for _ in 0..count {
        let kind = cursor_u16(bytes, &mut offset)?;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let length = cursor_u32(bytes, &mut offset)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid const payload length".to_string())?;
        if end > bytes.len() {
            return Err("truncated const payload".to_string());
        }
        entries.push(ConstEntry {
            kind,
            payload: bytes[offset..end].to_vec(),
        });
        offset = end;
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in const pool".to_string());
    }
    Ok(ConstPool { entries })
}

pub(super) fn read_manifest(bytes: &[u8]) -> Result<BinaryReprManifest, String> {
    let mut offset = 0;
    let manifest = BinaryReprManifest {
        package_name: cursor_u32(bytes, &mut offset)?,
        package_ident: cursor_u32(bytes, &mut offset)?,
        package_version: cursor_u32(bytes, &mut offset)?,
        ident_key: cursor_u32(bytes, &mut offset)?,
        ident_fingerprint: cursor_u32(bytes, &mut offset)?,
        signing_fingerprint: cursor_u32(bytes, &mut offset)?,
        author: cursor_u32(bytes, &mut offset)?,
        url: cursor_u32(bytes, &mut offset)?,
    };
    let _binary_repr_major = cursor_u16(bytes, &mut offset)?;
    let _binary_repr_minor = cursor_u16(bytes, &mut offset)?;
    let _language_major = cursor_u16(bytes, &mut offset)?;
    let _language_minor = cursor_u16(bytes, &mut offset)?;
    let _minimum_runtime_major = cursor_u16(bytes, &mut offset)?;
    let _minimum_runtime_minor = cursor_u16(bytes, &mut offset)?;
    let _dependency_count = cursor_u32(bytes, &mut offset)?;
    let _native_link_count = cursor_u32(bytes, &mut offset)?;
    let _export_count = cursor_u32(bytes, &mut offset)?;
    let _entry_function = cursor_u32(bytes, &mut offset)?;
    let _entry_flags = cursor_u32(bytes, &mut offset)?;
    if offset != bytes.len() {
        return Err("invalid trailing bytes in manifest".to_string());
    }
    Ok(manifest)
}

pub(super) fn read_import_table(bytes: &[u8]) -> Result<ImportTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 21));
    for _ in 0..count {
        entries.push(ImportEntry {
            package_name: cursor_u32(bytes, &mut offset)?,
            package_ident: cursor_u32(bytes, &mut offset)?,
            version: cursor_u32(bytes, &mut offset)?,
            pin: match cursor_u8(bytes, &mut offset)? {
                0 => false,
                1 => true,
                other => return Err(format!("unsupported import pin value {other}")),
            },
            flags: cursor_u32(bytes, &mut offset)?,
            used_symbols: read_used_symbols(bytes, &mut offset)?,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in import table".to_string());
    }
    Ok(ImportTable { entries })
}

pub(super) fn read_used_symbols(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<Vec<AbiUsedSymbol>, String> {
    let count = cursor_u32(bytes, offset)? as usize;
    let mut symbols = Vec::with_capacity(bounded_capacity(count, bytes.len() - *offset, 36));
    for _ in 0..count {
        symbols.push(AbiUsedSymbol {
            name: cursor_u32(bytes, offset)?,
            sig_hash: cursor_hash(bytes, offset)?,
        });
    }
    Ok(symbols)
}

pub(super) fn read_resource_table(bytes: &[u8]) -> Result<ResourceTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 12));
    for _ in 0..count {
        entries.push(ResourceEntry {
            type_id: cursor_u32(bytes, &mut offset)?,
            close_function_id: cursor_u32(bytes, &mut offset)?,
            flags: cursor_u32(bytes, &mut offset)?,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in resource table".to_string());
    }
    Ok(ResourceTable { entries })
}

pub(super) fn read_global_table(bytes: &[u8]) -> Result<Vec<GlobalEntry>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut globals = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 12));
    for _ in 0..count {
        globals.push(GlobalEntry {
            name: cursor_u32(bytes, &mut offset)?,
            type_id: cursor_u32(bytes, &mut offset)?,
            flags: cursor_u32(bytes, &mut offset)?,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in global table".to_string());
    }
    Ok(globals)
}

pub(super) fn read_export_table(bytes: &[u8]) -> Result<Vec<DecodedExport>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut exports = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 12));
    for _ in 0..count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = match cursor_u16(bytes, &mut offset)? {
            kind => decode_callable_export_kind(kind)?,
        };
        let _flags = cursor_u16(bytes, &mut offset)?;
        let function_id = cursor_u32(bytes, &mut offset)?;
        exports.push(DecodedExport {
            name,
            kind,
            function_id,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in export table".to_string());
    }
    Ok(exports)
}

pub(super) fn read_abi_index(bytes: &[u8]) -> Result<AbiIndex, String> {
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

pub(super) fn validate_abi_index(
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
            BinaryReprExportKind::Func | BinaryReprExportKind::Sub => continue,
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
            .filter(|(_, entry)| entry.kind == entry_kind && entry.name == abi_export.name)
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

pub(super) fn abi_export_for_decoded<'a>(
    abi: &'a AbiIndex,
    export: &DecodedExport,
) -> Option<&'a AbiExport> {
    abi.exports
        .iter()
        .find(|abi_export| abi_export.name == export.name && abi_export.kind == export.kind)
}

pub(super) fn decode_callable_export_kind(value: u16) -> Result<BinaryReprExportKind, String> {
    match decode_export_kind(value)? {
        BinaryReprExportKind::Func => Ok(BinaryReprExportKind::Func),
        BinaryReprExportKind::Sub => Ok(BinaryReprExportKind::Sub),
        other => Err(format!(
            "unsupported callable export kind {}",
            encode_export_kind(other)
        )),
    }
}

pub(super) fn decode_export_kind(value: u16) -> Result<BinaryReprExportKind, String> {
    match value {
        1 => Ok(BinaryReprExportKind::Func),
        2 => Ok(BinaryReprExportKind::Sub),
        3 => Ok(BinaryReprExportKind::Type),
        4 => Ok(BinaryReprExportKind::Union),
        5 => Ok(BinaryReprExportKind::Enum),
        other => Err(format!("unsupported export kind {other}")),
    }
}

pub(super) fn encode_export_kind(kind: BinaryReprExportKind) -> u16 {
    match kind {
        BinaryReprExportKind::Func => 1,
        BinaryReprExportKind::Sub => 2,
        BinaryReprExportKind::Type => 3,
        BinaryReprExportKind::Union => 4,
        BinaryReprExportKind::Enum => 5,
    }
}

pub(super) fn type_name(types: &HashMap<u32, String>, id: u32) -> Result<&str, String> {
    if let Some(name) = primitive_type_name(id) {
        return Ok(name);
    }
    types
        .get(&id)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown type id {id}"))
}

pub(super) fn string_at(strings: &[String], id: u32) -> Result<&str, String> {
    strings
        .get(id as usize)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown string id {id}"))
}

pub(super) fn function_sig_hash(
    function: &Function,
    export_kind: BinaryReprExportKind,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
) -> Result<[u8; ABI_HASH_LEN], String> {
    let mut serializer = AbiSerializer::new(strings, types, constants);
    serializer.bytes.extend_from_slice(b"MFBABI\0");
    serializer.put_u16(ABI_FORMAT_VERSION);
    serializer.put_str("function");
    serializer.put_u16(encode_export_kind(export_kind));
    serializer.put_u16(function.flags & (FUNCTION_FLAG_ISOLATED | FUNCTION_FLAG_SUB));
    serializer.put_u32(function.params.len() as u32);
    for param in &function.params {
        serializer.serialize_type(param.type_id)?;
        if param.default_const == u32::MAX {
            serializer.put_u8(0);
        } else {
            serializer.put_u8(1);
            serializer.serialize_const(param.default_const)?;
        }
    }
    serializer.serialize_type(function.return_type)?;
    Ok(hash_bytes(&serializer.bytes))
}

pub(super) fn type_sig_hash(
    type_id: u32,
    export_kind: BinaryReprExportKind,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
) -> Result<[u8; ABI_HASH_LEN], String> {
    let mut serializer = AbiSerializer::new(strings, types, constants);
    serializer.bytes.extend_from_slice(b"MFBABI\0");
    serializer.put_u16(ABI_FORMAT_VERSION);
    serializer.put_str("type");
    serializer.put_u16(encode_export_kind(export_kind));
    serializer.serialize_type(type_id)?;
    Ok(hash_bytes(&serializer.bytes))
}

impl<'a> AbiSerializer<'a> {
    pub(super) fn new(
        strings: &'a [String],
        types: &'a TypeTable,
        constants: &'a ConstPool,
    ) -> Self {
        Self {
            strings,
            types,
            constants,
            bytes: Vec::new(),
            type_refs: HashMap::new(),
            next_ref: 0,
            depth: 0,
        }
    }

    pub(super) fn serialize_type(&mut self, id: u32) -> Result<(), String> {
        // Depth cap (bug-153): reject a deep acyclic type chain before it
        // overflows the native stack. The `type_refs` cycle guard only rejects
        // repeated ids, so a separate counter is needed. Balanced decrement on
        // the success path; an over-deep graph aborts the whole serialization.
        self.depth += 1;
        if self.depth > MAX_TYPE_GRAPH_DEPTH {
            return Err(format!(
                "type graph too deep (exceeds {MAX_TYPE_GRAPH_DEPTH})"
            ));
        }
        let result = self.serialize_type_inner(id);
        self.depth -= 1;
        result
    }

    fn serialize_type_inner(&mut self, id: u32) -> Result<(), String> {
        if let Some(primitive) = primitive_type_name(id) {
            self.put_u8(1);
            self.put_u32(id);
            self.put_str(primitive);
            return Ok(());
        }

        if let Some(ref_id) = self.type_refs.get(&id).copied() {
            self.put_u8(2);
            self.put_u32(ref_id);
            return Ok(());
        }

        let entry = id
            .checked_sub(FIRST_TABLE_TYPE_ID)
            .and_then(|index| self.types.entries.get(index as usize))
            .ok_or_else(|| format!("unknown type id {id}"))?;
        let ref_id = self.next_ref;
        self.next_ref = self
            .next_ref
            .checked_add(1)
            .ok_or_else(|| "ABI type graph has too many nodes".to_string())?;
        self.type_refs.insert(id, ref_id);

        self.put_u8(3);
        self.put_u32(ref_id);
        self.put_u16(entry.kind);
        match entry.kind {
            1 => self.serialize_record_type(entry),
            2 => self.serialize_union_type(entry),
            3 => self.serialize_enum_type(entry),
            4 => {
                self.put_str("list");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            5 => {
                self.put_str("map");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)
            }
            6 => {
                self.put_str("result");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            7 => {
                self.put_str("thread");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)?;
                // The resource plane (if present) is part of the signature hash.
                if entry.payload.len() >= 12 {
                    self.serialize_type(checked_u32_at(&entry.payload, 8)?)?;
                }
                Ok(())
            }
            8 => self.serialize_function_type(entry),
            _ => {
                self.put_str("opaque");
                self.put_str(string_at(self.strings, entry.name)?);
                Ok(())
            }
        }
    }

    pub(super) fn serialize_record_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("record");
        let mut offset = 0;
        let field_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(field_count);
        for _ in 0..field_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            let type_id = cursor_u32(&entry.payload, &mut offset)?;
            let _visibility = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            self.serialize_type(type_id)?;
            self.put_u32(_visibility);
        }
        Ok(())
    }

    pub(super) fn serialize_union_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("union");
        let mut offset = 0;
        let variant_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(variant_count);
        for _ in 0..variant_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            let field_count = cursor_u32(&entry.payload, &mut offset)?;
            self.put_u32(field_count);
            for _ in 0..field_count {
                let field_name = cursor_u32(&entry.payload, &mut offset)?;
                let field_type = cursor_u32(&entry.payload, &mut offset)?;
                self.put_str(string_at(self.strings, field_name)?);
                self.serialize_type(field_type)?;
            }
        }
        Ok(())
    }

    pub(super) fn serialize_enum_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("enum");
        let mut offset = 0;
        let member_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(member_count);
        for _ in 0..member_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            let ordinal = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            self.put_u32(ordinal);
        }
        Ok(())
    }

    pub(super) fn serialize_function_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("function-type");
        let mut offset = 0;
        let isolated = cursor_u32(&entry.payload, &mut offset)?;
        let param_count = cursor_u32(&entry.payload, &mut offset)?;
        let return_type = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(isolated);
        self.put_u32(param_count);
        self.serialize_type(return_type)?;
        for _ in 0..param_count {
            self.serialize_type(cursor_u32(&entry.payload, &mut offset)?)?;
        }
        Ok(())
    }

    pub(super) fn serialize_const(&mut self, id: u32) -> Result<(), String> {
        let constant = self
            .constants
            .entries
            .get(id as usize)
            .ok_or_else(|| format!("unknown const id {id}"))?;
        self.put_u16(constant.kind);
        match constant.kind {
            6 => {
                let string_id = checked_u32_at(&constant.payload, 0)?;
                self.put_str(string_at(self.strings, string_id)?);
            }
            _ => {
                self.put_u32(constant.payload.len() as u32);
                self.bytes.extend_from_slice(&constant.payload);
            }
        }
        Ok(())
    }

    pub(super) fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(super) fn put_u16(&mut self, value: u16) {
        put_u16(&mut self.bytes, value);
    }

    pub(super) fn put_u32(&mut self, value: u32) {
        put_u32(&mut self.bytes, value);
    }

    pub(super) fn put_str(&mut self, value: &str) {
        put_bytes(&mut self.bytes, value.as_bytes());
    }
}

pub(super) fn is_exported_function(function: &Function) -> bool {
    function.kind == FUNCTION_BINARY_REPR && function.flags & FUNCTION_FLAG_PRIVATE == 0
}
