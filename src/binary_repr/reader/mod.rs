use super::*;

mod abi_index;
mod type_decode;
pub(in crate::binary_repr) use abi_index::*;
pub(in crate::binary_repr) use type_decode::*;

/// Maximum depth of the composite type-reference graph an untrusted `.mfp` may
/// encode. Real type graphs are shallow; a crafted long *linear* chain of
/// distinct composite types (List OF id → List OF id → …) would otherwise recurse
/// one native stack frame per link and overflow the stack before any signature is
/// trusted (bug-153). The cycle guards (`in_progress` / `type_refs`) only reject
/// *repeated* ids, not a deep acyclic chain, so a separate depth cap is required.
pub(super) const MAX_TYPE_GRAPH_DEPTH: usize = 256;

// === Package-meta section decoding =========================================
// Read-side of the self-describing `package-meta` (18) section; its encoder is
// the adjacent `encode_package_meta`. The `doc` (17) section's codec --
// `read_doc_table`, `doc_kind_name` and `encode_doc_table` -- moved to
// `mfb_wire::docs` (plan-126-D) so the registry can decode published docs; they
// arrive here through the glob re-export in mod.rs.

/// Encode section 18 (plan-61-D §3), or `None` when there is nothing to encode.
///
/// ```text
/// u32          fieldCount
///   per field:
///     u16      fieldId        (1 = description; 2..= reserved)
///     u32      byteLength
///     u8[]     utf8 value
/// ```
///
/// A field-id/length design rather than a positional record, so a later field
/// is additive *within* the section and an unknown id is skipped by the same
/// logic that makes the section itself skippable.
pub(super) fn encode_package_meta(description: &str) -> Option<Vec<u8>> {
    if description.is_empty() {
        return None;
    }
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1); // one field
    put_u16(&mut bytes, PACKAGE_META_FIELD_DESCRIPTION);
    put_u32(&mut bytes, description.len() as u32);
    bytes.extend_from_slice(description.as_bytes());
    Some(bytes)
}

/// Decode section 18. Returns the description, or an empty string when the
/// section carries no `description` field.
///
/// **Unknown field ids are skipped, not rejected** — that is what makes a later
/// field additive. The 4096-byte cap is re-checked here rather than trusted
/// from manifest validation, because a hand-built payload never went through
/// the manifest.
pub(super) fn read_package_meta(bytes: &[u8]) -> Result<String, String> {
    let mut offset = 0usize;
    let field_count = cursor_u32(bytes, &mut offset)? as usize;
    let mut description = String::new();
    for _ in 0..field_count {
        let field_id = cursor_u16(bytes, &mut offset)?;
        let length = cursor_u32(bytes, &mut offset)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or("package meta field length overflows")?;
        let value = bytes
            .get(offset..end)
            .ok_or("truncated package meta field")?;
        offset = end;
        if field_id == PACKAGE_META_FIELD_DESCRIPTION {
            if length > crate::manifest::MAX_DESCRIPTION_BYTES {
                return Err(format!(
                    "package description exceeds the {} byte limit ({length} bytes)",
                    crate::manifest::MAX_DESCRIPTION_BYTES,
                ));
            }
            description = String::from_utf8(value.to_vec())
                .map_err(|_| "package description is not valid UTF-8".to_string())?;
        }
        // Any other field id: skipped. Its length prefix is what makes that
        // possible without knowing what it was.
    }
    Ok(description)
}

// === Container framing + identity/signature validation =====================

/// A typed handle over the frozen wire section ids (declared in mod.rs). It
/// carries the fetch policy — `require` for a mandatory section, `optional` for
/// one whose absence is a normal, defaulting case — so the container decoder
/// reads as one line per section instead of a hand-written `get`/`ok_or_else`
/// (or `match`) block each (bug-335 B3).
#[derive(Clone, Copy)]
enum SectionKind {
    Manifest,
    StringPool,
    TypeTable,
    ConstPool,
    ImportTable,
    ExportTable,
    GlobalTable,
    FunctionTable,
    NativeLibraryTable,
    ResourceTable,
    AbiIndex,
    BinaryRepr,
    DocTable,
    PackageMeta,
}

impl SectionKind {
    fn id(self) -> u16 {
        match self {
            SectionKind::Manifest => SECTION_MANIFEST,
            SectionKind::StringPool => SECTION_STRING_POOL,
            SectionKind::TypeTable => SECTION_TYPE_TABLE,
            SectionKind::ConstPool => SECTION_CONST_POOL,
            SectionKind::ImportTable => SECTION_IMPORT_TABLE,
            SectionKind::ExportTable => SECTION_EXPORT_TABLE,
            SectionKind::GlobalTable => SECTION_GLOBAL_TABLE,
            SectionKind::FunctionTable => SECTION_FUNCTION_TABLE,
            SectionKind::NativeLibraryTable => SECTION_NATIVE_LIBRARY_TABLE,
            SectionKind::ResourceTable => SECTION_RESOURCE_TABLE,
            SectionKind::AbiIndex => SECTION_ABI_INDEX,
            SectionKind::BinaryRepr => SECTION_BINARY_REPR,
            SectionKind::DocTable => SECTION_DOC_TABLE,
            SectionKind::PackageMeta => SECTION_PACKAGE_META,
        }
    }

