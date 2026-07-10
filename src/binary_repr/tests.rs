use super::*;

#[cfg(test)]
mod doc_table_tests {
    use super::*;

    #[test]
    fn doc_table_round_trips() {
        let docs = PackageDocs {
            package: Some(PackageDocEntry {
                name: "mathx".to_string(),
                desc: vec![
                    (0, "First paragraph.".to_string()),
                    (0, "Second.".to_string()),
                ],
                deprecated: Some(String::new()),
            }),
            decls: vec![
                DeclDocEntry {
                    kind: "func".to_string(),
                    name: "addUp".to_string(),
                    signature: "EXPORT FUNC addUp(a AS Integer, b AS Integer) AS Integer"
                        .to_string(),
                    group: "Math".to_string(),
                    desc: vec![(0, "Adds.".to_string()), (1, "Overflows.".to_string())],
                    args: vec![
                        ("a".to_string(), "first".to_string()),
                        ("b".to_string(), "second".to_string()),
                    ],
                    props: vec![],
                    ret: "the sum".to_string(),
                    errors: vec![("5001".to_string(), "overflow".to_string())],
                    example: "LET x AS Integer = addUp(1, 2)".to_string(),
                    internal: false,
                    deprecated: None,
                },
                DeclDocEntry {
                    kind: "type".to_string(),
                    name: "Point".to_string(),
                    signature: "EXPORT TYPE Point".to_string(),
                    group: String::new(),
                    desc: vec![],
                    args: vec![],
                    props: vec![("x".to_string(), "the x".to_string())],
                    ret: String::new(),
                    errors: vec![],
                    example: String::new(),
                    internal: true,
                    deprecated: Some("use Coord".to_string()),
                },
            ],
        };

        let bytes = encode_doc_table(&docs);
        let decoded = read_doc_table(&bytes).expect("doc table decodes");

        let package = decoded.package.expect("package entry");
        assert_eq!(package.name, "mathx");
        assert_eq!(package.desc, docs.package.as_ref().unwrap().desc);
        assert_eq!(package.deprecated, Some(String::new()));

        assert_eq!(decoded.decls.len(), 2);
        let add = &decoded.decls[0];
        assert_eq!(add.kind, "func");
        assert_eq!(add.name, "addUp");
        assert_eq!(add.group, "Math");
        assert_eq!(
            add.desc,
            vec![(0, "Adds.".to_string()), (1, "Overflows.".to_string())]
        );
        assert_eq!(add.args, docs.decls[0].args);
        assert_eq!(add.errors, docs.decls[0].errors);
        assert_eq!(add.ret, "the sum");
        assert!(!add.internal);
        assert_eq!(add.deprecated, None);

        let point = &decoded.decls[1];
        assert_eq!(point.kind, "type");
        assert_eq!(point.props, docs.decls[1].props);
        assert!(point.internal);
        assert_eq!(point.deprecated, Some("use Coord".to_string()));
    }
}

// ---------------------------------------------------------------------------
// Shared fixtures for the writer/reader/round-trip tests below.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod fixtures {
    use super::*;
    use crate::ir::{IrBinding, IrField, IrParam, IrSourceLoc, IrType, IrVariant};

    pub(super) fn loc() -> IrSourceLoc {
        IrSourceLoc::default()
    }

    pub(super) fn const_int(value: &str) -> IrValue {
        IrValue::Const {
            type_: "Integer".to_string(),
            value: value.to_string(),
        }
    }

    pub(super) fn fn_named(name: &str, visibility: &str, kind: &str, returns: &str) -> IrFunction {
        IrFunction {
            name: name.to_string(),
            visibility: visibility.to_string(),
            kind: kind.to_string(),
            isolated: false,
            params: vec![],
            returns: returns.to_string(),
            body: vec![],
            file: "src/main.mfb".to_string(),
            resource_owners: std::collections::HashMap::new(),
            loc: loc(),
        }
    }

    pub(super) fn empty_project(name: &str) -> IrProject {
        IrProject {
            name: name.to_string(),
            entry: None,
            bindings: vec![],
            types: vec![],
            functions: vec![],
            native_resources: vec![],
            link_functions: vec![],
            link_aliases: vec![],
            docs: crate::ir::ProjectDocs::default(),
        }
    }

    /// A project that exercises most of the writer: an exported func with a
    /// defaulted param, an exported sub, a private isolated func, a record type,
    /// a union type, an enum type, globals of every visibility, and an entry
    /// point returning Integer with args.
    pub(super) fn rich_project() -> IrProject {
        let mut project = empty_project("richpkg");
        project.entry = Some(crate::ir::EntryPoint {
            name: "main".to_string(),
            returns: "Integer".to_string(),
            accepts_args: true,
        });
        project.bindings = vec![
            IrBinding {
                name: "gPriv".to_string(),
                visibility: "private".to_string(),
                mutable: true,
                type_: "Integer".to_string(),
                value: Some(const_int("1")),
                loc: loc(),
                file: String::new(),
                explicit_type: false,
            },
            IrBinding {
                name: "gPkg".to_string(),
                visibility: "public".to_string(),
                mutable: false,
                type_: "String".to_string(),
                value: None,
                loc: loc(),
                file: String::new(),
                explicit_type: false,
            },
            IrBinding {
                name: "gExp".to_string(),
                visibility: "export".to_string(),
                mutable: false,
                type_: "List OF Integer".to_string(),
                value: None,
                loc: loc(),
                file: String::new(),
                explicit_type: false,
            },
        ];
        project.types = vec![
            IrType {
                kind: "type".to_string(),
                visibility: "export".to_string(),
                name: "Point".to_string(),
                fields: vec![
                    IrField {
                        visibility: Some("export".to_string()),
                        name: "x".to_string(),
                        type_: "Integer".to_string(),
                        loc: loc(),
                    },
                    IrField {
                        visibility: Some("private".to_string()),
                        name: "y".to_string(),
                        type_: "Integer".to_string(),
                        loc: loc(),
                    },
                ],
                includes: vec![],
                variants: vec![],
                members: vec![],
                loc: loc(),
                file: "src/main.mfb".to_string(),
            },
            IrType {
                kind: "union".to_string(),
                visibility: "export".to_string(),
                name: "Shape".to_string(),
                fields: vec![],
                includes: vec![],
                variants: vec![IrVariant {
                    name: "Dot".to_string(),
                    fields: vec![IrField {
                        visibility: None,
                        name: "p".to_string(),
                        type_: "Point".to_string(),
                        loc: loc(),
                    }],
                    loc: loc(),
                }],
                members: vec![],
                loc: loc(),
                file: "src/main.mfb".to_string(),
            },
            IrType {
                kind: "enum".to_string(),
                visibility: "export".to_string(),
                name: "Color".to_string(),
                fields: vec![],
                includes: vec![],
                variants: vec![],
                members: vec![
                    crate::ir::IrEnumMember {
                        name: "Red".to_string(),
                    },
                    crate::ir::IrEnumMember {
                        name: "Green".to_string(),
                    },
                ],
                loc: loc(),
                file: "src/main.mfb".to_string(),
            },
        ];
        let mut exported = fn_named("main", "export", "function", "Integer");
        exported.params = vec![
            IrParam {
                name: "n".to_string(),
                type_: "Integer".to_string(),
                default: None,
                loc: loc(),
            },
            IrParam {
                name: "m".to_string(),
                type_: "Integer".to_string(),
                default: Some(const_int("0")),
                loc: loc(),
            },
        ];
        let mut isolated = fn_named("worker", "private", "function", "Nothing");
        isolated.isolated = true;
        project.functions = vec![
            exported,
            fn_named("doThing", "export", "sub", "Nothing"),
            isolated,
        ];
        project
    }

    /// Encode a project to inner MFPC bytes with the given metadata.
    pub(super) fn encode_project(project: &IrProject, metadata: &BinaryReprMetadata) -> Vec<u8> {
        build_binary_repr_bytes(project, metadata).expect("encode")
    }

    /// Wrap inner MFPC bytes in a minimal but valid v1.0 `.mfp` container whose
    /// header identity matches an all-empty-key manifest, so
    /// `read_package_binary_repr` accepts it.
    pub(super) fn wrap_mfp(binary_repr: &[u8], name: &str, ident: &str, version: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]);
        put_u16(&mut bytes, 1); // container major
        put_u16(&mut bytes, 0); // container minor
        put_u32(&mut bytes, 0); // reserved (offset 12..16)
        put_u32(&mut bytes, 0); // reserved (offset 16..20)
        let put_len = |bytes: &mut Vec<u8>, s: &str| {
            put_u32(bytes, s.len() as u32);
            bytes.extend_from_slice(s.as_bytes());
        };
        put_len(&mut bytes, name);
        put_len(&mut bytes, ident);
        put_len(&mut bytes, version);
        put_len(&mut bytes, ""); // author
        put_len(&mut bytes, ""); // url
        put_len(&mut bytes, ""); // identKey
        put_len(&mut bytes, ""); // signingKey
        put_len(&mut bytes, ""); // proof
        put_len(&mut bytes, ""); // proofSig
        put_len(&mut bytes, ""); // attestation
        put_len(&mut bytes, ""); // attestationSig
        bytes.extend_from_slice(&[0u8; 32]); // packageBinaryHash
        put_u64(&mut bytes, binary_repr.len() as u64);
        put_u16(&mut bytes, 0); // signature type (unsigned)
        put_u32(&mut bytes, 0); // signature length
        bytes.extend_from_slice(binary_repr);
        bytes
    }

    /// Write a `.mfp` byte blob to a temp file and return its path.
    pub(super) fn temp_mfp(bytes: &[u8]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("mfb-binrepr-test-{}-{}.mfp", std::process::id(), n));
        std::fs::write(&path, bytes).expect("write temp mfp");
        path
    }
}

#[cfg(test)]
mod resource_table_tests {
    use super::*;

