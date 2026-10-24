// ---------------------------------------------------------------------------
// reader.rs — remaining decode error branches and composite ABI serialization.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;

#[test]
fn mfp_payload_rejects_truncated_hash_and_signature_and_length() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let good = wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0");
    // Chop off the trailing payload so the declared binary_repr length no
    // longer matches the file length.
    let short = &good[..good.len() - 4];
    assert!(mfp_binary_repr_payload(short).is_err());
    // Chop deep into the fixed prefix (past magic+version) to trip an early
    // truncation guard.
    assert!(mfp_binary_repr_payload(&good[..30]).is_err());
}

/// Build a valid MFPC that omits exactly one required section.
fn mfpc_missing(section_id: u16) -> Vec<u8> {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    let proj = &package.project;
    let all: Vec<(u16, Vec<u8>)> = vec![
        (SECTION_MANIFEST, proj.encode_manifest()),
        (SECTION_STRING_POOL, proj.strings.encode()),
        (SECTION_TYPE_TABLE, proj.types.encode()),
        (SECTION_CONST_POOL, proj.constants.encode()),
        (SECTION_IMPORT_TABLE, proj.imports.encode()),
        (SECTION_EXPORT_TABLE, proj.encode_exports()),
        (SECTION_GLOBAL_TABLE, proj.encode_globals()),
        (
            SECTION_FUNCTION_TABLE,
            proj.encode_functions(&vec![(0u64, 0u64); proj.functions.len()]),
        ),
        (SECTION_BINARY_REPR, proj.binary_repr.clone()),
        (SECTION_ABI_INDEX, proj.abi.encode()),
    ];
    let sections: Vec<Section> = all
        .into_iter()
        .filter(|(id, _)| *id != section_id)
        .map(|(id, data)| Section::new(id, data))
        .collect();
    encode_sections(&sections)
}

#[test]
fn read_binary_repr_package_names_each_missing_section() {
    for id in [
        SECTION_STRING_POOL,
        SECTION_TYPE_TABLE,
        SECTION_CONST_POOL,
        SECTION_FUNCTION_TABLE,
        SECTION_BINARY_REPR,
        SECTION_EXPORT_TABLE,
        SECTION_MANIFEST,
        SECTION_IMPORT_TABLE,
        SECTION_ABI_INDEX,
    ] {
        let bytes = mfpc_missing(id);
        assert!(
            read_binary_repr_package(&bytes).is_err(),
            "missing section {id} should be rejected"
        );
    }
}

/// The global table is optional: a package without one (a producer that wrote
/// none) decodes, with no globals, rather than being refused.
#[test]
fn a_package_without_a_global_table_decodes_with_no_globals() {
    let package = read_binary_repr_package(&mfpc_missing(SECTION_GLOBAL_TABLE))
        .expect("an absent global table is not an error");
    assert!(package.project.globals.is_empty());
    assert!(
        !package.project.functions.is_empty(),
        "the rest still decodes"
    );
}

/// The function table's code region is retired: an entry that names any
/// non-empty one — by offset alone or by length — is refused by name.
#[test]
fn a_function_entry_naming_a_flat_code_region_is_refused() {
    for (code_offset, code_length) in [(8u64, 0u64), (0, 4)] {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, 1); // one function
        put_u32(&mut bytes, 0); // name
        put_u16(&mut bytes, 0); // kind
        put_u16(&mut bytes, 0); // flags
        put_u32(&mut bytes, 0); // param count
        put_u32(&mut bytes, TYPE_INTEGER); // return type
        put_u32(&mut bytes, 0); // register count
        put_u64(&mut bytes, code_offset);
        put_u64(&mut bytes, code_length);
        put_u32(&mut bytes, u32::MAX); // source map
        put_u32(&mut bytes, 0); // cleanup count
        put_u64(&mut bytes, 0); // cleanup offset
        let strings = vec![String::new()];
        assert_eq!(
            read_function_table(&bytes, &strings, &std::collections::HashMap::new()).err(),
            Some("flat function code stream is no longer supported".to_string()),
            "offset {code_offset}, length {code_length}"
        );
    }
}

#[test]
fn read_binary_repr_package_without_optional_sections_still_decodes() {
    // The manifest built above encodes fine with no resource/global/doc data;
    // rebuild with all required sections present and confirm it decodes.
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    assert!(read_binary_repr_package(&inner).is_ok());
}