    /// The phrase spliced into the "MFPC is missing the … section" diagnostic a
    /// required section raises when absent.
    fn label(self) -> &'static str {
        match self {
            SectionKind::Manifest => "manifest",
            SectionKind::StringPool => "string pool",
            SectionKind::TypeTable => "type table",
            SectionKind::ConstPool => "const pool",
            SectionKind::ImportTable => "import table",
            SectionKind::ExportTable => "export table",
            SectionKind::GlobalTable => "global table",
            SectionKind::FunctionTable => "function table",
            SectionKind::NativeLibraryTable => "native library table",
            SectionKind::ResourceTable => "resource table",
            SectionKind::AbiIndex => "ABI_INDEX",
            SectionKind::BinaryRepr => "Binary Representation",
            SectionKind::DocTable => "doc",
            SectionKind::PackageMeta => "package metadata",
        }
    }

    /// Fetch a mandatory section, erroring with the section's label when absent.
    ///
    /// plan-126-C: the map is a `BTreeMap` since the section-table decode moved
    /// to `mfb_wire::mfpc` — chosen so section iteration order is deterministic.
    fn require<'a>(self, sections: &BTreeMap<u16, &'a [u8]>) -> Result<&'a [u8], String> {
        sections
            .get(&self.id())
            .copied()
            .ok_or_else(|| format!("MFPC is missing the {} section", self.label()))
    }

    /// Fetch an optional section; `None` when the package does not carry it.
    fn optional<'a>(self, sections: &BTreeMap<u16, &'a [u8]>) -> Option<&'a [u8]> {
        sections.get(&self.id()).copied()
    }
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
    // The third `.mfp` fixed-prefix decoder (see the table in
    // `mfb_wire::mfp`'s module doc). Its policy is the laxest of the three: no
    // per-field byte caps, because it only needs the identity fields to
    // cross-check against the payload's own manifest, and the payload is
    // already covered by `packageBinaryHash`. It shares the primitives and
    // keeps that policy.
    if bytes.len() < FIXED_PREFIX_LEN {
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

    let mut offset = FIXED_PREFIX_LEN;
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

// `validate_mfp_signature_header` moved to `mfb_wire::mfp` as
// `validate_signature_header` (plan-126-B). The registry had a byte-identical
// copy; they were diffed before merging, so neither was chosen over the other.

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
    // plan-126-C: the section-table decode was inline here and independently
    // reimplemented in `repository/src/abi.rs`. It is now
    // `mfb_wire::mfpc::read_section_table`, enforcing the **union** of what the
    // two copies had between them — so this side gained the declared-count
    // ceiling (bug-578) that only the registry's copy had, and the registry
    // gained the MFPC major-version check and the checked width conversion that
    // only this one had.
    //
    // The map is a `BTreeMap` now rather than a `HashMap`, so section iteration
    // order is deterministic.
    let sections = mfb_wire::mfpc::read_section_table(bytes)?;

    let strings = StringPool {
        values: read_string_pool(SectionKind::StringPool.require(&sections)?)?,
    };
    let types = read_type_entries(SectionKind::TypeTable.require(&sections)?, &strings.values)?;
    let type_names = type_entry_names(&types, &strings.values)?;
    let constants = read_const_pool(SectionKind::ConstPool.require(&sections)?)?;
    let functions = read_function_table(
        SectionKind::FunctionTable.require(&sections)?,
        // Function bodies are carried by SECTION_BINARY_REPR (structured Binary Representation), not a
        // flat code section; the function table records zero-length code regions.
        &[],
        &strings.values,
        &type_names,
    )?;
    validate_default_functions(&functions, &strings.values)?;
    let binary_repr = SectionKind::BinaryRepr.require(&sections)?.to_vec();
    let exports = read_export_table(SectionKind::ExportTable.require(&sections)?)?;
    let resources = match SectionKind::ResourceTable.optional(&sections) {
        Some(section) => read_resource_table(section)?,
        None => ResourceTable::new(),
    };
    let globals = match SectionKind::GlobalTable.optional(&sections) {
        Some(section) => read_global_table(section)?,
        None => Vec::new(),
    };
    let docs = match SectionKind::DocTable.optional(&sections) {
        Some(section) => read_doc_table(section)?,
        None => PackageDocs::default(),
    };
    // Section 18 is optional; its absence is the normal case and decodes to an
    // empty description, following the DOC absence idiom directly above.
    let description = match SectionKind::PackageMeta.optional(&sections) {
        Some(section) => read_package_meta(section)?,
        None => String::new(),
    };
    // Section 10 is present only for a binding package (plan-46-B); its absence is
    // the normal case and decodes to an empty table.
    let native_libraries = match SectionKind::NativeLibraryTable.optional(&sections) {
        Some(section) => read_native_library_table(section, &strings.values)?,
        None => NativeLibraryTable::default(),
    };
    let manifest = read_manifest(SectionKind::Manifest.require(&sections)?)?;
    let imports = read_import_table(SectionKind::ImportTable.require(&sections)?)?;
    let abi = read_abi_index(SectionKind::AbiIndex.require(&sections)?)?;
    validate_manifest_counts(&manifest, &imports, &exports)?;
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
            native_libraries,
            description,
        },
        exports,
    })
}

