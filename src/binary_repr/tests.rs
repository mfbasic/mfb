use super::*;

#[cfg(test)]
mod doc_table_tests {
    use super::*;

    #[test]
    fn doc_table_round_trips() {
        let docs = PackageDocs {
            package: Some(PackageDocEntry {
                name: "mathx".to_string(),
                desc: vec![(0, "First paragraph.".to_string()), (0, "Second.".to_string())],
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
        assert_eq!(add.desc, vec![(0, "Adds.".to_string()), (1, "Overflows.".to_string())]);
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
