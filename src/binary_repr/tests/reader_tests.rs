// ---------------------------------------------------------------------------
// reader.rs — decode paths, error handling, and container framing.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;

#[test]
fn read_binary_repr_package_round_trips_rich_project() {
    let project = rich_project();
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let bytes = encode_project(&project, &metadata);
    let package = read_binary_repr_package(&bytes).expect("decode package");
    assert_eq!(package.project.functions.len(), 3);
    assert_eq!(package.exports.len(), 2);
    assert_eq!(package.project.globals.len(), 3);
}

/// plan-61-D Phase 2: a real package round-trips its description through
/// section 18, and a package without one emits no section 18 at all.
#[test]
fn a_description_round_trips_through_section_eighteen() {
    let project = rich_project();

    // No description: no section, and it reads back empty.
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let without = encode_project(&project, &metadata);
    let package = read_binary_repr_package(&without).expect("decode package");
    assert_eq!(package.project.description, "");

    // With one: the section is present and the value survives.
    let mut metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    metadata.description = "A rich demo package.".to_string();
    let with = encode_project(&project, &metadata);
    let package = read_binary_repr_package(&with).expect("decode package");
    assert_eq!(package.project.description, "A rich demo package.");

    // The no-description build must be byte-identical to what a
    // pre-plan-61-D compiler produced — that is the whole reason the
    // section is omitted rather than emitted empty. A shorter payload is
    // the observable proof the section is genuinely absent.
    assert!(
        without.len() < with.len(),
        "the description-free build must not carry an empty section 18",
    );
}

/// **The forward-compatibility regression test** — the premise the entire
/// section-18 design rests on (plan-61-D §2).
///
/// A payload carrying a section with an id no reader knows must parse
/// **successfully** and ignore it. This is the claim that makes adding
/// section 18 safe for readers built before it existed, tested in the only
/// direction a current-day test can: a *future* section against *this*
/// reader.
#[test]
fn a_section_with_an_unknown_id_is_parsed_and_ignored() {
    let project = rich_project();
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let bytes = encode_project(&project, &metadata);
    let baseline = read_binary_repr_package(&bytes).expect("baseline decodes");

    // Rebuild the same payload with two extra sections carrying ids no
    // reader knows — one just past the allocated range, one far outside it.
    let mut sections = decompose_sections(&bytes);
    sections.push(Section::new(99, vec![0xde, 0xad, 0xbe, 0xef]));
    sections.push(Section::new(4242, vec![0x01, 0x02]));
    let extended = encode_sections(&sections);

    let package =
        read_binary_repr_package(&extended).expect("an unknown section id must parse, not error");

    // And it is genuinely ignored: everything the reader does understand is
    // unchanged.
    assert_eq!(
        package.project.functions.len(),
        baseline.project.functions.len()
    );
    assert_eq!(package.exports.len(), baseline.exports.len());
    assert_eq!(
        package.project.globals.len(),
        baseline.project.globals.len()
    );
    assert_eq!(package.project.description, baseline.project.description);
}

/// Split an encoded MFPC payload back into its sections, so a test can add
/// one and re-encode.
fn decompose_sections(bytes: &[u8]) -> Vec<Section> {
    let count = checked_u32_at(bytes, 12).unwrap() as usize;
    (0..count)
        .map(|index| {
            let entry = 16 + index * 24;
            let id = checked_u16_at(bytes, entry).unwrap();
            let offset = checked_u64_at(bytes, entry + 8).unwrap() as usize;
            let length = checked_u64_at(bytes, entry + 16).unwrap() as usize;
            Section::new(id, bytes[offset..offset + length].to_vec())
        })
        .collect()
}

#[test]
fn read_binary_repr_package_rejects_bad_magic_and_version() {
    let project = rich_project();
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let mut bytes = encode_project(&project, &metadata);

    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert!(read_binary_repr_package(&bad_magic).is_err());

    // Wrong MFPC major version at offset 4.
    bytes[4] = 0xFF;
    bytes[5] = 0xFF;
    assert!(read_binary_repr_package(&bytes).is_err());
}