// === Per-section table decoders =============================================

pub(super) fn read_function_table(
    bytes: &[u8],
    code: &[u8],
    strings: &[String],
    _types: &HashMap<u32, String>,
) -> Result<Vec<Function>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    // A function entry occupies >= 52 wire bytes, so use that as the min-element
    // size for the PKG-05 pre-allocation bound (was an understated 4, ~13x loose).
    let mut functions = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 52));
    for _ in 0..count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = cursor_u16(bytes, &mut offset)?;
        let flags = cursor_u16(bytes, &mut offset)?;
        if flags & RETIRED_FUNCTION_FLAG_SUB != 0 {
            return Err(retired_sub_error());
        }
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

/// Validate every parameter record whose default is a hidden default function
/// (`PARAM_FLAG_DEFAULT_FUNCTION`, plan-136-B).
///
/// A `.mfp` is untrusted input, and an importer calls the function such a record
/// names on every call that omits the argument. So the reader refuses, and never
/// repairs, a record that is not exactly what the writer produces: the flag
/// without `PARAM_FLAG_DEFAULT`, an index outside the function table, or a target
/// that is not a private, parameterless function returning the parameter's type
/// and named as a hidden default function.
pub(super) fn validate_default_functions(
    functions: &[Function],
    strings: &[String],
) -> Result<(), String> {
    for function in functions {
        for param in &function.params {
            if param.flags & PARAM_FLAG_DEFAULT_FUNCTION == 0 {
                continue;
            }
            let refuse = |why: String| -> Result<(), String> {
                Err(format!(
                    "function `{}` parameter `{}` has an invalid default function: {why}",
                    string_at(strings, function.name).unwrap_or("<invalid>"),
                    string_at(strings, param.name).unwrap_or("<invalid>"),
                ))
            };
            if param.flags & PARAM_FLAG_DEFAULT == 0 {
                return refuse("the default-function flag is set without the default flag".into());
            }
            let Some(target) = functions.get(param.default_const as usize) else {
                return refuse(format!(
                    "function index {} is outside the function table",
                    param.default_const
                ));
            };
            if target.flags & FUNCTION_FLAG_PRIVATE == 0 {
                return refuse("the target function is not private".into());
            }
            if !target.params.is_empty() {
                return refuse("the target function takes parameters".into());
            }
            if target.return_type != param.type_id {
                return refuse("the target function does not return the parameter's type".into());
            }
            if !crate::internal_name::is_hidden_default_function(string_at(strings, target.name)?) {
                return refuse("the target function is not a hidden default function".into());
            }
        }
    }
    Ok(())
}

pub(super) fn read_manifest(bytes: &[u8]) -> Result<BinaryReprManifest, String> {
    let mut offset = 0;
    let package_name = cursor_u32(bytes, &mut offset)?;
    let package_ident = cursor_u32(bytes, &mut offset)?;
    let package_version = cursor_u32(bytes, &mut offset)?;
    let ident_key = cursor_u32(bytes, &mut offset)?;
    let ident_fingerprint = cursor_u32(bytes, &mut offset)?;
    let signing_fingerprint = cursor_u32(bytes, &mut offset)?;
    let author = cursor_u32(bytes, &mut offset)?;
    let url = cursor_u32(bytes, &mut offset)?;
    let _binary_repr_major = cursor_u16(bytes, &mut offset)?;
    let _binary_repr_minor = cursor_u16(bytes, &mut offset)?;
    let _language_major = cursor_u16(bytes, &mut offset)?;
    let _language_minor = cursor_u16(bytes, &mut offset)?;
    let _minimum_runtime_major = cursor_u16(bytes, &mut offset)?;
    let _minimum_runtime_minor = cursor_u16(bytes, &mut offset)?;
    let dependency_count = cursor_u32(bytes, &mut offset)?;
    // Reserved: the writer always emits 0 here, including for a binding package
    // whose native libraries live in section 10, so there is nothing to validate
    // it against. Kept in the layout for wire stability.
    let _native_link_count = cursor_u32(bytes, &mut offset)?;
    let export_count = cursor_u32(bytes, &mut offset)?;
    // Reserved: a package has no entry point, so the writer emits the IR entry
    // only for a program image. Decoded for layout, ignored by every consumer.
    let _entry_function = cursor_u32(bytes, &mut offset)?;
    let _entry_flags = cursor_u32(bytes, &mut offset)?;
    if offset != bytes.len() {
        return Err("invalid trailing bytes in manifest".to_string());
    }
    Ok(BinaryReprManifest {
        package_name,
        package_ident,
        package_version,
        ident_key,
        ident_fingerprint,
        signing_fingerprint,
        author,
        url,
        dependency_count,
        export_count,
    })
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
        let kind = decode_callable_export_kind(cursor_u16(bytes, &mut offset)?)?;
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