#[test]
fn primitive_type_name_covers_handle_and_term_types() {
    assert_eq!(primitive_type_name(TYPE_BYTE), Some("Byte"));
    assert_eq!(primitive_type_name(TYPE_ERROR), Some("Error"));
    // `TermColor` the TYPE is retired (plan-122-F), but this row deliberately
    // survives it: the reader must still name `TYPE_TERM_COLOR` so a `.mfp`
    // published before the retirement decodes to something recognizable instead of
    // failing opaquely. The encoder side — that nothing produces the id any more —
    // is `no_encoder_emits_the_retired_term_color_id`.
    assert_eq!(primitive_type_name(TYPE_TERM_COLOR), Some("TermColor"));
    assert_eq!(primitive_type_name(TYPE_TERM_SIZE), Some("TermSize"));
    assert_eq!(primitive_type_name(TYPE_SOCKET_HANDLE), Some("tcp.Socket"));
    assert_eq!(
        primitive_type_name(TYPE_LISTENER_HANDLE),
        Some("tcp.Listener")
    );
}

#[test]
fn type_entry_names_rejects_cyclic_type() {
    // A composite (list, kind 4) whose payload references its own id.
    let mut types = TypeTable::new();
    types.entries.push(TypeEntry {
        kind: 4,
        name: 0,
        owner_package: 0,
        abi_export_kind: None,
        payload: FIRST_TABLE_TYPE_ID.to_le_bytes().to_vec(),
    });
    types
        .ids
        .insert("List#self".to_string(), FIRST_TABLE_TYPE_ID);
    let strings = vec!["List#self".to_string()];
    assert!(type_entry_names(&types, &strings).is_err());
}

#[test]
fn decode_function_type_round_trips_via_type_entry_names() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    types.type_id(
        &mut strings,
        &crate::types::ParameterType::declared("ISOLATED FUNC(Integer, String) AS Boolean"),
    );
    let names = type_entry_names(&types, &strings.values).expect("names");
    assert!(names
        .values()
        .any(|n| n == "ISOLATED FUNC(Integer, String) AS Boolean"));
}

#[test]
fn validate_abi_index_recomputes_type_export_hashes() {
    // A record type export whose ABI sigHash was tampered with must be
    // rejected at decode, exactly as a tampered callable export hash is.
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    let mut payload = Vec::new();
    put_u32(&mut payload, 0); // zero fields
    let type_id = types.add_entry(&mut strings, "pkg", "Point", 1, payload);
    let constants = ConstPool::new();
    let name = strings.intern("Point");
    let sig_hash = type_sig_hash(
        type_id,
        BinaryReprExportKind::Type,
        &strings.values,
        &types,
        &constants,
    )
    .unwrap();
    let imports = ImportTable { entries: vec![] };

    let good = AbiIndex {
        exports: vec![AbiExport {
            name,
            kind: BinaryReprExportKind::Type,
            sig_hash,
        }],
        dep_edges: vec![],
    };
    validate_abi_index(
        &good,
        &[],
        &imports,
        &strings.values,
        &types,
        &constants,
        &[],
    )
    .expect("a faithful type export hash validates");

    let mut tampered_hash = sig_hash;
    tampered_hash[0] ^= 0xff;
    let tampered = AbiIndex {
        exports: vec![AbiExport {
            name,
            kind: BinaryReprExportKind::Type,
            sig_hash: tampered_hash,
        }],
        dep_edges: vec![],
    };
    let err = validate_abi_index(
        &tampered,
        &[],
        &imports,
        &strings.values,
        &types,
        &constants,
        &[],
    )
    .expect_err("a forged type export hash must be rejected");
    assert!(
        err.contains("type export `Point` sigHash disagrees"),
        "{err}"
    );

    // An export naming a type that is absent from the TYPE_TABLE is an error.
    let orphan_name = strings.intern("Ghost");
    let orphan = AbiIndex {
        exports: vec![AbiExport {
            name: orphan_name,
            kind: BinaryReprExportKind::Union,
            sig_hash,
        }],
        dep_edges: vec![],
    };
    let err = validate_abi_index(
        &orphan,
        &[],
        &imports,
        &strings.values,
        &types,
        &constants,
        &[],
    )
    .expect_err("an unbacked type export must be rejected");
    assert!(err.contains("is missing from the type table"), "{err}");
}

/// bug-37: a decoded 64-bit length or offset is rejected, not truncated, when
/// it does not fit the host's address space. Every value fits on a 64-bit
/// host, so the guard is exercised directly rather than through a crafted
/// package.
#[test]
fn checked_usize_rejects_a_length_beyond_the_address_space() {
    assert_eq!(checked_usize(0, "length"), Ok(0));
    assert_eq!(checked_usize(usize::MAX as u64, "length"), Ok(usize::MAX));
    // Only a target narrower than 64 bits can overflow; assert the guard
    // rejects there and admits everything here.
    assert_eq!(checked_usize(u64::MAX, "length").is_ok(), usize::BITS >= 64);
    if usize::BITS < 64 {
        let err = checked_usize(u64::from(u32::MAX) + 1, "MFPC section length")
            .expect_err("oversized length must be rejected");
        assert!(err.contains("exceeds the address space"), "{err}");
    }
}