#[test]
fn read_binary_repr_package_rejects_short_input() {
    assert!(read_binary_repr_package(&[]).is_err());
    assert!(read_binary_repr_package(b"MFPC").is_err());
}

#[test]
fn read_binary_repr_package_rejects_truncated_section_table() {
    let project = rich_project();
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let bytes = encode_project(&project, &metadata);
    // Truncate mid-section-table.
    let truncated = &bytes[..40];
    assert!(read_binary_repr_package(truncated).is_err());
}

#[test]
fn primitive_type_name_and_type_name_resolution() {
    assert_eq!(primitive_type_name(TYPE_INTEGER), Some("Integer"));
    assert_eq!(primitive_type_name(TYPE_MONEY), Some("Money"));
    assert_eq!(primitive_type_name(TYPE_FILE_HANDLE), Some("fs.File"));
    assert_eq!(primitive_type_name(999_999), None);
    let empty = std::collections::HashMap::new();
    assert_eq!(type_name(&empty, TYPE_STRING).unwrap(), "String");
    assert!(type_name(&empty, FIRST_TABLE_TYPE_ID).is_err());
}

#[test]
fn string_at_bounds_check() {
    let strings = vec!["a".to_string(), "b".to_string()];
    assert_eq!(string_at(&strings, 1).unwrap(), "b");
    assert!(string_at(&strings, 5).is_err());
}

#[test]
fn export_kind_encode_decode_round_trips() {
    for kind in [
        BinaryReprExportKind::Func,
        BinaryReprExportKind::Sub,
        BinaryReprExportKind::Type,
        BinaryReprExportKind::Union,
        BinaryReprExportKind::Enum,
    ] {
        let encoded = encode_export_kind(kind);
        assert!(decode_export_kind(encoded).unwrap() == kind);
    }
    assert!(decode_export_kind(99).is_err());
    // Callable-only decoder rejects Type/Union/Enum.
    assert!(decode_callable_export_kind(3).is_err());
    assert!(decode_callable_export_kind(1).unwrap() == BinaryReprExportKind::Func);
}

#[test]
fn doc_kind_name_maps_codes() {
    assert_eq!(doc_kind_name(DOC_KIND_FUNC), "func");
    assert_eq!(doc_kind_name(DOC_KIND_SUB), "sub");
    assert_eq!(doc_kind_name(DOC_KIND_TYPE), "type");
    assert_eq!(doc_kind_name(DOC_KIND_UNION), "union");
    assert_eq!(doc_kind_name(DOC_KIND_ENUM), "enum");
    assert_eq!(doc_kind_name(999), "func");
}

#[test]
fn read_doc_table_handles_absent_package() {
    let docs = PackageDocs {
        package: None,
        decls: vec![],
    };
    let bytes = encode_doc_table(&docs);
    let decoded = read_doc_table(&bytes).expect("decode empty doc table");
    assert!(decoded.package.is_none());
    assert!(decoded.decls.is_empty());
}

#[test]
fn read_doc_table_rejects_truncation() {
    assert!(read_doc_table(&[]).is_err());
    // Package flag set to 1 but nothing follows.
    assert!(read_doc_table(&[1]).is_err());
}

#[test]
fn read_doc_table_rejects_trailing_bytes() {
    // bug-282 B3: the doc table was added after audit-1 PKG-05 and skipped the
    // trailing-bytes rejection every other section performs, so garbage past
    // the last declaration decoded silently.
    let docs = PackageDocs {
        package: None,
        decls: vec![],
    };
    let mut bytes = encode_doc_table(&docs);
    bytes.push(0xAA);
    let err = match read_doc_table(&bytes) {
        Ok(_) => panic!("trailing garbage must be rejected"),
        Err(err) => err,
    };
    assert!(err.contains("trailing bytes"), "unexpected error: {err}");
}

