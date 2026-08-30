// ---------------------------------------------------------------------------
// sections.rs — StringPool, TypeTable, ConstPool, ImportTable, AbiIndex.
// ---------------------------------------------------------------------------

use super::*;

#[test]
fn string_pool_interns_and_dedups() {
    let mut pool = StringPool::new();
    let a = pool.intern("alpha");
    let b = pool.intern("beta");
    let a2 = pool.intern("alpha");
    assert_eq!(a, a2);
    assert_ne!(a, b);
    let bytes = pool.encode();
    let decoded = read_string_pool(&bytes).expect("decode string pool");
    assert_eq!(decoded, vec!["alpha".to_string(), "beta".to_string()]);
}

#[test]
fn type_id_maps_primitives_and_composites() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    assert_eq!(types.type_id(&mut strings, "Nothing"), TYPE_NOTHING);
    assert_eq!(types.type_id(&mut strings, "Boolean"), TYPE_BOOLEAN);
    assert_eq!(types.type_id(&mut strings, "Integer"), TYPE_INTEGER);
    assert_eq!(types.type_id(&mut strings, "Float"), TYPE_FLOAT);
    assert_eq!(types.type_id(&mut strings, "Fixed"), TYPE_FIXED);
    assert_eq!(types.type_id(&mut strings, "Money"), TYPE_MONEY);
    assert_eq!(types.type_id(&mut strings, "String"), TYPE_STRING);
    assert_eq!(types.type_id(&mut strings, "Byte"), TYPE_BYTE);
    assert_eq!(types.type_id(&mut strings, "fs.File"), TYPE_FILE_HANDLE);
    assert_eq!(
        types.type_id(&mut strings, "tcp.Socket"),
        TYPE_SOCKET_HANDLE
    );
    assert_eq!(
        types.type_id(&mut strings, "tcp.Listener"),
        TYPE_LISTENER_HANDLE
    );
    assert_eq!(types.type_id(&mut strings, "Error"), TYPE_ERROR);
    assert_eq!(types.type_id(&mut strings, "TermColor"), TYPE_TERM_COLOR);
    assert_eq!(types.type_id(&mut strings, "TermSize"), TYPE_TERM_SIZE);

    // Composite names get fresh table ids (>= FIRST_TABLE_TYPE_ID) and are
    // interned so a repeated name resolves to the same id.
    let list = types.type_id(&mut strings, "List OF Integer");
    assert!(list >= FIRST_TABLE_TYPE_ID);
    assert_eq!(types.type_id(&mut strings, "List OF Integer"), list);
    let nested = types.type_id(&mut strings, "List OF List OF String");
    assert_ne!(nested, list);
    let result = types.type_id(&mut strings, "Result OF Integer");
    assert_ne!(result, list);
    let map = types.type_id(&mut strings, "Map OF String TO Integer");
    assert_ne!(map, result);
    let entry = types.type_id(&mut strings, "MapEntry OF String TO Integer");
    assert_ne!(entry, map);
    // `Set OF T` gets its own id, distinct from a `List OF T` of the same element.
    let set = types.type_id(&mut strings, "Set OF Integer");
    assert!(set >= FIRST_TABLE_TYPE_ID);
    assert_ne!(set, list);
    assert_eq!(types.type_id(&mut strings, "Set OF Integer"), set);
    let func = types.type_id(&mut strings, "FUNC(Integer) AS Boolean");
    assert_ne!(func, entry);
    let iso = types.type_id(&mut strings, "ISOLATED FUNC() AS Nothing");
    assert_ne!(iso, func);
    // An unknown bare name registers as a fresh opaque record type.
    let opaque = types.type_id(&mut strings, "MyType");
    assert!(opaque >= FIRST_TABLE_TYPE_ID);
    assert_eq!(types.type_id(&mut strings, "MyType"), opaque);
}

#[test]
fn type_id_composites_decode_back_to_source_names() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    for name in [
        "List OF Integer",
        "Set OF Integer",
        "Set OF List OF String",
        "Map OF String TO Integer",
        "Result OF Integer",
        "MapEntry OF String TO Integer",
        "FUNC(Integer, String) AS Boolean",
        "ISOLATED FUNC() AS Nothing",
    ] {
        types.type_id(&mut strings, name);
    }
    let names = type_entry_names(&types, &strings.values).expect("decode names");
    let decoded: std::collections::HashSet<&str> = names.values().map(String::as_str).collect();
    assert!(decoded.contains("List OF Integer"));
    assert!(decoded.contains("Set OF Integer"));
    assert!(decoded.contains("Set OF List OF String"));
    assert!(decoded.contains("Map OF String TO Integer"));
    assert!(decoded.contains("Result OF Integer"));
    assert!(decoded.contains("MapEntry OF String TO Integer"));
    assert!(decoded.contains("FUNC(Integer, String) AS Boolean"));
    assert!(decoded.contains("ISOLATED FUNC() AS Nothing"));
}