#[test]
fn abi_serializer_rejects_reserved_type_ids_without_overflow() {
    // Id 0 is neither a primitive nor a table id (>= 10) — a tampered package
    // can carry it; the serializer must report it cleanly rather than
    // underflowing `id - FIRST_TABLE_TYPE_ID`. (Id 9 is now `TYPE_MONEY`, a
    // valid primitive, so it serializes; only id 0 stays reserved-low.)
    let strings: Vec<String> = Vec::new();
    let mut types = TypeTable::new();
    types.entries.push(TypeEntry {
        kind: 3,
        name: 0,
        owner_package: 0,
        abi_export_kind: None,
        payload: Vec::new(),
    });
    let constants = ConstPool::new();
    {
        let id = 0u32;
        let mut serializer = AbiSerializer::new(&strings, &types, &constants);
        let err = serializer
            .serialize_type(id)
            .expect_err("reserved type id must not serialize");
        assert_eq!(err, format!("unknown type id {id}"));
    }
    // The reused low slot id 9 now serializes as the Money primitive.
    let mut serializer = AbiSerializer::new(&strings, &types, &constants);
    serializer
        .serialize_type(TYPE_MONEY)
        .expect("Money primitive serializes");
}

#[test]
fn abi_serializer_walks_composite_field_types() {
    // An exported record whose fields are composite types forces the ABI
    // serializer through the list/map/result/function/thread arms, plus the
    // type_refs cache hit for a repeated reference.
    use crate::ir::{IrField, IrType};
    let mut project = empty_project("abicomp");
    let field = |name: &str, ty: &str| IrField {
        visibility: Some("export".to_string()),
        name: name.to_string(),
        type_: crate::types::ParameterType::parse(ty),
        loc: loc(),
    };
    project.types = vec![IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: "Bag".to_string(),
        fields: vec![
            field("l", "List OF Integer"),
            field("m", "Map OF String TO Integer"),
            field("r", "#Result OF Integer"),
            field("f", "FUNC(Integer) AS Boolean"),
            field("t", "Thread OF Integer TO String"),
            // A second List OF Integer forces the type_refs cache-hit path.
            field("l2", "List OF Integer"),
        ],
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: loc(),
        file: String::new(),
    }];
    let metadata = BinaryReprMetadata::new("abicomp".to_string(), "1".to_string());
    // Lowering computes the type sig hash, exercising serialize_type's arms.
    let lowered = lower_project(&project, &metadata).expect("lower");
    assert!(!lowered.abi.exports.is_empty());
    // Round-trips through encode/decode too.
    assert!(read_binary_repr_package(&lowered.encode()).is_ok());
}

#[test]
fn validate_abi_index_rejects_export_missing_function() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    // An EXPORT_TABLE entry that points past the function table.
    let bogus = DecodedExport {
        name: package.exports[0].name,
        kind: package.exports[0].kind,
        function_id: 9999,
    };
    assert!(validate_abi_index(
        &package.project.abi,
        std::slice::from_ref(&bogus),
        &package.project.imports,
        &package.project.strings.values,
        &package.project.types,
        &package.project.constants,
        &package.project.functions,
    )
    .is_err());
}

#[test]
fn read_function_table_rejects_nonempty_code_region() {
    // A function claiming a non-zero code length is rejected (flat code is
    // no longer supported).
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1); // one function
    put_u32(&mut bytes, 0); // name
    put_u16(&mut bytes, FUNCTION_BINARY_REPR); // kind
    put_u16(&mut bytes, 0); // flags
    put_u32(&mut bytes, 0); // param count
    put_u32(&mut bytes, TYPE_NOTHING); // return type
    put_u32(&mut bytes, 0); // register count
    put_u64(&mut bytes, 0); // code offset
    put_u64(&mut bytes, 4); // code length (non-zero!)
    put_u32(&mut bytes, u32::MAX); // source map
    put_u32(&mut bytes, 0); // cleanup count
    put_u64(&mut bytes, 0); // cleanup offset
    let strings: Vec<String> = vec![String::new()];
    let empty = std::collections::HashMap::new();
    // a non-empty code region names the retired flat code stream -> refused.
    assert!(read_function_table(&bytes, &strings, &empty).is_err());
}