#[test]
fn read_string_pool_rejects_trailing_and_truncation() {
    // Count 0 but trailing bytes present.
    let mut trailing = Vec::new();
    put_u32(&mut trailing, 0);
    trailing.push(0xAA);
    assert!(read_string_pool(&trailing).is_err());
    // Count 1 but entry claims more bytes than exist.
    let mut truncated = Vec::new();
    put_u32(&mut truncated, 1);
    put_u32(&mut truncated, 100);
    assert!(read_string_pool(&truncated).is_err());
}

#[test]
fn read_type_entries_rejects_duplicate_name_and_kind() {
    // bug-282 B2: two entries sharing `(name, kind)` let validation and
    // decoding disagree about which definition an export names.
    // `validate_abi_index` passes when *any* same-name candidate reproduces
    // the hash, while `package_type_exports` collects into a last-wins map --
    // so a crafted package can pass validation against entry A while importers
    // compile against entry B. The writer interns a name once and never emits
    // a duplicate, so rejecting one costs nothing legitimate.
    let mut strings = StringPool::new();
    let name = strings.intern("Dup");
    // Two 20-byte headers naming the same (kind 1, "Dup"), both with an empty
    // payload parked past the header block.
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 2);
    let payload_offset = 4 + 2 * 20;
    for _ in 0..2 {
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, name);
        put_u32(&mut bytes, 0);
        put_u32(&mut bytes, payload_offset as u32);
        put_u32(&mut bytes, 0);
    }
    let err = match read_type_entries(&bytes, &strings.values) {
        Ok(_) => panic!("a duplicate type definition must be rejected"),
        Err(err) => err,
    };
    assert!(err.contains("duplicate type"), "unexpected error: {err}");
}

#[test]
fn read_type_entries_rejects_bad_bounds() {
    // Claims one entry but the table is truncated.
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1);
    assert!(read_type_entries(&bytes, &[]).is_err());
}

#[test]
fn read_function_table_rejects_flat_code_and_trailing() {
    // Trailing garbage after a zero-function table.
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 0);
    bytes.push(0xAA);
    let empty = std::collections::HashMap::new();
    assert!(read_function_table(&bytes, &[], &[], &empty).is_err());
}

#[test]
fn read_manifest_rejects_trailing_bytes() {
    let mut bytes = Vec::new();
    for _ in 0..8 {
        put_u32(&mut bytes, 0);
    }
    for _ in 0..6 {
        put_u16(&mut bytes, 0);
    }
    for _ in 0..5 {
        put_u32(&mut bytes, 0);
    }
    // Valid so far; append a trailing byte.
    bytes.push(0xFF);
    assert!(read_manifest(&bytes).is_err());
}

#[test]
fn read_import_table_rejects_bad_pin() {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1); // one entry
    put_u32(&mut bytes, 0); // package_name
    put_u32(&mut bytes, 0); // package_ident
    put_u32(&mut bytes, 0); // version
    bytes.push(2); // invalid pin
    put_u32(&mut bytes, 0); // flags
    put_u32(&mut bytes, 0); // used symbol count
    assert!(read_import_table(&bytes).is_err());
}

#[test]
fn read_resource_and_global_tables_reject_trailing() {
    let mut res = Vec::new();
    put_u32(&mut res, 0);
    res.push(0xAB);
    assert!(read_resource_table(&res).is_err());
    let mut glob = Vec::new();
    put_u32(&mut glob, 0);
    glob.push(0xAB);
    assert!(read_global_table(&glob).is_err());
}

#[test]
fn read_export_table_rejects_bad_kind_and_trailing() {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1);
    put_u32(&mut bytes, 0); // name
    put_u16(&mut bytes, 3); // Type is not a callable export
    put_u16(&mut bytes, 0); // flags
    put_u32(&mut bytes, 0); // function id
    assert!(read_export_table(&bytes).is_err());
}

#[test]
fn read_abi_index_rejects_bad_version_and_pin() {
    let mut bad_version = Vec::new();
    put_u16(&mut bad_version, 999);
    assert!(read_abi_index(&bad_version).is_err());

    let mut bad_pin = Vec::new();
    put_u16(&mut bad_pin, ABI_FORMAT_VERSION);
    put_u16(&mut bad_pin, 0); // reserved
    put_u32(&mut bad_pin, 0); // export count
    put_u32(&mut bad_pin, 1); // edge count
    put_u32(&mut bad_pin, 0); // package_name
    put_u32(&mut bad_pin, 0); // package_ident
    put_u32(&mut bad_pin, 0); // version_request
    bad_pin.push(2); // invalid pin
    assert!(read_abi_index(&bad_pin).is_err());
}