#[test]
fn state_carrying_resource_type_round_trips() {
    // plan-52-D §4: a resource carrying `STATE T` is a composite (kind 11), not
    // an opaque name. It MUST decode back to the `" STATE "` spelling: a
    // consumer reads an imported function's signature from these ABI exports,
    // so a return encoded without its STATE would silently degrade every
    // importer to a bare handle.
    //
    // Before kind 11 the name matched no arm and fell to the `_` fallback,
    // which interned it as an empty RECORD entry — and the reader then failed
    // the whole package with "truncated binary representation".
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    // A user record as the payload, plus a built-in resource as the base.
    types.add_entry(&mut strings, "pkg", "Cursor", 1, {
        let mut payload = Vec::new();
        put_u32(&mut payload, 0);
        payload
    });
    let id = types.type_id(&mut strings, "File STATE Cursor");
    let names = type_entry_names(&types, &strings.values).expect("decode names");
    assert_eq!(
        names.get(&id).map(String::as_str),
        Some("File STATE Cursor")
    );
    // Interning the same spelling twice reuses the entry (keyed `State#b#s`).
    assert_eq!(types.type_id(&mut strings, "File STATE Cursor"), id);
}

#[test]
fn deep_acyclic_type_chain_is_rejected_not_overflow() {
    // bug-153: a long *linear* chain of distinct composite types (id N →
    // List OF id(N-1) → … → List OF Integer) passes the cycle guard (no id
    // repeats) but must be rejected by the depth cap before it overflows the
    // native stack. Build one link past the cap.
    let links = MAX_TYPE_GRAPH_DEPTH + 5;
    let mut raw: HashMap<u32, (u16, u32, Vec<u8>)> = HashMap::new();
    for i in 0..links {
        let id = FIRST_TABLE_TYPE_ID + i as u32;
        // Link 0 (deepest) points at a primitive; every other at the next id.
        let child = if i == 0 {
            TYPE_INTEGER
        } else {
            FIRST_TABLE_TYPE_ID + (i - 1) as u32
        };
        raw.insert(id, (4, 0, child.to_le_bytes().to_vec())); // kind 4 = List
    }
    let strings: Vec<String> = Vec::new();
    let head = FIRST_TABLE_TYPE_ID + (links - 1) as u32;
    let err = decode_type_name(
        head,
        &raw,
        &strings,
        &mut HashMap::new(),
        &mut HashSet::new(),
    )
    .expect_err("deep chain must be rejected");
    assert!(err.contains("too deep"), "unexpected error: {err}");

    // A shallow chain well within the cap still decodes (the cap must not
    // reject legitimate graphs).
    let shallow_head = FIRST_TABLE_TYPE_ID + 3;
    let name = decode_type_name(
        shallow_head,
        &raw,
        &strings,
        &mut HashMap::new(),
        &mut HashSet::new(),
    )
    .expect("a shallow chain decodes");
    assert!(name.starts_with("List OF "));
}

#[test]
fn thread_types_round_trip_with_and_without_resource() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    let data_only = types.thread_type(&mut strings, TYPE_INTEGER, None, TYPE_STRING);
    let with_res = types.thread_type(
        &mut strings,
        TYPE_INTEGER,
        Some(TYPE_FILE_HANDLE),
        TYPE_STRING,
    );
    assert_ne!(data_only, with_res);
    let worker = types.thread_worker_type(&mut strings, TYPE_INTEGER, None, TYPE_STRING);
    assert_ne!(worker, data_only);
    let names = type_entry_names(&types, &strings.values).expect("names");
    assert!(names.values().any(|n| n.starts_with("Thread OF ")));
    assert!(names.values().any(|n| n.starts_with("ThreadWorker OF ")));
}

#[test]
fn type_table_encode_decode_round_trips_payloads() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    types.type_id(&mut strings, "List OF Integer");
    types.type_id(&mut strings, "Map OF String TO Integer");
    let bytes = types.encode();
    let decoded = read_type_entries(&bytes, &strings.values).expect("decode types");
    assert_eq!(decoded.entries.len(), types.entries.len());
    // ids map preserved
    assert!(decoded.ids.contains_key("List#3"));
}