    #[test]
    fn standard_flags_set_sendable_bit_for_movable_resources() {
        let file = standard_resource_flags(builtins::fs::FILE_TYPE);
        let socket = standard_resource_flags(builtins::net::SOCKET_TYPE);
        let listener = standard_resource_flags(builtins::net::LISTENER_TYPE);
        assert!(file & RESOURCE_FLAG_SENDABLE != 0, "File must be sendable");
        assert!(
            socket & RESOURCE_FLAG_SENDABLE != 0,
            "Socket must be sendable"
        );
        assert!(
            listener & RESOURCE_FLAG_SENDABLE == 0,
            "Listener must not be sendable"
        );
        // The other standard flags remain set.
        for flags in [file, socket, listener] {
            assert!(flags & RESOURCE_FLAG_NATIVE != 0);
            assert!(flags & RESOURCE_FLAG_STANDARD != 0);
            assert!(flags & RESOURCE_FLAG_CLOSE_MAY_FAIL != 0);
        }
    }

    #[test]
    fn resource_table_round_trips_flags() {
        let table = ResourceTable {
            entries: vec![
                ResourceEntry {
                    type_id: 10,
                    close_function_id: BUILTIN_FS_CLOSE_FUNCTION_ID,
                    flags: standard_resource_flags(builtins::fs::FILE_TYPE),
                },
                ResourceEntry {
                    type_id: 11,
                    close_function_id: BUILTIN_NET_CLOSE_FUNCTION_ID,
                    flags: standard_resource_flags(builtins::net::LISTENER_TYPE),
                },
            ],
        };
        let bytes = table.encode();
        let decoded = read_resource_table(&bytes).expect("decode resource table");
        assert_eq!(decoded.entries.len(), 2);
        assert_eq!(decoded.entries[0].type_id, 10);
        assert_eq!(
            decoded.entries[0].close_function_id,
            BUILTIN_FS_CLOSE_FUNCTION_ID
        );
        assert!(decoded.entries[0].flags & RESOURCE_FLAG_SENDABLE != 0);
        assert!(decoded.entries[1].flags & RESOURCE_FLAG_SENDABLE == 0);
        assert_eq!(
            decoded.entries[1].close_function_id,
            BUILTIN_NET_CLOSE_FUNCTION_ID
        );
    }

    #[test]
    fn native_resource_entry_has_native_flag_without_standard() {
        // A native LINK resource carries NATIVE but not STANDARD; this is how
        // decode tells it from a built-in (plan-link-update.md §10).
        let mut strings = StringPool::new();
        let mut table = ResourceTable::new();
        let native = crate::ir::IrNativeResource {
            name: "Db".to_string(),
            visibility: "export".to_string(),
            close_function: "sqliteLink.close".to_string(),
            sendable: false,
            close_may_fail: true,
        };
        table.add_native(&mut strings, 42, &native);
        let entry = &table.entries[0];
        assert!(entry.flags & RESOURCE_FLAG_NATIVE != 0);
        assert!(entry.flags & RESOURCE_FLAG_STANDARD == 0);
        assert!(entry.flags & RESOURCE_FLAG_CLOSE_MAY_FAIL != 0);
        assert!(entry.flags & RESOURCE_FLAG_SENDABLE == 0);
        // The close op name round-trips through the string pool.
        let bytes = table.encode();
        let decoded = read_resource_table(&bytes).expect("decode resource table");
        assert_eq!(decoded.entries[0].type_id, 42);
        assert_eq!(
            string_at(&strings.values, decoded.entries[0].close_function_id).unwrap(),
            "sqliteLink.close"
        );
    }

    #[test]
    fn native_resource_sendable_bit_round_trips() {
        let mut strings = StringPool::new();
        let mut table = ResourceTable::new();
        let native = crate::ir::IrNativeResource {
            name: "Conn".to_string(),
            visibility: "export".to_string(),
            close_function: "lib.close".to_string(),
            sendable: true,
            close_may_fail: false,
        };
        table.add_native(&mut strings, 7, &native);
        let bytes = table.encode();
        let decoded = read_resource_table(&bytes).expect("decode resource table");
        assert!(decoded.entries[0].flags & RESOURCE_FLAG_SENDABLE != 0);
        assert!(decoded.entries[0].flags & RESOURCE_FLAG_CLOSE_MAY_FAIL == 0);
    }
}

// ---------------------------------------------------------------------------
// util.rs — low-level cursor readers, capacity guards, section framing.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod util_tests {
    use super::*;

    #[test]
    fn cursor_scalars_round_trip() {
        let mut bytes = Vec::new();
        bytes.push(0xAB);
        put_u16(&mut bytes, 0x1234);
        put_u32(&mut bytes, 0xDEAD_BEEF);
        put_u64(&mut bytes, 0x0102_0304_0506_0708);
        let mut offset = 0;
        assert_eq!(cursor_u8(&bytes, &mut offset).unwrap(), 0xAB);
        assert_eq!(cursor_u16(&bytes, &mut offset).unwrap(), 0x1234);
        assert_eq!(cursor_u32(&bytes, &mut offset).unwrap(), 0xDEAD_BEEF);
        assert_eq!(
            cursor_u64(&bytes, &mut offset).unwrap(),
            0x0102_0304_0506_0708
        );
        assert_eq!(offset, bytes.len());
    }

    #[test]
    fn cursor_scalars_reject_truncation() {
        let mut o = 0;
        assert!(cursor_u8(&[], &mut o).is_err());
        let mut o = 0;
        assert!(cursor_u16(&[0], &mut o).is_err());
        let mut o = 0;
        assert!(cursor_u32(&[0, 0, 0], &mut o).is_err());
        let mut o = 0;
        assert!(cursor_u64(&[0; 7], &mut o).is_err());
        let mut o = 0;
        assert!(cursor_hash(&[0; 31], &mut o).is_err());
    }

    #[test]
    fn cursor_string_round_trips_and_rejects_bad_input() {
        let mut bytes = Vec::new();
        put_bytes(&mut bytes, "héllo".as_bytes());
        let mut offset = 0;
        assert_eq!(cursor_string(&bytes, &mut offset).unwrap(), "héllo");
        assert_eq!(offset, bytes.len());

        // Truncated body (claims 10 bytes, has 2).
        let mut bad = Vec::new();
        put_u32(&mut bad, 10);
        bad.extend_from_slice(b"ab");
        let mut o = 0;
        assert!(cursor_string(&bad, &mut o).is_err());

        // Invalid UTF-8 body.
        let mut invalid = Vec::new();
        put_u32(&mut invalid, 1);
        invalid.push(0xFF);
        let mut o = 0;
        assert!(cursor_string(&invalid, &mut o).is_err());
    }

    #[test]
    fn cursor_hash_reads_thirty_two_bytes() {
        let data: Vec<u8> = (0..32u8).collect();
        let mut offset = 0;
        let hash = cursor_hash(&data, &mut offset).unwrap();
        assert_eq!(hash.to_vec(), data);
        assert_eq!(offset, 32);
    }

    #[test]
    fn cursor_prose_and_pair_and_optional_round_trip() {
        let mut bytes = Vec::new();
        put_prose_list(&mut bytes, &[(0, "a".to_string()), (2, "b".to_string())]);
        put_pair_list(&mut bytes, &[("k".to_string(), "v".to_string())]);
        put_optional_str(&mut bytes, &Some("present".to_string()));
        put_optional_str(&mut bytes, &None);

        let mut offset = 0;
        let prose = cursor_prose_list(&bytes, &mut offset).unwrap();
        assert_eq!(prose, vec![(0, "a".to_string()), (2, "b".to_string())]);
        let pairs = cursor_pair_list(&bytes, &mut offset).unwrap();
        assert_eq!(pairs, vec![("k".to_string(), "v".to_string())]);
        assert_eq!(
            cursor_optional_str(&bytes, &mut offset).unwrap(),
            Some("present".to_string())
        );
        assert_eq!(cursor_optional_str(&bytes, &mut offset).unwrap(), None);
        assert_eq!(offset, bytes.len());
    }

    #[test]
    fn cursor_prose_list_rejects_truncated_kind() {
        // Count says 1 element but no kind byte follows.
        let mut bytes = Vec::new();
        put_u32(&mut bytes, 1);
        let mut o = 0;
        assert!(cursor_prose_list(&bytes, &mut o).is_err());
    }

    #[test]
    fn cursor_optional_str_rejects_truncated_flag() {
        let mut o = 0;
        assert!(cursor_optional_str(&[], &mut o).is_err());
    }

    #[test]
    fn bounded_capacity_caps_to_remaining() {
        // A hostile count is clamped by remaining/min_elem.
        assert_eq!(bounded_capacity(u32::MAX as usize, 40, 8), 5);
        // A small count passes through unchanged.
        assert_eq!(bounded_capacity(3, 40, 8), 3);
        // min_elem of 0 is treated as 1 (no div-by-zero).
        assert_eq!(bounded_capacity(7, 100, 0), 7);
    }

    #[test]
    fn hash_bytes_and_hex_hash_are_consistent() {
        let hash = hash_bytes(b"abc");
        assert_eq!(hash.len(), ABI_HASH_LEN);
        let hex = hex_hash(&hash);
        assert_eq!(hex.len(), ABI_HASH_LEN * 2);
        // SHA-256("abc") starts with ba7816bf.
        assert!(hex.starts_with("ba7816bf"));
    }

    #[test]
    fn sorted_pairs_orders_lexicographically() {
        let sorted = sorted_pairs(vec![
            ("b".to_string(), "2".to_string()),
            ("a".to_string(), "1".to_string()),
        ]);
        assert_eq!(sorted[0].0, "a");
        assert_eq!(sorted[1].0, "b");
    }

    #[test]
    fn length_prefixed_helpers_round_trip_and_reject() {
        let mut bytes = Vec::new();
        put_bytes(&mut bytes, b"payload");
        put_bytes(&mut bytes, b"skip-me");
        let mut offset = 0;
        assert_eq!(
            read_length_prefixed(&bytes, &mut offset, "f").unwrap(),
            "payload"
        );
        skip_length_prefixed(&bytes, &mut offset, "g").unwrap();
        assert_eq!(offset, bytes.len());

        // Truncated: claims a long length.
        let mut bad = Vec::new();
        put_u32(&mut bad, 100);
        bad.extend_from_slice(b"x");
        let mut o = 0;
        assert!(read_length_prefixed(&bad, &mut o, "f").is_err());
        let mut o = 0;
        assert!(skip_length_prefixed(&bad, &mut o, "f").is_err());

        // Non-UTF8 in read_length_prefixed.
        let mut invalid = Vec::new();
        put_u32(&mut invalid, 1);
        invalid.push(0xFF);
        let mut o = 0;
        assert!(read_length_prefixed(&invalid, &mut o, "f").is_err());
    }

    #[test]
    fn checked_scalars_reject_out_of_bounds() {
        assert!(checked_u16_at(&[0], 0).is_err());
        assert!(checked_u32_at(&[0, 0], 0).is_err());
        assert!(checked_u64_at(&[0; 4], 0).is_err());
        // Overflowing offset.
        assert!(checked_u16_at(&[0; 4], usize::MAX).is_err());
    }

    #[test]
    fn encode_sections_frames_header_and_offsets() {
        let sections = vec![Section::new(1, vec![1, 2, 3]), Section::new(2, vec![9, 9])];
        let bytes = encode_sections(&sections);
        assert_eq!(&bytes[0..4], b"MFPC");
        // major version at offset 4.
        assert_eq!(checked_u16_at(&bytes, 4).unwrap(), MFPC_MAJOR_VERSION);
        // section count at offset 12.
        assert_eq!(checked_u32_at(&bytes, 12).unwrap(), 2);
        // First section table entry: id 1, offset points past the header+table.
        assert_eq!(checked_u16_at(&bytes, 16).unwrap(), 1);
        let first_off = checked_u64_at(&bytes, 16 + 8).unwrap() as usize;
        assert_eq!(first_off, 16 + 2 * 24);
        assert_eq!(&bytes[first_off..first_off + 3], &[1, 2, 3]);
    }

    #[test]
    fn hex_dump_formats_rows_of_sixteen() {
        let out = hex_dump(&[0xAB, 0x00, 0xFF]);
        assert_eq!(out, "AB 00 FF\n");
        // 17 bytes wraps to a second line.
        let data: Vec<u8> = (0..17u8).collect();
        let dump = hex_dump(&data);
        assert_eq!(dump.lines().count(), 2);
    }
}