#[test]
fn decode_type_field_maps_all_visibilities() {
    let strings = vec!["field".to_string()];
    let type_names = std::collections::HashMap::new();
    for (code, expected) in [
        (0u32, BinaryReprTypeVisibility::Export),
        (1, BinaryReprTypeVisibility::Private),
        (2, BinaryReprTypeVisibility::Public),
        (3, BinaryReprTypeVisibility::Export),
    ] {
        let mut payload = Vec::new();
        put_u32(&mut payload, 0); // name id
        put_u32(&mut payload, TYPE_INTEGER); // type id
        put_u32(&mut payload, code);
        let mut offset = 0;
        let field = decode_type_field(&payload, &mut offset, &type_names, &strings).unwrap();
        assert!(field.visibility == expected);
    }
    // Unknown visibility code is rejected.
    let mut payload = Vec::new();
    put_u32(&mut payload, 0);
    put_u32(&mut payload, TYPE_INTEGER);
    put_u32(&mut payload, 99);
    let mut offset = 0;
    assert!(decode_type_field(&payload, &mut offset, &type_names, &strings).is_err());
}

#[test]
fn package_identity_id_is_deterministic_and_content_addressed() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let wrapped = wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0");
    let container = mfp_binary_repr_payload(&wrapped).expect("payload");
    let id1 = package_identity_id(&container.identity, container.binary_repr);
    let id2 = package_identity_id(&container.identity, container.binary_repr);
    assert_eq!(id1, id2);
    assert_eq!(id1.len(), 16);
}

#[test]
fn mfp_payload_rejects_bad_magic_version_and_size() {
    assert!(mfp_binary_repr_payload(&[0u8; 4]).is_err());
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let mut good = wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0");
    assert!(mfp_binary_repr_payload(&good).is_ok());

    // Corrupt the container magic.
    let mut bad_magic = good.clone();
    bad_magic[0] = 0;
    assert!(mfp_binary_repr_payload(&bad_magic).is_err());

    // Corrupt the container version (offset 8..12 -> 2.0).
    good[8] = 2;
    assert!(mfp_binary_repr_payload(&good).is_err());
}

#[test]
fn validate_mfp_signature_header_accepts_valid_variants() {
    assert!(validate_mfp_signature_header(0, 0).is_ok());
    assert!(validate_mfp_signature_header(1, 64).is_ok());
    assert!(validate_mfp_signature_header(0, 1).is_err());
    assert!(validate_mfp_signature_header(1, 10).is_err());
    assert!(validate_mfp_signature_header(9, 0).is_err());
}