/// A `Map` whose KEY is itself a `Map` must split at the TOP-LEVEL ` TO `.
///
/// The wire encoder used to `split_once(" TO ")`, which takes the LEFTMOST
/// separator — the same mis-split bug-108.2 fixed in the front end. For
/// `Map OF Map OF String TO Integer TO Boolean` that yields key
/// `Map OF String` and value `Integer TO Boolean`: two types that do not exist,
/// interned into the `.mfp`'s type table. plan-106-E routes the split through
/// the canonical grammar, where the rule lives once.
#[test]
fn a_nested_map_key_splits_at_the_top_level_separator() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    types.type_id(&mut strings, "Map OF Map OF String TO Integer TO Boolean");
    let names = type_entry_names(&types, &strings.values).expect("names");
    let interned: Vec<&String> = names.values().collect();
    assert!(
        interned.iter().any(|n| *n == "Map OF String TO Integer"),
        "the inner Map must be interned as the KEY; got {interned:?}"
    );
    // A leftmost split would have interned these two non-existent types instead.
    assert!(
        !interned.iter().any(|n| *n == "Integer TO Boolean"),
        "bogus value type interned; got {interned:?}"
    );
    assert!(
        !interned.iter().any(|n| *n == "Map OF String"),
        "bogus key type interned; got {interned:?}"
    );
}

#[test]
fn const_pool_stores_every_scalar_kind() {
    let mut strings = StringPool::new();
    let mut pool = ConstPool::new();
    let kinds = [
        ("Nothing", ""),
        ("String", "hi"),
        ("Integer", "-42"),
        ("Float", "3.5"),
        ("Fixed", "1.25"),
        ("Boolean", "true"),
        ("Byte", "255"),
        ("Money", "1.25"),
        ("Scalar", "128512"),
    ];
    for (type_, value) in kinds {
        pool.add(
            &mut strings,
            &IrValue::Const {
                type_: crate::types::ParameterType::parse(type_),
                value: value.to_string(),
            },
        )
        .expect("add const");
    }
    let bytes = pool.encode();
    let decoded = read_const_pool(&bytes).expect("decode const pool");
    assert_eq!(decoded.entries.len(), kinds.len());
    // Integer -42 round-trips through its little-endian payload.
    let int_entry = &decoded.entries[2];
    assert_eq!(int_entry.kind, 3);
    let raw = i64::from_le_bytes(int_entry.payload.clone().try_into().unwrap());
    assert_eq!(raw, -42);
    // Money `1.25` stores under wire id TYPE_MONEY with raw 125000 (5 places).
    let money_entry = &decoded.entries[decoded.entries.len() - 2];
    assert_eq!(money_entry.kind, TYPE_MONEY as u16);
    let money_raw = i64::from_le_bytes(money_entry.payload.clone().try_into().unwrap());
    assert_eq!(money_raw, 125_000);
    // Scalar U+1F600 (128512) stores under wire id TYPE_SCALAR as a 4-byte LE
    // codepoint, and re-encoding the pool is byte-identical (plan-41-B).
    let scalar_entry = decoded.entries.last().unwrap();
    assert_eq!(scalar_entry.kind, TYPE_SCALAR as u16);
    let scalar_cp = u32::from_le_bytes(scalar_entry.payload.clone().try_into().unwrap());
    assert_eq!(scalar_cp, 128_512);
    assert_eq!(read_const_pool(&bytes).expect("re-decode").encode(), bytes);
}

#[test]
fn scalar_wire_id_and_reserved_band() {
    // Scalar takes id 10; the table-type base moved to 20 (plan-41-B).
    // plan-89-A: AttributedString claimed the first reserved primitive id (11);
    // ids 12–19 remain the reserved primitive band and must have no name mapping.
    assert_eq!(TYPE_SCALAR, 10);
    assert_eq!(TYPE_ATTRIBUTED_STRING, 11);
    assert_eq!(FIRST_TABLE_TYPE_ID, 20);
    assert_eq!(primitive_type_name(TYPE_SCALAR), Some("Scalar"));
    assert_eq!(
        primitive_type_name(TYPE_ATTRIBUTED_STRING),
        Some("AttributedString")
    );
    for reserved in 12..=19 {
        assert_eq!(
            primitive_type_name(reserved),
            None,
            "reserved id {reserved} must be unmapped"
        );
    }
}