// ---------------------------------------------------------------------------
// sections.rs — StringPool, TypeTable, ConstPool, ImportTable, AbiIndex.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod sections_tests {
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
        assert_eq!(types.type_id(&mut strings, "String"), TYPE_STRING);
        assert_eq!(types.type_id(&mut strings, "Byte"), TYPE_BYTE);
        assert_eq!(types.type_id(&mut strings, "File"), TYPE_FILE_HANDLE);
        assert_eq!(types.type_id(&mut strings, "Socket"), TYPE_SOCKET_HANDLE);
        assert_eq!(
            types.type_id(&mut strings, "Listener"),
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
        assert!(decoded.contains("Map OF String TO Integer"));
        assert!(decoded.contains("Result OF Integer"));
        assert!(decoded.contains("MapEntry OF String TO Integer"));
        assert!(decoded.contains("FUNC(Integer, String) AS Boolean"));
        assert!(decoded.contains("ISOLATED FUNC() AS Nothing"));
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
        ];
        for (type_, value) in kinds {
            pool.add(
                &mut strings,
                &IrValue::Const {
                    type_: type_.to_string(),
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
                        type_: type_.to_string(),
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
}

// ---------------------------------------------------------------------------
// writer.rs — lowering an IrProject to the section model + helper parsers.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod writer_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn lower_project_populates_all_tables() {
        let project = rich_project();
        let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
        let lowered = lower_project(&project, &metadata).expect("lower");
        assert_eq!(lowered.functions.len(), 3);
        assert_eq!(lowered.globals.len(), 3);
        // Point, Shape, Color plus any composite (List OF Integer) types.
        assert!(lowered.types.entries.len() >= 3);
        // Entry function flags: bit0 set, args bit set, Integer-return bit set.
        assert_eq!(lowered.entry_flags & 1, 1);
        assert_eq!(lowered.entry_flags & (1 << 1), 1 << 1);
        assert_eq!(lowered.entry_flags & (1 << 2), 1 << 2);
        assert_ne!(lowered.entry_function, u32::MAX);
        // Exactly the two exported callables (main, doThing) are exported.
        assert_eq!(lowered.export_count(), 2);
    }

    #[test]
    fn lower_project_without_entry_uses_sentinel() {
        let mut project = empty_project("noentry");
        project.functions = vec![fn_named("f", "private", "function", "Integer")];
        let metadata = BinaryReprMetadata::new("noentry".to_string(), "1".to_string());
        let lowered = lower_project(&project, &metadata).expect("lower");
        assert_eq!(lowered.entry_function, u32::MAX);
        assert_eq!(lowered.entry_flags, 0);
        assert_eq!(lowered.export_count(), 0);
    }

    #[test]
    fn lower_project_missing_entry_function_errors() {
        let mut project = empty_project("badentry");
        project.entry = Some(crate::ir::EntryPoint {
            name: "ghost".to_string(),
            returns: "Nothing".to_string(),
            accepts_args: false,
        });
        let metadata = BinaryReprMetadata::new("badentry".to_string(), "1".to_string());
        assert!(lower_project(&project, &metadata).is_err());
    }

    #[test]
    fn native_resources_add_type_and_resource_entry() {
        let mut project = empty_project("linkpkg");
        project.native_resources = vec![
            crate::ir::IrNativeResource {
                name: "Db".to_string(),
                visibility: "export".to_string(),
                close_function: "lib.close".to_string(),
                sendable: true,
                close_may_fail: true,
            },
            crate::ir::IrNativeResource {
                name: "Priv".to_string(),
                visibility: "private".to_string(),
                close_function: "lib.shut".to_string(),
                sendable: false,
                close_may_fail: false,
            },
        ];
        let metadata = BinaryReprMetadata::new("linkpkg".to_string(), "1".to_string());
        let lowered = lower_project(&project, &metadata).expect("lower");
        assert_eq!(lowered.resources.entries.len(), 2);
        // The exported native resource carries an ABI export kind.
        assert!(lowered
            .types
            .entries
            .iter()
            .any(|entry| entry.abi_export_kind.is_some()));
    }

    #[test]
    fn split_top_level_types_respects_nesting() {
        assert!(split_top_level_types("").is_empty());
        assert!(split_top_level_types("  ").is_empty());
        assert_eq!(
            split_top_level_types("Integer, String"),
            vec!["Integer".to_string(), "String".to_string()]
        );
        // A comma inside a nested FUNC(...) is not a top-level separator.
        assert_eq!(
            split_top_level_types("FUNC(Integer, String) AS Boolean, Byte"),
            vec![
                "FUNC(Integer, String) AS Boolean".to_string(),
                "Byte".to_string()
            ]
        );
    }

    #[test]
    fn parse_function_type_handles_isolated_and_plain() {
        let plain = parse_function_type("FUNC(Integer, String) AS Boolean").unwrap();
        assert!(!plain.isolated);
        assert_eq!(
            plain.params,
            vec!["Integer".to_string(), "String".to_string()]
        );
        assert_eq!(plain.returns, "Boolean");

        let iso = parse_function_type("ISOLATED FUNC() AS Nothing").unwrap();
        assert!(iso.isolated);
        assert!(iso.params.is_empty());
        assert_eq!(iso.returns, "Nothing");

        // Not a function type.
        assert!(parse_function_type("Integer").is_none());
        // Missing ") AS " terminator.
        assert!(parse_function_type("FUNC(Integer").is_none());
    }

    #[test]
    fn split_function_type_rest_finds_top_level_terminator() {
        assert_eq!(
            split_function_type_rest("Integer) AS Boolean"),
            Some(("Integer", "Boolean"))
        );
        // Nested parens are skipped until the top-level ") AS ".
        assert_eq!(
            split_function_type_rest("FUNC() AS Integer) AS Boolean"),
            Some(("FUNC() AS Integer", "Boolean"))
        );
        assert_eq!(split_function_type_rest("Integer"), None);
    }

    #[test]
    fn fixed_raw_from_decimal_covers_signs_fractions_and_rounding() {
        // Whole number.
        assert_eq!(fixed_raw_from_decimal("2").unwrap(), 2i64 << 32);
        // Negative.
        assert_eq!(fixed_raw_from_decimal("-2").unwrap(), -(2i64 << 32));
        // 0.5 == half of the scale.
        assert_eq!(fixed_raw_from_decimal("0.5").unwrap(), 1i64 << 31);
        // Leading-dot form.
        assert_eq!(fixed_raw_from_decimal(".5").unwrap(), 1i64 << 31);
        // Rounds up when the fractional remainder is >= half.
        let quarter = fixed_raw_from_decimal("0.25").unwrap();
        assert_eq!(quarter, 1i64 << 30);
    }

    #[test]
    fn fixed_raw_from_decimal_rejects_malformed() {
        assert!(fixed_raw_from_decimal("").is_err());
        assert!(fixed_raw_from_decimal(".").is_err());
        assert!(fixed_raw_from_decimal("1.2x").is_err());
        assert!(fixed_raw_from_decimal("notanumber").is_err());
        // Out of i64 range after scaling.
        assert!(fixed_raw_from_decimal("99999999999999").is_err());
    }

    #[test]
    fn ir_uses_resource_type_detects_file_param() {
        let mut project = empty_project("res");
        let mut f = fn_named("takesFile", "export", "sub", "Nothing");
        f.params = vec![crate::ir::IrParam {
            name: "h".to_string(),
            type_: "File".to_string(),
            default: None,
            loc: loc(),
        }];
        project.functions = vec![f];
        assert!(ir_uses_resource_type(&project));

        let plain = empty_project("plain");
        assert!(!ir_uses_resource_type(&plain));
    }

    #[test]
    fn is_resource_type_name_matches_builtins() {
        assert!(is_resource_type_name("File"));
        assert!(is_resource_type_name("Socket"));
        assert!(!is_resource_type_name("Integer"));
    }

    #[test]
    fn standard_resource_flags_marks_sendable_types() {
        let file = standard_resource_flags(builtins::fs::FILE_TYPE);
        assert!(file & RESOURCE_FLAG_SENDABLE != 0);
        let listener = standard_resource_flags(builtins::net::LISTENER_TYPE);
        assert!(listener & RESOURCE_FLAG_SENDABLE == 0);
    }

    #[test]
    fn source_type_payload_encodes_union_and_enum() {
        use crate::ir::{IrField, IrType, IrVariant};
        let mut strings = StringPool::new();
        let mut types = TypeTable::new();
        let union = IrType {
            kind: "union".to_string(),
            visibility: "export".to_string(),
            name: "U".to_string(),
            fields: vec![],
            includes: vec![],
            variants: vec![IrVariant {
                name: "A".to_string(),
                fields: vec![IrField {
                    visibility: None,
                    name: "v".to_string(),
                    type_: "Integer".to_string(),
                    loc: loc(),
                }],
                loc: loc(),
            }],
            members: vec![],
            loc: loc(),
            file: String::new(),
        };
        let source_types = std::collections::HashMap::new();
        let payload = source_type_payload(&mut strings, &mut types, &source_types, &union)
            .expect("union payload");
        // First u32 is the variant count.
        assert_eq!(checked_u32_at(&payload, 0).unwrap(), 1);
    }

    #[test]
    fn concrete_union_variants_rejects_unknown_include() {
        use crate::ir::IrType;
        let bad = IrType {
            kind: "union".to_string(),
            visibility: "export".to_string(),
            name: "U".to_string(),
            fields: vec![],
            includes: vec!["Missing".to_string()],
            variants: vec![],
            members: vec![],
            loc: loc(),
            file: String::new(),
        };
        let source_types = std::collections::HashMap::new();
        assert!(concrete_union_variants(&source_types, &bad).is_err());
    }
}