/// One raw type entry of `kind` over `payload`, as a corrupted `.mfp` hands it
/// to the reader.
fn raw_entry(kind: u16, payload: Vec<u8>) -> TypeEntry {
    TypeEntry {
        kind,
        name: 0,
        owner_package: 0,
        abi_export_kind: None,
        payload,
    }
}

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// A composite naming a table id no entry defines is refused by name, through
/// a list element and through the resource plane of a thread or worker handle.
#[test]
fn type_entry_names_refuses_a_reference_to_an_undefined_type_id() {
    let missing = FIRST_TABLE_TYPE_ID + 7;
    let strings = vec!["T".to_string()];
    for entry in [
        raw_entry(4, words(&[missing])),
        raw_entry(7, words(&[TYPE_INTEGER, TYPE_INTEGER, missing])),
        raw_entry(10, words(&[TYPE_INTEGER, TYPE_INTEGER, missing])),
    ] {
        let kind = entry.kind;
        let mut types = TypeTable::new();
        types.entries.push(entry);
        assert_eq!(
            type_entry_names(&types, &strings),
            Err(format!("unknown type id {missing}")),
            "kind {kind}"
        );
    }
}

/// A FUNC export pointed at a type entry is not a type export.
#[test]
fn decode_type_export_refuses_a_function_export() {
    let entry = raw_entry(1, Vec::new());
    assert_eq!(
        decode_type_export(
            "f",
            BinaryReprExportKind::Func,
            &entry,
            &HashMap::new(),
            &[]
        )
        .map(|export| export.name),
        Err("export `f` is not a type export".to_string())
    );
}

/// A record field with a visibility no writer emits is refused.
#[test]
fn decode_type_export_refuses_an_unknown_field_visibility() {
    let entry = raw_entry(1, words(&[1, 0, TYPE_INTEGER, 9]));
    let strings = vec!["x".to_string()];
    assert_eq!(
        decode_type_export(
            "R",
            BinaryReprExportKind::Type,
            &entry,
            &HashMap::new(),
            &strings
        )
        .map(|export| export.name),
        Err("unsupported type field visibility 9".to_string())
    );
}

/// Bytes past the last enum member are refused, not ignored.
#[test]
fn decode_type_export_refuses_trailing_payload_bytes() {
    let entry = raw_entry(3, words(&[0, 42]));
    assert_eq!(
        decode_type_export(
            "E",
            BinaryReprExportKind::Enum,
            &entry,
            &HashMap::new(),
            &[]
        )
        .map(|export| export.name),
        Err("exported type `E` has trailing payload bytes".to_string())
    );
}

#[test]
fn abi_serializer_names_a_type_kind_it_does_not_define() {
    // A decoded table can carry a kind this compiler does not define: it is
    // hashed by name alone (`opaque` + the name), so two such entries differ
    // exactly when their names do — and a name index past the string table is
    // refused rather than hashed as garbage.
    let strings: Vec<String> = vec!["Mystery".to_string()];
    let entry = |name: u32| TypeEntry {
        kind: 99,
        name,
        owner_package: 0,
        abi_export_kind: None,
        payload: Vec::new(),
    };
    let constants = ConstPool::new();
    let mut types = TypeTable::new();
    types.entries.push(entry(0));
    let mut serializer = AbiSerializer::new(&strings, &types, &constants);
    serializer
        .serialize_type(FIRST_TABLE_TYPE_ID)
        .expect("an unknown kind serializes by name");
    let contains = |haystack: &[u8], needle: &str| {
        haystack
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    assert!(contains(&serializer.bytes, "opaque"));
    assert!(contains(&serializer.bytes, "Mystery"));

    let mut dangling = TypeTable::new();
    dangling.entries.push(entry(5));
    let mut serializer = AbiSerializer::new(&strings, &dangling, &constants);
    assert!(serializer.serialize_type(FIRST_TABLE_TYPE_ID).is_err());
}

#[test]
fn a_signed_header_whose_signature_runs_past_the_file_is_truncated() {
    // A well-formed signed header (Ed25519, 64 bytes) whose file ends right
    // after it: the signature bytes are missing, which is reported as such
    // rather than read past the end.
    let mut bytes = wrap_mfp(&[], "p", "p", "1.0.0");
    let at = bytes.len() - 6;
    bytes[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
    bytes[at + 2..at + 6].copy_from_slice(&64u32.to_le_bytes());
    match mfp_binary_repr_payload(&bytes) {
        Ok(_) => panic!("a signature past the end of the file was accepted"),
        Err(message) => assert_eq!(message, "truncated .mfp signature"),
    }
}
