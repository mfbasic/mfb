use super::*;

#[test]
fn standard_flags_set_sendable_bit_for_movable_resources() {
    let file = standard_resource_flags(crate::codegen::builtins::fs::FILE_TYPE_ID);
    let socket = standard_resource_flags(builtins::net::SOCKET_TYPE_ID);
    let listener = standard_resource_flags(builtins::net::LISTENER_TYPE_ID);
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
                flags: standard_resource_flags(crate::codegen::builtins::fs::FILE_TYPE_ID),
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