// ---------------------------------------------------------------------------
// reader.rs — decode paths, error handling, and container framing.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod reader_tests {
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
        assert_eq!(primitive_type_name(TYPE_FILE_HANDLE), Some("File"));
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
}

// ---------------------------------------------------------------------------
// builder.rs — package_exports / package_info / package_type_exports / resources.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod builder_tests {
    use super::fixtures::*;
    use super::*;

    fn decoded_package() -> PackageBinaryRepr {
        let inner = encode_project(
            &rich_project(),
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        read_binary_repr_package(&inner).expect("decode package")
    }

    #[test]
    fn package_exports_lists_callables_with_signatures() {
        let package = decoded_package();
        let exports = package_exports(&package).expect("exports");
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(names.contains(&"doThing"));
        let main = exports.iter().find(|e| e.name == "main").unwrap();
        assert_eq!(main.return_type, "Integer");
        assert_eq!(main.params.len(), 2);
        // The defaulted parameter carries has_default.
        assert!(main.params[1].has_default);
        assert!(!main.params[0].has_default);
    }

    #[test]
    fn package_info_reports_counts_and_metadata() {
        let package = decoded_package();
        let info = package_info(&package).expect("info");
        assert_eq!(info.manifest_name, "richpkg");
        assert_eq!(info.manifest_version, "1.0.0");
        assert_eq!(info.function_count, 3);
        assert_eq!(info.global_count, 3);
        // Two exported callables plus exported Point/Shape/Color type exports.
        assert!(info.export_count >= 2);
        assert_eq!(info.abi_format_version, ABI_FORMAT_VERSION);
        // Globals report visibility strings.
        let visibilities: Vec<&str> = info.globals.iter().map(|g| g.visibility.as_str()).collect();
        assert!(visibilities.contains(&"private"));
        assert!(visibilities.contains(&"public"));
        assert!(visibilities.contains(&"export"));
    }

    #[test]
    fn package_type_exports_decodes_record_union_enum() {
        let package = decoded_package();
        let types = package_type_exports(&package).expect("type exports");
        let point = types.iter().find(|t| t.name == "Point").expect("Point");
        assert!(point.kind == BinaryReprExportKind::Type);
        assert_eq!(point.fields.len(), 2);
        let shape = types.iter().find(|t| t.name == "Shape").expect("Shape");
        assert!(shape.kind == BinaryReprExportKind::Union);
        assert_eq!(shape.variants.len(), 1);
        let color = types.iter().find(|t| t.name == "Color").expect("Color");
        assert!(color.kind == BinaryReprExportKind::Enum);
        assert_eq!(color.members, vec!["Red".to_string(), "Green".to_string()]);
    }

    #[test]
    fn resolve_resource_close_name_maps_builtins_and_functions() {
        let package = decoded_package();
        assert_eq!(
            resolve_resource_close_name(&package, BUILTIN_FS_CLOSE_FUNCTION_ID).unwrap(),
            builtins::resource_close_function(builtins::fs::FILE_TYPE).map(str::to_string)
        );
        assert_eq!(
            resolve_resource_close_name(&package, BUILTIN_NET_CLOSE_FUNCTION_ID).unwrap(),
            builtins::resource_close_function(builtins::net::SOCKET_TYPE).map(str::to_string)
        );
        // A function-id index resolves to that function's name.
        let named = resolve_resource_close_name(&package, 0).unwrap();
        assert!(named.is_some());
        // An out-of-range id resolves to None.
        assert!(resolve_resource_close_name(&package, u32::MAX - 5)
            .unwrap()
            .is_none());
    }

    #[test]
    fn package_resource_exports_decodes_native_link_resource() {
        let mut project = empty_project("linkpkg");
        project.native_resources = vec![crate::ir::IrNativeResource {
            name: "Db".to_string(),
            visibility: "export".to_string(),
            close_function: "lib.close".to_string(),
            sendable: true,
            close_may_fail: true,
        }];
        let inner = encode_project(
            &project,
            &BinaryReprMetadata::new("linkpkg".to_string(), "1".to_string()),
        );
        let package = read_binary_repr_package(&inner).expect("decode");
        let resources = package_resource_exports(&package).expect("resources");
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].type_name, "Db");
        assert!(resources[0].native);
        assert!(resources[0].sendable);
        assert_eq!(resources[0].close_function.as_deref(), Some("lib.close"));
    }
}