#[test]
fn read_package_binary_repr_round_trips_through_temp_file() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let path = temp_mfp(&wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0"));
    let package = read_package_binary_repr(&path).expect("read package");
    assert_eq!(package.exports.len(), 2);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn package_info_from_mfp_matches_the_on_disk_reader() {
    // The resolver reads a downloaded blob in memory rather than staging it
    // at a predictable path in the shared temp directory.
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let bytes = wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0");
    let path = temp_mfp(&bytes);
    let from_disk = read_package_info(&path).expect("read from disk");
    let _ = std::fs::remove_file(&path);
    let from_memory = package_info_from_mfp(&bytes).expect("read from memory");
    assert_eq!(from_memory.manifest_name, from_disk.manifest_name);
    assert_eq!(from_memory.imports.len(), from_disk.imports.len());
    assert!(package_info_from_mfp(b"not a package").is_err());
}

#[test]
fn validate_container_identity_rejects_mismatch() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    // Header claims a different name than the manifest.
    let path = temp_mfp(&wrap_mfp(&inner, "WRONG", "WRONG", "1.0.0"));
    assert!(read_package_binary_repr(&path).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn abi_serializer_serializes_composite_types() {
    // Build a project with record, union, enum, list, map, result, function,
    // and thread types so serialize_type walks each arm.
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    types.type_id(&mut strings, "List OF Integer");
    types.type_id(&mut strings, "Map OF String TO Integer");
    types.type_id(&mut strings, "Result OF Integer");
    let func = types.type_id(&mut strings, "FUNC(Integer) AS Boolean");
    let list = types.type_id(&mut strings, "List OF Integer");
    let constants = ConstPool::new();
    // A primitive serializes to a self-describing block.
    let hash_prim = type_sig_hash(
        TYPE_INTEGER,
        BinaryReprExportKind::Type,
        &strings.values,
        &types,
        &constants,
    )
    .unwrap();
    let hash_list = type_sig_hash(
        list,
        BinaryReprExportKind::Type,
        &strings.values,
        &types,
        &constants,
    )
    .unwrap();
    let hash_func = type_sig_hash(
        func,
        BinaryReprExportKind::Type,
        &strings.values,
        &types,
        &constants,
    )
    .unwrap();
    assert_ne!(hash_prim, hash_list);
    assert_ne!(hash_list, hash_func);
}

#[test]
fn abi_serializer_hashes_state_composites_structurally() {
    // bug-277: kind 11 (`<base> STATE <state>`) fell through to the opaque arm,
    // which hashes the interned name `State#<baseId>#<stateId>`. Those ids are
    // table positions, so the hash tracked position instead of shape. Both
    // halves are asserted here: stable under an unrelated renumber, and
    // sensitive to a change in the STATE payload's own type.
    let state_hash = |lead: Option<&str>, state: &str| {
        let mut strings = StringPool::new();
        let mut types = TypeTable::new();
        // An unrelated type interned first shifts every later table id.
        if let Some(lead) = lead {
            types.type_id(&mut strings, lead);
        }
        let id = types.type_id(&mut strings, &format!("fs.File STATE {state}"));
        let constants = ConstPool::new();
        type_sig_hash(
            id,
            BinaryReprExportKind::Type,
            &strings.values,
            &types,
            &constants,
        )
        .unwrap()
    };

    // (a) An unrelated type declared ahead of the STATE payload renumbers the
    // table but changes nothing semantic — the hash must not move.
    assert_eq!(
        state_hash(None, "List OF Integer"),
        state_hash(Some("Map OF String TO Integer"), "List OF Integer"),
        "STATE sig hash must not track type-table position"
    );

    // (b) Changing the STATE payload's shape is a real ABI change.
    assert_ne!(
        state_hash(None, "List OF Integer"),
        state_hash(None, "List OF String"),
        "STATE sig hash must track the state type's structure"
    );

    // A STATE composite must not collide with its own bare base type.
    let mut strings = StringPool::new();
    let types = TypeTable::new();
    let constants = ConstPool::new();
    let bare = type_sig_hash(
        TYPE_FILE_HANDLE,
        BinaryReprExportKind::Type,
        &strings.values,
        &types,
        &constants,
    )
    .unwrap();
    let _ = &mut strings;
    assert_ne!(bare, state_hash(None, "List OF Integer"));
}

#[test]
fn abi_serializer_rejects_deep_acyclic_type_chain() {
    // bug-153: serialize_type must reject a deep-but-acyclic type graph via
    // the depth cap. `type_refs` only guards *cycles* (repeated ids), so a
    // long linear chain of distinct composites would otherwise recurse one
    // native frame per link and overflow the stack before any hash is formed.
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    let deep_name = format!("{}Integer", "List OF ".repeat(MAX_TYPE_GRAPH_DEPTH + 5));
    let head = types.type_id(&mut strings, &deep_name);
    let constants = ConstPool::new();
    let err = type_sig_hash(
        head,
        BinaryReprExportKind::Type,
        &strings.values,
        &types,
        &constants,
    )
    .expect_err("deep chain must be rejected, not overflow the stack");
    assert!(err.contains("too deep"), "unexpected error: {err}");
}