#[test]
fn const_pool_rejects_bad_values_and_types() {
    let mut strings = StringPool::new();
    let mut pool = ConstPool::new();
    for (type_, value) in [
        ("Integer", "not-a-number"),
        ("Float", "xyz"),
        ("Byte", "999"),
        ("Weird", "0"),
    ] {
        assert!(pool
            .add(
                &mut strings,
                &IrValue::Const {
                    type_: crate::types::ParameterType::parse(type_),
                    value: value.to_string(),
                },
            )
            .is_err());
    }
    // Non-const IR values are rejected.
    assert!(pool
        .add(&mut strings, &IrValue::Local("x".to_string()))
        .is_err());
}

#[test]
fn import_table_from_metadata_and_encode_round_trip() {
    let mut metadata = BinaryReprMetadata::new("pkg".to_string(), "1.0.0".to_string());
    metadata.dependencies = vec![
        BinaryReprDependency {
            name: "dep".to_string(),
            ident: String::new(),
            version: "^1".to_string(),
            pin: true,
            flags: 0x5,
        },
        BinaryReprDependency {
            name: "other".to_string(),
            ident: "other-ident".to_string(),
            version: "2".to_string(),
            pin: false,
            flags: 0,
        },
    ];
    let mut strings = StringPool::new();
    let table = ImportTable::from_metadata(&mut strings, &metadata);
    assert_eq!(table.entries.len(), 2);
    // Empty ident falls back to the name.
    assert_eq!(
        string_at(&strings.values, table.entries[0].package_ident).unwrap(),
        "dep"
    );
    assert_eq!(
        string_at(&strings.values, table.entries[1].package_ident).unwrap(),
        "other-ident"
    );
    let bytes = table.encode();
    let decoded = read_import_table(&bytes).expect("decode import table");
    assert_eq!(decoded.entries.len(), 2);
    assert!(decoded.entries[0].pin);
    assert!(!decoded.entries[1].pin);
    assert_eq!(decoded.entries[0].flags, 0x5);
}

#[test]
fn import_table_records_used_symbols() {
    let mut metadata = BinaryReprMetadata::new("pkg".to_string(), "1".to_string());
    metadata.dependencies = vec![BinaryReprDependency {
        name: "dep".to_string(),
        ident: String::new(),
        version: "1".to_string(),
        pin: false,
        flags: 0,
    }];
    let mut strings = StringPool::new();
    let mut table = ImportTable::from_metadata(&mut strings, &metadata);
    let mut used = std::collections::HashSet::new();
    used.insert("dep.foo".to_string());
    used.insert("dep.bar".to_string());
    used.insert("unrelated.baz".to_string());
    let mut hashes = std::collections::HashMap::new();
    hashes.insert("dep.foo".to_string(), hash_bytes(b"foo"));
    hashes.insert("dep.bar".to_string(), hash_bytes(b"bar"));
    table.record_used_imports(&mut strings, &used, &hashes);
    let symbols = &table.entries[0].used_symbols;
    assert_eq!(symbols.len(), 2);
    // Sorted by symbol name: bar before foo.
    assert_eq!(string_at(&strings.values, symbols[0].name).unwrap(), "bar");
    assert_eq!(string_at(&strings.values, symbols[1].name).unwrap(), "foo");
}

#[test]
fn abi_index_encode_decode_round_trips() {
    let project = super::fixtures::rich_project();
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let lowered = lower_project(&project, &metadata).expect("lower");
    let bytes = lowered.abi.encode();
    let decoded = read_abi_index(&bytes).expect("decode abi index");
    assert_eq!(decoded.exports.len(), lowered.abi.exports.len());
    // Re-encoding the decoded index is byte-identical.
    assert_eq!(decoded.encode(), bytes);
}

#[test]
fn type_id_falls_back_for_malformed_composites() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    // No " AS " terminator: the Thread/ThreadWorker parser rejects the name, so
    // it interns as an opaque entry rather than a structured composite.
    let t = types.type_id(&mut strings, "Thread OF Garbage");
    let tw = types.type_id(&mut strings, "ThreadWorker OF Garbage");
    // No " TO " separator: Map/MapEntry fall back to an opaque entry.
    let m = types.type_id(&mut strings, "Map OF Garbage");
    let me = types.type_id(&mut strings, "MapEntry OF Garbage");
    for id in [t, tw, m, me] {
        assert!(id >= FIRST_TABLE_TYPE_ID);
    }
    // Each is stable on a second intern.
    assert_eq!(types.type_id(&mut strings, "Thread OF Garbage"), t);
    assert_eq!(types.type_id(&mut strings, "Map OF Garbage"), m);
}