// ---------------------------------------------------------------------------
// mod.rs — public entry points and full end-to-end round-trips.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod mod_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn build_and_read_package_exports_end_to_end() {
        let inner = encode_project(
            &rich_project(),
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        let path = temp_mfp(&wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0"));
        let exports = read_package_exports(&path).expect("exports");
        assert_eq!(exports.len(), 2);
        let info = read_package_info(&path).expect("info");
        assert_eq!(info.manifest_name, "richpkg");
        let type_exports = read_package_type_exports(&path).expect("type exports");
        assert!(type_exports.iter().any(|t| t.name == "Point"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_package_ir_with_identity_round_trips_the_ir() {
        let project = rich_project();
        let inner = encode_project(
            &project,
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        let path = temp_mfp(&wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0"));
        let (id, ir) = read_package_ir_with_identity(&path).expect("ir with identity");
        assert_eq!(id.len(), 16);
        // The decoded IR carries the same function names as the source project.
        let decoded_names: std::collections::HashSet<String> =
            ir.functions.iter().map(|f| f.name.clone()).collect();
        for f in &project.functions {
            assert!(decoded_names.contains(&f.name));
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_package_docs_returns_docs_or_empty() {
        // A project with no DOC blocks yields empty docs.
        let inner = encode_project(
            &empty_project("nodocs"),
            &BinaryReprMetadata::new("nodocs".to_string(), "1".to_string()),
        );
        let path = temp_mfp(&wrap_mfp(&inner, "nodocs", "nodocs", "1"));
        let docs = read_package_docs(&path).expect("docs");
        assert!(docs.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_package_resources_reads_standard_and_native() {
        let mut project = empty_project("linkpkg");
        project.native_resources = vec![crate::ir::IrNativeResource {
            name: "Conn".to_string(),
            visibility: "export".to_string(),
            close_function: "lib.close".to_string(),
            sendable: false,
            close_may_fail: true,
        }];
        let inner = encode_project(
            &project,
            &BinaryReprMetadata::new("linkpkg".to_string(), "1".to_string()),
        );
        let path = temp_mfp(&wrap_mfp(&inner, "linkpkg", "linkpkg", "1"));
        let resources = read_package_resources(&path).expect("resources");
        assert!(resources.iter().any(|r| r.type_name == "Conn"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn public_read_helpers_error_on_missing_file() {
        let missing = std::path::Path::new("/nonexistent/does-not-exist.mfp");
        assert!(read_package_exports(missing).is_err());
        assert!(read_package_info(missing).is_err());
        assert!(read_package_docs(missing).is_err());
        assert!(read_package_resources(missing).is_err());
        assert!(read_package_ir_with_identity(missing).is_err());
    }

    #[test]
    fn write_binary_repr_hex_writes_a_hex_file() {
        let dir = std::env::temp_dir().join(format!(
            "mfb-binrepr-hex-{}-{}",
            std::process::id(),
            "richpkg"
        ));
        let _ = std::fs::create_dir_all(&dir);
        let project = rich_project();
        let path = write_binary_repr_hex(&dir, &project, "1.0.0").expect("write hex");
        let contents = std::fs::read_to_string(&path).expect("read hex");
        // Hex dump uses uppercase two-digit bytes; the MFPC magic leads.
        assert!(contents.starts_with("4D 46 50 43") || contents.starts_with("4D46"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn binary_repr_metadata_new_defaults_are_empty() {
        let metadata = BinaryReprMetadata::new("pkg".to_string(), "1.0.0".to_string());
        assert_eq!(metadata.name, "pkg");
        assert_eq!(metadata.version, "1.0.0");
        assert!(metadata.ident.is_empty());
        assert!(metadata.dependencies.is_empty());
    }
}

// ---------------------------------------------------------------------------
// writer.rs — resource + imported-call IR walkers over every op/value arm.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod writer_walker_tests {
    use super::fixtures::*;
    use super::*;
    use crate::ast::LoopKind;
    use crate::ir::{IrMatchCase, IrMatchPattern, IrParam, IrRecordUpdate, IrSourceLoc};

    fn file_local() -> IrValue {
        IrValue::LocalRef {
            name: "h".to_string(),
            type_: "File".to_string(),
        }
    }

    // A value that touches every IrValue arm the walkers recurse through, each
    // carrying a `File` resource type so the resource walkers record it.
    fn every_value() -> Vec<IrValue> {
        vec![
            IrValue::Const {
                type_: "File".to_string(),
                value: "0".to_string(),
            },
            IrValue::Local("a".to_string()),
            IrValue::Global("g".to_string()),
            IrValue::LocalRef {
                name: "a".to_string(),
                type_: "File".to_string(),
            },
            IrValue::FunctionRef {
                name: "dep.helper".to_string(),
                type_: "File".to_string(),
            },
            IrValue::Closure {
                name: "dep.helper".to_string(),
                type_: "File".to_string(),
                captures: vec![IrValue::Local("a".to_string())],
            },
            IrValue::Capture {
                index: 0,
                type_: "File".to_string(),
                by_ref: true,
            },
            IrValue::Call {
                target: "dep.helper".to_string(),
                args: vec![file_local()],
                loc: IrSourceLoc::default(),
                type_: "File".to_string(),
            },
            IrValue::CallResult {
                target: "dep.helper".to_string(),
                args: vec![file_local()],
                loc: IrSourceLoc::default(),
                type_: "File".to_string(),
            },
            IrValue::Constructor {
                type_: "File".to_string(),
                args: vec![file_local()],
            },
            IrValue::UnionWrap {
                union_type: "U".to_string(),
                member_type: "File".to_string(),
                value: Box::new(file_local()),
            },
            IrValue::UnionExtract {
                type_: "File".to_string(),
                value: Box::new(file_local()),
            },
            IrValue::ResultIsOk {
                value: Box::new(file_local()),
            },
            IrValue::ResultValue {
                type_: "File".to_string(),
                value: Box::new(file_local()),
            },
            IrValue::ResultError {
                value: Box::new(file_local()),
            },
            IrValue::WithUpdate {
                type_: "File".to_string(),
                target: Box::new(file_local()),
                updates: vec![IrRecordUpdate {
                    field: "x".to_string(),
                    value: file_local(),
                }],
            },
            IrValue::ListLiteral {
                type_: "File".to_string(),
                values: vec![file_local()],
            },
            IrValue::MapLiteral {
                type_: "File".to_string(),
                entries: vec![(file_local(), file_local())],
            },
            IrValue::MemberAccess {
                target: Box::new(file_local()),
                member: "m".to_string(),
                type_: "File".to_string(),
            },
            IrValue::Binary {
                op: "+".to_string(),
                left: Box::new(file_local()),
                right: Box::new(file_local()),
                loc: IrSourceLoc::default(),
                type_: "File".to_string(),
            },
            IrValue::Unary {
                op: "-".to_string(),
                operand: Box::new(file_local()),
                loc: IrSourceLoc::default(),
                type_: "File".to_string(),
            },
        ]
    }

    // A function body touching every IrOp arm the walkers recurse through.
    fn every_op_body() -> Vec<IrOp> {
        let call = IrValue::Call {
            target: "dep.helper".to_string(),
            args: every_value(),
            loc: IrSourceLoc::default(),
            type_: "File".to_string(),
        };
        vec![
            IrOp::Bind {
                mutable: true,
                name: "a".to_string(),
                type_: "File".to_string(),
                value: Some(call.clone()),
                loc: IrSourceLoc::default(),
                explicit_type: true,
            },
            IrOp::Assign {
                name: "a".to_string(),
                value: file_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::AssignGlobal {
                name: "g".to_string(),
                value: file_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::StateAssign {
                resource: "a".to_string(),
                value: file_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::Eval {
                value: call.clone(),
                loc: IrSourceLoc::default(),
            },
            IrOp::Return {
                value: Some(file_local()),
                loc: IrSourceLoc::default(),
            },
            IrOp::ExitProgram {
                code: file_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::Fail {
                error: file_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::ExitLoop {
                kind: LoopKind::While,
                loc: IrSourceLoc::default(),
            },
            IrOp::ContinueLoop {
                kind: LoopKind::While,
                loc: IrSourceLoc::default(),
            },
            IrOp::If {
                condition: file_local(),
                then_body: vec![IrOp::Eval {
                    value: file_local(),
                    loc: IrSourceLoc::default(),
                }],
                else_body: vec![IrOp::Eval {
                    value: file_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::Match {
                value: file_local(),
                cases: vec![IrMatchCase {
                    pattern: IrMatchPattern::Value(file_local()),
                    guard: Some(file_local()),
                    body: vec![IrOp::Eval {
                        value: file_local(),
                        loc: IrSourceLoc::default(),
                    }],
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::While {
                kind: LoopKind::While,
                condition: file_local(),
                body: vec![IrOp::Eval {
                    value: file_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::For {
                name: "i".to_string(),
                type_: "File".to_string(),
                start: file_local(),
                end: file_local(),
                step: file_local(),
                body: vec![IrOp::Eval {
                    value: file_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::DoUntil {
                body: vec![IrOp::Eval {
                    value: file_local(),
                    loc: IrSourceLoc::default(),
                }],
                condition: file_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::ForEach {
                name: "e".to_string(),
                type_: "File".to_string(),
                iterable: file_local(),
                body: vec![IrOp::Eval {
                    value: file_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::Trap {
                name: "err".to_string(),
                body: vec![IrOp::Eval {
                    value: file_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
        ]
    }

    fn corpus_function() -> IrFunction {
        let mut f = fn_named("corpus", "export", "function", "File");
        f.params = vec![IrParam {
            name: "h".to_string(),
            type_: "File".to_string(),
            default: None,
            loc: loc(),
        }];
        f.body = every_op_body();
        f
    }

    #[test]
    fn ir_uses_resource_type_walks_every_op_and_value_arm() {
        let mut project = empty_project("walk");
        project.functions = vec![corpus_function()];
        assert!(ir_uses_resource_type(&project));

        // ops_use_resource_type and value_uses_resource_type directly.
        let body = every_op_body();
        assert!(ops_use_resource_type(&body));
        for value in every_value() {
            // Every arm carries a File type somewhere, so the value walker sees it.
            assert!(
                value_uses_resource_type(&value)
                    || matches!(value, IrValue::Local(_) | IrValue::Global(_))
            );
        }
    }

    // A `File`-free twin of `file_local`, so a body built from it makes every
    // op/value arm evaluate to `false` and `.any(..)` must visit them all.
    fn plain_local() -> IrValue {
        IrValue::Local("x".to_string())
    }

    fn every_plain_value() -> Vec<IrValue> {
        vec![
            IrValue::Const {
                type_: "Integer".to_string(),
                value: "0".to_string(),
            },
            IrValue::Local("a".to_string()),
            IrValue::Global("g".to_string()),
            IrValue::LocalRef {
                name: "a".to_string(),
                type_: "Integer".to_string(),
            },
            IrValue::FunctionRef {
                name: "f".to_string(),
                type_: "Integer".to_string(),
            },
            IrValue::Closure {
                name: "f".to_string(),
                type_: "Integer".to_string(),
                captures: vec![plain_local()],
            },
            IrValue::Capture {
                index: 0,
                type_: "Integer".to_string(),
                by_ref: false,
            },
            IrValue::Call {
                target: "f".to_string(),
                args: vec![plain_local()],
                loc: IrSourceLoc::default(),
                type_: "Integer".to_string(),
            },
            IrValue::CallResult {
                target: "f".to_string(),
                args: vec![plain_local()],
                loc: IrSourceLoc::default(),
                type_: "Integer".to_string(),
            },
            IrValue::Constructor {
                type_: "Integer".to_string(),
                args: vec![plain_local()],
            },
            IrValue::UnionWrap {
                union_type: "U".to_string(),
                member_type: "Integer".to_string(),
                value: Box::new(plain_local()),
            },
            IrValue::UnionExtract {
                type_: "Integer".to_string(),
                value: Box::new(plain_local()),
            },
            IrValue::ResultIsOk {
                value: Box::new(plain_local()),
            },
            IrValue::ResultValue {
                type_: "Integer".to_string(),
                value: Box::new(plain_local()),
            },
            IrValue::ResultError {
                value: Box::new(plain_local()),
            },
            IrValue::WithUpdate {
                type_: "Integer".to_string(),
                target: Box::new(plain_local()),
                updates: vec![IrRecordUpdate {
                    field: "x".to_string(),
                    value: plain_local(),
                }],
            },
            IrValue::ListLiteral {
                type_: "List OF Integer".to_string(),
                values: vec![plain_local()],
            },
            IrValue::MapLiteral {
                type_: "Map OF String TO Integer".to_string(),
                entries: vec![(plain_local(), plain_local())],
            },
            IrValue::MemberAccess {
                target: Box::new(plain_local()),
                member: "m".to_string(),
                type_: "Integer".to_string(),
            },
            IrValue::Binary {
                op: "+".to_string(),
                left: Box::new(plain_local()),
                right: Box::new(plain_local()),
                loc: IrSourceLoc::default(),
                type_: "Integer".to_string(),
            },
            IrValue::Unary {
                op: "-".to_string(),
                operand: Box::new(plain_local()),
                loc: IrSourceLoc::default(),
                type_: "Integer".to_string(),
            },
        ]
    }

    fn every_plain_op_body() -> Vec<IrOp> {
        vec![
            IrOp::Bind {
                mutable: true,
                name: "a".to_string(),
                type_: "Integer".to_string(),
                value: Some(plain_local()),
                loc: IrSourceLoc::default(),
                explicit_type: true,
            },
            IrOp::Assign {
                name: "a".to_string(),
                value: plain_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::AssignGlobal {
                name: "g".to_string(),
                value: plain_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::StateAssign {
                resource: "a".to_string(),
                value: plain_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::Eval {
                value: plain_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::Return {
                value: Some(plain_local()),
                loc: IrSourceLoc::default(),
            },
            IrOp::ExitProgram {
                code: plain_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::Fail {
                error: plain_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::ExitLoop {
                kind: LoopKind::While,
                loc: IrSourceLoc::default(),
            },
            IrOp::ContinueLoop {
                kind: LoopKind::While,
                loc: IrSourceLoc::default(),
            },
            IrOp::If {
                condition: plain_local(),
                then_body: vec![IrOp::Eval {
                    value: plain_local(),
                    loc: IrSourceLoc::default(),
                }],
                else_body: vec![IrOp::Eval {
                    value: plain_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::Match {
                value: plain_local(),
                cases: vec![IrMatchCase {
                    pattern: IrMatchPattern::Value(plain_local()),
                    guard: Some(plain_local()),
                    body: vec![IrOp::Eval {
                        value: plain_local(),
                        loc: IrSourceLoc::default(),
                    }],
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::While {
                kind: LoopKind::While,
                condition: plain_local(),
                body: vec![IrOp::Eval {
                    value: plain_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::For {
                name: "i".to_string(),
                type_: "Integer".to_string(),
                start: plain_local(),
                end: plain_local(),
                step: plain_local(),
                body: vec![IrOp::Eval {
                    value: plain_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::DoUntil {
                body: vec![IrOp::Eval {
                    value: plain_local(),
                    loc: IrSourceLoc::default(),
                }],
                condition: plain_local(),
                loc: IrSourceLoc::default(),
            },
            IrOp::ForEach {
                name: "e".to_string(),
                type_: "Integer".to_string(),
                iterable: plain_local(),
                body: vec![IrOp::Eval {
                    value: plain_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::Trap {
                name: "err".to_string(),
                body: vec![IrOp::Eval {
                    value: plain_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
        ]
    }

    #[test]
    fn resource_walkers_visit_every_arm_when_no_resource_present() {
        // `.any(..)` short-circuits on the first `true`, so a resource-free body
        // is required to make the walkers traverse every op/value match arm.
        let body = every_plain_op_body();
        assert!(!ops_use_resource_type(&body));
        for value in every_plain_value() {
            assert!(!value_uses_resource_type(&value));
        }
        // The imported-call walker also visits every arm (no import matches).
        let empty: std::collections::HashMap<String, [u8; ABI_HASH_LEN]> =
            std::collections::HashMap::new();
        let mut used = std::collections::HashSet::new();
        for op in &body {
            collect_imported_calls_op(op, &empty, &mut used);
        }
        for value in every_plain_value() {
            collect_imported_calls_value(&value, &empty, &mut used);
        }
        assert!(used.is_empty());
        // collect_resource_names_in_ops/value over a resource-free body records nothing.
        let mut names = std::collections::HashSet::new();
        let mut record = |type_: &str, names: &mut std::collections::HashSet<String>| {
            if is_resource_type_name(type_) {
                names.insert(type_.to_string());
            }
        };
        collect_resource_names_in_ops(&body, &mut names, &mut record);
        for value in every_plain_value() {
            collect_resource_names_in_value(&value, &mut names, &mut record);
        }
        assert!(names.is_empty());
    }

    #[test]
    fn collect_resource_names_walk_gathers_file_over_every_arm() {
        // Same coverage over the name-collecting walkers, but with File present.
        let body = every_op_body();
        let mut names = std::collections::HashSet::new();
        let mut record = |type_: &str, names: &mut std::collections::HashSet<String>| {
            if is_resource_type_name(type_) {
                names.insert(type_.to_string());
            }
        };
        collect_resource_names_in_ops(&body, &mut names, &mut record);
        assert!(names.contains("File"));
        for value in every_value() {
            collect_resource_names_in_value(&value, &mut names, &mut record);
        }
        assert!(names.contains("File"));
    }

    #[test]
    fn collect_resource_type_names_gathers_file() {
        let mut project = empty_project("walk");
        project.functions = vec![corpus_function()];
        let mut names = std::collections::HashSet::new();
        collect_resource_type_names(&project, &mut names);
        assert!(names.contains("File"));
    }

    #[test]
    fn lower_project_emits_file_resource_table_from_body() {
        // The resource walker drives the RESOURCE_TABLE, so lowering a body that
        // uses File must emit a standard file resource entry.
        let mut project = empty_project("walk");
        project.functions = vec![corpus_function()];
        let metadata = BinaryReprMetadata::new("walk".to_string(), "1".to_string());
        let lowered = lower_project(&project, &metadata).expect("lower");
        assert!(lowered
            .resources
            .entries
            .iter()
            .any(|e| e.close_function_id == BUILTIN_FS_CLOSE_FUNCTION_ID));
    }

    #[test]
    fn collect_imported_calls_records_used_symbols() {
        // Build a fake imported-hash map naming `dep.helper`, then walk a body
        // that references it in every recursive position.
        let mut imported = std::collections::HashMap::new();
        imported.insert("dep.helper".to_string(), hash_bytes(b"helper"));
        let mut used = std::collections::HashSet::new();
        for op in every_op_body() {
            collect_imported_calls_op(&op, &imported, &mut used);
        }
        assert!(used.contains("dep.helper"));
    }

    #[test]
    fn socket_and_listener_resources_are_emitted_when_used() {
        let mut project = empty_project("net");
        let mut f = fn_named("takes", "export", "sub", "Nothing");
        f.params = vec![
            IrParam {
                name: "s".to_string(),
                type_: "Socket".to_string(),
                default: None,
                loc: loc(),
            },
            IrParam {
                name: "l".to_string(),
                type_: "Listener".to_string(),
                default: None,
                loc: loc(),
            },
        ];
        project.functions = vec![f];
        let metadata = BinaryReprMetadata::new("net".to_string(), "1".to_string());
        let lowered = lower_project(&project, &metadata).expect("lower");
        // Socket + Listener both produce resource entries.
        assert_eq!(lowered.resources.entries.len(), 2);
    }
}

// ---------------------------------------------------------------------------
// writer.rs — cross-package lowering (external function metadata + import path).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod cross_package_tests {
    use super::fixtures::*;
    use super::*;
    use crate::ir::IrSourceLoc;

    fn dep_project() -> IrProject {
        let mut dep = empty_project("dep");
        dep.functions = vec![fn_named("helper", "export", "function", "Integer")];
        dep
    }

    fn write_dep_mfp() -> std::path::PathBuf {
        let inner = encode_project(
            &dep_project(),
            &BinaryReprMetadata::new("dep".to_string(), "1.0.0".to_string()),
        );
        temp_mfp(&wrap_mfp(&inner, "dep", "dep", "1.0.0"))
    }

    #[test]
    fn lower_package_project_resolves_external_calls() {
        let dep_path = write_dep_mfp();
        // A consumer that calls `dep.helper`.
        let mut consumer = empty_project("app");
        let mut main = fn_named("main", "export", "function", "Integer");
        main.body = vec![IrOp::Return {
            value: Some(IrValue::Call {
                target: "dep.helper".to_string(),
                args: vec![],
                loc: IrSourceLoc::default(),
                type_: "Integer".to_string(),
            }),
            loc: IrSourceLoc::default(),
        }];
        consumer.functions = vec![main];
        let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
        metadata.dependencies = vec![BinaryReprDependency {
            name: "dep".to_string(),
            ident: String::new(),
            version: "1.0.0".to_string(),
            pin: false,
            flags: 0,
        }];
        let lowered = lower_package_project(&consumer, &metadata, &[dep_path.clone()])
            .expect("lower package");
        // The import table records the used symbol `helper`.
        let used = &lowered.imports.entries[0].used_symbols;
        assert_eq!(used.len(), 1);
        assert_eq!(
            string_at(&lowered.strings.values, used[0].name).unwrap(),
            "helper"
        );
        let _ = std::fs::remove_file(&dep_path);
    }

    #[test]
    fn external_function_metadata_assigns_ids_and_hashes() {
        let dep_path = write_dep_mfp();
        let package = read_package_binary_repr(&dep_path).expect("decode dep");
        let (ids, returns, hashes) =
            external_function_metadata(5, std::slice::from_ref(&package)).expect("metadata");
        assert!(ids.contains_key("dep.helper"));
        // Base id 5 + the function's own id 0.
        assert_eq!(ids["dep.helper"], 5);
        assert_eq!(returns["dep.helper"], "Integer");
        assert!(hashes.contains_key("dep.helper"));
        let _ = std::fs::remove_file(&dep_path);
    }

    #[test]
    fn build_package_binary_repr_bytes_round_trips() {
        let dep_path = write_dep_mfp();
        let consumer = empty_project("app");
        let metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
        let bytes = build_package_binary_repr_bytes(&consumer, &metadata, &[dep_path.clone()])
            .expect("build");
        assert!(read_binary_repr_package(&bytes).is_ok());
        let _ = std::fs::remove_file(&dep_path);
    }
}

// ---------------------------------------------------------------------------
// builder.rs + reader.rs — package_info over imports/docs; ABI validation errs.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod package_info_and_validation_tests {
    use super::fixtures::*;
    use super::*;
    use crate::ir::IrSourceLoc;

    fn dep_mfp() -> std::path::PathBuf {
        let mut dep = empty_project("dep");
        dep.functions = vec![fn_named("helper", "export", "function", "Integer")];
        let inner = encode_project(
            &dep,
            &BinaryReprMetadata::new("dep".to_string(), "1.0.0".to_string()),
        );
        temp_mfp(&wrap_mfp(&inner, "dep", "dep", "1.0.0"))
    }

    /// A consumer package importing `dep`, calling `dep.helper`, with docs.
    fn consumer_with_import_and_docs() -> (IrProject, BinaryReprMetadata) {
        let mut consumer = empty_project("app");
        let mut main = fn_named("run", "export", "function", "Integer");
        main.body = vec![IrOp::Return {
            value: Some(IrValue::Call {
                target: "dep.helper".to_string(),
                args: vec![],
                loc: IrSourceLoc::default(),
                type_: "Integer".to_string(),
            }),
            loc: IrSourceLoc::default(),
        }];
        consumer.functions = vec![main];
        consumer.docs = crate::ir::ProjectDocs {
            package: Some(crate::ir::IrPackageDoc {
                name: "app".to_string(),
                desc: vec![(0, "An app.".to_string())],
                deprecated: None,
            }),
            decls: vec![crate::ir::IrDocDecl {
                kind: crate::ir::IrDocKind::Func,
                name: "run".to_string(),
                signature: "EXPORT FUNC run() AS Integer".to_string(),
                group: String::new(),
                desc: vec![(0, "Runs.".to_string())],
                args: vec![],
                props: vec![],
                ret: "the answer".to_string(),
                errors: vec![],
                example: String::new(),
                internal: false,
                deprecated: None,
            }],
        };
        let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
        metadata.dependencies = vec![BinaryReprDependency {
            name: "dep".to_string(),
            ident: String::new(),
            version: "1.0.0".to_string(),
            pin: true,
            flags: 0,
        }];
        (consumer, metadata)
    }

    fn write_consumer() -> (std::path::PathBuf, std::path::PathBuf) {
        let dep = dep_mfp();
        let (consumer, metadata) = consumer_with_import_and_docs();
        let inner =
            build_package_binary_repr_bytes(&consumer, &metadata, &[dep.clone()]).expect("build");
        let path = temp_mfp(&wrap_mfp(&inner, "app", "app", "1.0.0"));
        (path, dep)
    }

    #[test]
    fn package_info_reports_imports_and_used_symbols() {
        let (path, dep) = write_consumer();
        let info = read_package_info(&path).expect("info");
        assert_eq!(info.import_count, 1);
        let import = &info.imports[0];
        assert_eq!(import.package_name, "dep");
        assert!(import.pin);
        // The consumer references `dep.helper`, recorded as a used symbol.
        assert!(import.used_symbols.iter().any(|s| s.name == "helper"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&dep);
    }

    #[test]
    fn package_docs_round_trip_through_container() {
        let (path, dep) = write_consumer();
        let docs = read_package_docs(&path).expect("docs");
        assert!(!docs.is_empty());
        assert_eq!(docs.package.as_ref().unwrap().name, "app");
        assert!(docs.decls.iter().any(|d| d.name == "run"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&dep);
    }

    #[test]
    fn read_package_binary_repr_decodes_import_and_doc_sections() {
        let (path, dep) = write_consumer();
        let package = read_package_binary_repr(&path).expect("decode");
        assert_eq!(package.project.imports.entries.len(), 1);
        assert!(!package.project.docs.is_empty());
        assert!(!package.project.abi.dep_edges.is_empty());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&dep);
    }

    #[test]
    fn validate_abi_index_rejects_sig_hash_mismatch() {
        // Decode a valid package, then corrupt an ABI export sig hash and
        // re-validate: the sig-hash check must fail.
        let inner = encode_project(
            &rich_project(),
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        let package = read_binary_repr_package(&inner).expect("decode");
        let mut abi = package.project.abi.clone();
        if let Some(export) = abi.exports.first_mut() {
            export.sig_hash[0] ^= 0xFF;
        }
        let err = validate_abi_index(
            &abi,
            &package.exports,
            &package.project.imports,
            &package.project.strings.values,
            &package.project.types,
            &package.project.constants,
            &package.project.functions,
        );
        assert!(err.is_err());
    }

    #[test]
    fn validate_abi_index_rejects_missing_export_entry() {
        let inner = encode_project(
            &rich_project(),
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        let package = read_binary_repr_package(&inner).expect("decode");
        let mut abi = package.project.abi.clone();
        abi.exports.clear(); // now no ABI entry backs the EXPORT_TABLE
        let err = validate_abi_index(
            &abi,
            &package.exports,
            &package.project.imports,
            &package.project.strings.values,
            &package.project.types,
            &package.project.constants,
            &package.project.functions,
        );
        assert!(err.is_err());
    }

    #[test]
    fn abi_export_for_decoded_finds_matching_entry() {
        let inner = encode_project(
            &rich_project(),
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        let package = read_binary_repr_package(&inner).expect("decode");
        let export = &package.exports[0];
        assert!(abi_export_for_decoded(&package.project.abi, export).is_some());
    }
}

// ---------------------------------------------------------------------------
// Coverage top-up: thread type names, enum/union payloads, cleanups, and the
// remaining ABI-validation / decode error branches.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod gap_tests {
    use super::fixtures::*;
    use super::*;
    use crate::ir::{IrEnumMember, IrField, IrType, IrVariant};

    #[test]
    fn type_id_parses_thread_source_names() {
        let mut strings = StringPool::new();
        let mut types = TypeTable::new();
        let data = types.type_id(&mut strings, "Thread OF Integer TO String");
        let with_res = types.type_id(&mut strings, "Thread OF Integer RES File TO String");
        let worker = types.type_id(&mut strings, "ThreadWorker OF Integer TO String");
        assert!(data >= FIRST_TABLE_TYPE_ID);
        assert_ne!(data, with_res);
        assert_ne!(worker, data);
        let names = type_entry_names(&types, &strings.values).expect("names");
        assert!(names.values().any(|n| n == "Thread OF Integer TO String"));
        assert!(names
            .values()
            .any(|n| n == "ThreadWorker OF Integer TO String"));
    }

    #[test]
    fn source_type_payload_encodes_enum_members_with_ordinals() {
        let mut strings = StringPool::new();
        let mut types = TypeTable::new();
        let enum_type = IrType {
            kind: "enum".to_string(),
            visibility: "export".to_string(),
            name: "Color".to_string(),
            fields: vec![],
            includes: vec![],
            variants: vec![],
            members: vec![
                IrEnumMember {
                    name: "Red".to_string(),
                },
                IrEnumMember {
                    name: "Green".to_string(),
                },
            ],
            loc: loc(),
            file: String::new(),
        };
        let source_types = std::collections::HashMap::new();
        let payload = source_type_payload(&mut strings, &mut types, &source_types, &enum_type)
            .expect("enum payload");
        assert_eq!(checked_u32_at(&payload, 0).unwrap(), 2); // member count
                                                             // Second field is the first member's ordinal (0).
        assert_eq!(checked_u32_at(&payload, 8).unwrap(), 0);
    }

    #[test]
    fn concrete_union_variants_flattens_included_unions() {
        let base = IrType {
            kind: "union".to_string(),
            visibility: "export".to_string(),
            name: "Base".to_string(),
            fields: vec![],
            includes: vec![],
            variants: vec![IrVariant {
                name: "A".to_string(),
                fields: vec![],
                loc: loc(),
            }],
            members: vec![],
            loc: loc(),
            file: String::new(),
        };
        let derived = IrType {
            kind: "union".to_string(),
            visibility: "export".to_string(),
            name: "Derived".to_string(),
            fields: vec![],
            includes: vec!["Base".to_string()],
            variants: vec![IrVariant {
                name: "B".to_string(),
                fields: vec![IrField {
                    visibility: None,
                    name: "v".to_string(),
                    type_: "Integer".to_string(),
                    loc: loc(),
                }],
                loc: loc(),
            }],
            members: vec![],
            loc: loc(),
            file: String::new(),
        };
        let mut source_types = std::collections::HashMap::new();
        source_types.insert("Base", &base);
        source_types.insert("Derived", &derived);
        let variants = concrete_union_variants(&source_types, &derived).expect("flatten");
        // Base's A followed by Derived's B.
        let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["A", "B"]);
    }

    #[test]
    fn union_with_includes_round_trips_variants_through_encode() {
        // Exercise the writer's union-include flattening end to end.
        let mut project = empty_project("uni");
        project.types = vec![
            IrType {
                kind: "union".to_string(),
                visibility: "public".to_string(),
                name: "Base".to_string(),
                fields: vec![],
                includes: vec![],
                variants: vec![IrVariant {
                    name: "A".to_string(),
                    fields: vec![],
                    loc: loc(),
                }],
                members: vec![],
                loc: loc(),
                file: String::new(),
            },
            IrType {
                kind: "union".to_string(),
                visibility: "export".to_string(),
                name: "Derived".to_string(),
                fields: vec![],
                includes: vec!["Base".to_string()],
                variants: vec![IrVariant {
                    name: "B".to_string(),
                    fields: vec![],
                    loc: loc(),
                }],
                members: vec![],
                loc: loc(),
                file: String::new(),
            },
        ];
        let metadata = BinaryReprMetadata::new("uni".to_string(), "1".to_string());
        let bytes = build_binary_repr_bytes(&project, &metadata).expect("encode");
        let package = read_binary_repr_package(&bytes).expect("decode");
        let type_exports = package_type_exports(&package).expect("type exports");
        let derived = type_exports
            .iter()
            .find(|t| t.name == "Derived")
            .expect("Derived");
        // The exported union carries both the included and own variants.
        assert_eq!(derived.variants.len(), 2);
    }

    #[test]
    fn encode_functions_emits_registers_and_cleanups() {
        // The writer never emits cleanups from lowering, so build a project and
        // splice a cleanup + register into a function before re-encoding.
        let project = rich_project();
        let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
        let mut lowered = lower_project(&project, &metadata).expect("lower");
        lowered.functions[0].registers.push(Register {
            type_id: TYPE_INTEGER,
            flags: 0,
        });
        lowered.functions[0].cleanups.push(Cleanup {
            id: 7,
            start_pc: 1,
            end_pc: 2,
            resource_register: 0,
            close_function_id: BUILTIN_FS_CLOSE_FUNCTION_ID,
            flags: CLEANUP_FLAG_RECORD_SECONDARY_CLOSE_FAILURE,
        });
        let bytes = lowered.encode();
        let package = read_binary_repr_package(&bytes).expect("decode");
        let info = package_info(&package).expect("info");
        assert_eq!(info.cleanup_count, 1);
        let cleanup = &info.cleanups[0];
        assert_eq!(cleanup.cleanup_id, 7);
        assert!(cleanup.records_secondary_close_failure);
    }

    #[test]
    fn validate_abi_index_rejects_dep_request_and_symbol_mismatches() {
        // Build a consumer with a dependency edge, then corrupt the edge's
        // version and used symbols to trip both disagreement branches.
        let mut consumer = empty_project("app");
        consumer.functions = vec![fn_named("run", "export", "function", "Integer")];
        let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
        metadata.dependencies = vec![BinaryReprDependency {
            name: "dep".to_string(),
            ident: String::new(),
            version: "1.0.0".to_string(),
            pin: false,
            flags: 0,
        }];
        let lowered = lower_project(&consumer, &metadata).expect("lower");
        let bytes = lowered.encode();
        let package = read_binary_repr_package(&bytes).expect("decode");

        // Version-request disagreement.
        let mut abi = package.project.abi.clone();
        abi.dep_edges[0].version_request = abi.dep_edges[0].version_request.wrapping_add(1);
        assert!(validate_abi_index(
            &abi,
            &package.exports,
            &package.project.imports,
            &package.project.strings.values,
            &package.project.types,
            &package.project.constants,
            &package.project.functions,
        )
        .is_err());

        // Used-symbol count disagreement.
        let mut abi2 = package.project.abi.clone();
        abi2.dep_edges[0].used_symbols.push(AbiUsedSymbol {
            name: 0,
            sig_hash: [0; ABI_HASH_LEN],
        });
        assert!(validate_abi_index(
            &abi2,
            &package.exports,
            &package.project.imports,
            &package.project.strings.values,
            &package.project.types,
            &package.project.constants,
            &package.project.functions,
        )
        .is_err());
    }

    #[test]
    fn validate_abi_index_rejects_edge_set_mismatch() {
        let mut consumer = empty_project("app");
        consumer.functions = vec![fn_named("run", "export", "function", "Integer")];
        let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
        metadata.dependencies = vec![BinaryReprDependency {
            name: "dep".to_string(),
            ident: String::new(),
            version: "1".to_string(),
            pin: false,
            flags: 0,
        }];
        let lowered = lower_project(&consumer, &metadata).expect("lower");
        let package = read_binary_repr_package(&lowered.encode()).expect("decode");
        let mut abi = package.project.abi.clone();
        abi.dep_edges.clear(); // edges no longer match the IMPORT_TABLE set
        assert!(validate_abi_index(
            &abi,
            &package.exports,
            &package.project.imports,
            &package.project.strings.values,
            &package.project.types,
            &package.project.constants,
            &package.project.functions,
        )
        .is_err());
    }

    #[test]
    fn package_type_exports_errors_when_type_missing_from_table() {
        // An ABI export naming a type absent from the type table is rejected.
        let project = rich_project();
        let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
        let mut package =
            read_binary_repr_package(&build_binary_repr_bytes(&project, &metadata).unwrap())
                .expect("decode");
        // Drop the type entries so exported Point/Shape/Color can't be resolved.
        package.project.types.entries.clear();
        package.project.types.ids.clear();
        assert!(package_type_exports(&package).is_err());
    }

    #[test]
    fn read_binary_repr_package_rejects_duplicate_section() {
        // Hand-forge an MFPC whose section table lists the same id twice.
        let mut sections = vec![Section::new(SECTION_STRING_POOL, {
            let mut b = Vec::new();
            put_u32(&mut b, 0);
            b
        })];
        sections.push(Section::new(SECTION_STRING_POOL, {
            let mut b = Vec::new();
            put_u32(&mut b, 0);
            b
        }));
        let bytes = encode_sections(&sections);
        match read_binary_repr_package(&bytes) {
            Ok(_) => panic!("expected duplicate-section error"),
            Err(err) => assert!(err.contains("duplicate"), "got: {err}"),
        }
    }

    #[test]
    fn read_binary_repr_package_reports_missing_required_sections() {
        // A well-formed MFPC that carries only the string pool is missing the
        // type/const/function/manifest/import/abi/export sections.
        let bytes = encode_sections(&[Section::new(SECTION_STRING_POOL, {
            let mut b = Vec::new();
            put_u32(&mut b, 0);
            b
        })]);
        assert!(read_binary_repr_package(&bytes).is_err());
    }
}

// ---------------------------------------------------------------------------
// mod.rs — error-formatting closures on the public read/write entry points.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod mod_error_path_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn read_package_ir_with_identity_reports_bad_container() {
        // A file that is not a valid .mfp container trips the payload error map.
        let path = temp_mfp(b"not an mfp file at all, definitely too short");
        assert!(read_package_ir_with_identity(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_package_ir_with_identity_reports_bad_inner_payload() {
        // Valid container header wrapping garbage MFPC bytes.
        let path = temp_mfp(&wrap_mfp(b"MFPCnope", "pkg", "pkg", "1"));
        assert!(read_package_ir_with_identity(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_package_ir_with_identity_reports_identity_mismatch() {
        let inner = encode_project(
            &rich_project(),
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        // Header claims a name the manifest does not carry.
        let path = temp_mfp(&wrap_mfp(&inner, "WRONG", "WRONG", "1.0.0"));
        assert!(read_package_ir_with_identity(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_binary_repr_hex_reports_unwritable_directory() {
        // A directory that does not exist makes fs::write fail.
        let dir = std::path::Path::new("/nonexistent-dir-xyz/deeper");
        let result = write_binary_repr_hex(dir, &rich_project(), "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn read_package_helpers_report_inner_decode_errors() {
        // A container that parses at the container layer but whose inner MFPC is
        // truncated trips each public helper's error-formatting closure.
        let inner = encode_project(
            &rich_project(),
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        // Corrupt the inner MFPC major version so read_binary_repr_package fails.
        let mut broken = inner.clone();
        broken[4] = 0xFF;
        broken[5] = 0xFF;
        let path = temp_mfp(&wrap_mfp(&broken, "richpkg", "richpkg", "1.0.0"));
        assert!(read_package_exports(&path).is_err());
        assert!(read_package_info(&path).is_err());
        assert!(read_package_type_exports(&path).is_err());
        assert!(read_package_resources(&path).is_err());
        assert!(read_package_docs(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}

// ---------------------------------------------------------------------------
// reader.rs — remaining decode error branches and composite ABI serialization.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod reader_gap_tests {
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
        assert_eq!(primitive_type_name(TYPE_TERM_COLOR), Some("TermColor"));
        assert_eq!(primitive_type_name(TYPE_TERM_SIZE), Some("TermSize"));
        assert_eq!(primitive_type_name(TYPE_SOCKET_HANDLE), Some("Socket"));
        assert_eq!(primitive_type_name(TYPE_LISTENER_HANDLE), Some("Listener"));
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
        types.type_id(&mut strings, "ISOLATED FUNC(Integer, String) AS Boolean");
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
        validate_abi_index(&good, &[], &imports, &strings.values, &types, &constants, &[])
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
        assert!(err.contains("type export `Point` sigHash disagrees"), "{err}");

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

    #[test]
    fn abi_serializer_rejects_reserved_type_ids_without_overflow() {
        // Ids 0 and 9 are neither primitives nor table ids (>= 10). A tampered
        // package can carry them; the serializer must report them cleanly on
        // every profile rather than underflowing `id - FIRST_TABLE_TYPE_ID`.
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
        for id in [0u32, 9] {
            let mut serializer = AbiSerializer::new(&strings, &types, &constants);
            let err = serializer
                .serialize_type(id)
                .expect_err("reserved type id must not serialize");
            assert_eq!(err, format!("unknown type id {id}"));
        }
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
            type_: ty.to_string(),
            loc: loc(),
        };
        project.types = vec![IrType {
            kind: "type".to_string(),
            visibility: "export".to_string(),
            name: "Bag".to_string(),
            fields: vec![
                field("l", "List OF Integer"),
                field("m", "Map OF String TO Integer"),
                field("r", "Result OF Integer"),
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
        // code buffer is empty, so code_end > code.len() -> truncated code error.
        assert!(read_function_table(&bytes, &[], &strings, &empty).is_err());
    }
}

// ---------------------------------------------------------------------------
// mod.rs — post-decode inner-IR error on read_package_ir_with_identity.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod mod_inner_ir_error_tests {
    use super::fixtures::*;
    use super::*;

    /// Rebuild the inner MFPC of `rich_project` with a corrupt BINARY_REPR
    /// section: the container + all metadata sections stay valid (so the package
    /// and its identity decode) but `decode_binary_repr` on the payload fails.
    fn container_with_corrupt_binary_repr() -> Vec<u8> {
        let inner = encode_project(
            &rich_project(),
            &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
        );
        let package = read_binary_repr_package(&inner).expect("decode");
        let proj = &package.project;
        let sections = vec![
            Section::new(SECTION_MANIFEST, proj.encode_manifest()),
            Section::new(SECTION_STRING_POOL, proj.strings.encode()),
            Section::new(SECTION_TYPE_TABLE, proj.types.encode()),
            Section::new(SECTION_CONST_POOL, proj.constants.encode()),
            Section::new(SECTION_IMPORT_TABLE, proj.imports.encode()),
            Section::new(SECTION_EXPORT_TABLE, proj.encode_exports()),
            Section::new(SECTION_GLOBAL_TABLE, proj.encode_globals()),
            Section::new(
                SECTION_FUNCTION_TABLE,
                proj.encode_functions(&vec![(0u64, 0u64); proj.functions.len()]),
            ),
            // Garbage payload: not a valid Binary Representation blob.
            Section::new(SECTION_BINARY_REPR, b"not-a-binary-repr".to_vec()),
            Section::new(SECTION_ABI_INDEX, proj.abi.encode()),
        ];
        encode_sections(&sections)
    }

    #[test]
    fn read_package_ir_with_identity_reports_inner_ir_decode_failure() {
        let inner = container_with_corrupt_binary_repr();
        // The package + identity decode, but the IR payload does not.
        assert!(read_binary_repr_package(&inner).is_ok());
        let path = temp_mfp(&wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0"));
        let err = read_package_ir_with_identity(&path);
        assert!(err.is_err());
        let _ = std::fs::remove_file(&path);
    }
}
