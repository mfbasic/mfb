use super::*;

pub(crate) fn supported_helper_specs() -> &'static [RuntimeHelperSpec] {
    &[
        AUDIO_DEVICES_SPEC,
        AUDIO_OPEN_INPUT_SPEC,
        AUDIO_OPEN_INPUT_DEVICE_SPEC,
        AUDIO_OPEN_OUTPUT_SPEC,
        AUDIO_OPEN_OUTPUT_DEVICE_SPEC,
        AUDIO_READ_SPEC,
        AUDIO_READ_TIMEOUT_SPEC,
        AUDIO_WRITE_SPEC,
        AUDIO_POLL_SPEC,
        AUDIO_POLL_TIMEOUT_SPEC,
        AUDIO_AVAILABLE_SPEC,
        AUDIO_XRUNS_SPEC,
        AUDIO_CLOSE_INPUT_SPEC,
        AUDIO_CLOSE_OUTPUT_SPEC,
        CRYPTO_RANDOM_BYTES_SPEC,
        CRYPTO_GENERATE_P256_RAW_SPEC,
        CRYPTO_GENERATE_P384_RAW_SPEC,
        CRYPTO_GENERATE_P521_RAW_SPEC,
        CRYPTO_P256_SIGN_SPEC,
        CRYPTO_P384_SIGN_SPEC,
        CRYPTO_P521_SIGN_SPEC,
        CRYPTO_P256_VERIFY_SPEC,
        CRYPTO_P384_VERIFY_SPEC,
        CRYPTO_P521_VERIFY_SPEC,
        DATETIME_NOW_NANOS_SPEC,
        DATETIME_MONOTONIC_NANOS_SPEC,
        DATETIME_LOCAL_OFFSET_SPEC,
        IO_PRINT_SPEC,
        IO_WRITE_SPEC,
        IO_PRINT_ERROR_SPEC,
        IO_WRITE_ERROR_SPEC,
        IO_FLUSH_SPEC,
        IO_IS_BUFFERED_SPEC,
        IO_SET_BUFFERED_SPEC,
        IO_INPUT_SPEC,
        IO_READ_LINE_SPEC,
        IO_READ_CHAR_SPEC,
        IO_READ_BYTE_SPEC,
        IO_POLL_INPUT_SPEC,
        IO_IS_INPUT_TERMINAL_SPEC,
        IO_IS_OUTPUT_TERMINAL_SPEC,
        IO_IS_ERROR_TERMINAL_SPEC,
        TERM_ON_SPEC,
        TERM_OFF_SPEC,
        TERM_IS_ON_SPEC,
        TERM_SET_FOREGROUND_SPEC,
        TERM_SET_BACKGROUND_SPEC,
        TERM_SET_BOLD_SPEC,
        TERM_SET_UNDERLINE_SPEC,
        TERM_SHOW_CURSOR_SPEC,
        TERM_HIDE_CURSOR_SPEC,
        TERM_CLEAR_SPEC,
        TERM_SYNC_SPEC,
        TERM_MOVE_TO_SPEC,
        TERM_GET_FOREGROUND_SPEC,
        TERM_GET_BACKGROUND_SPEC,
        TERM_GET_BOLD_SPEC,
        TERM_GET_UNDERLINE_SPEC,
        TERM_TERMINAL_SIZE_SPEC,
        FS_FILE_EXISTS_SPEC,
        FS_DIRECTORY_EXISTS_SPEC,
        FS_EXISTS_SPEC,
        FS_CURRENT_DIRECTORY_SPEC,
        FS_TEMP_DIRECTORY_SPEC,
        FS_SET_CURRENT_DIRECTORY_SPEC,
        FS_DELETE_FILE_SPEC,
        FS_CREATE_DIRECTORY_SPEC,
        FS_CREATE_DIRECTORIES_SPEC,
        FS_DELETE_DIRECTORY_SPEC,
        FS_LIST_DIRECTORY_SPEC,
        FS_OPEN_SPEC,
        FS_OPEN_FILE_SPEC,
        FS_OPEN_FILE_NO_FOLLOW_SPEC,
        FS_OPEN_WITHIN_SPEC,
        FS_CREATE_TEMP_FILE_SPEC,
        FS_CLOSE_SPEC,
        FS_SET_BUFFERED_SPEC,
        FS_IS_BUFFERED_SPEC,
        FS_FLUSH_SPEC,
        FS_WRITE_ALL_SPEC,
        FS_READ_TEXT_SPEC,
        FS_READ_BYTES_SPEC,
        FS_WRITE_TEXT_SPEC,
        FS_WRITE_TEXT_ATOMIC_SPEC,
        FS_WRITE_BYTES_SPEC,
        FS_WRITE_BYTES_ATOMIC_SPEC,
        FS_APPEND_TEXT_SPEC,
        FS_APPEND_BYTES_SPEC,
        FS_READ_LINE_SPEC,
        FS_READ_ALL_SPEC,
        FS_READ_ALL_BYTES_SPEC,
        FS_WRITE_ALL_BYTES_SPEC,
        FS_EOF_SPEC,
        FS_CANONICAL_PATH_SPEC,
        FS_IS_WITHIN_SPEC,
        OS_GET_ENV_SPEC,
        OS_GET_ENV_OR_SPEC,
        OS_HAS_ENV_SPEC,
        OS_SET_ENV_SPEC,
        OS_UNSET_ENV_SPEC,
        OS_ENVIRON_SPEC,
        OS_ARGS_SPEC,
        OS_PID_SPEC,
        OS_EXECUTABLE_PATH_SPEC,
        OS_RESOURCE_PATH_SPEC,
        OS_NAME_SPEC,
        OS_ARCH_SPEC,
        OS_HOST_NAME_SPEC,
        OS_USER_NAME_SPEC,
        OS_CPU_COUNT_SPEC,
        // No `strings::` row: those ops are all native-direct (lowered inline; no
        // `_mfb_rt_strings_*` helper is ever emitted, bug-120.1). The dead spec
        // table that used to sit beside this comment is gone (bug-326-A1).
        THREAD_START_SPEC,
        THREAD_IS_RUNNING_SPEC,
        THREAD_WAIT_FOR_SPEC,
        THREAD_CANCEL_SPEC,
        THREAD_DROP_SPEC,
        THREAD_SEND_SPEC,
        THREAD_POLL_SPEC,
        THREAD_READ_SPEC,
        THREAD_RECEIVE_SPEC,
        THREAD_EMIT_SPEC,
        THREAD_TRANSFER_SPEC,
        THREAD_ACCEPT_SPEC,
        THREAD_EMIT_RESOURCE_SPEC,
        THREAD_READ_RESOURCE_SPEC,
        THREAD_IS_CANCELLED_SPEC,
        THREAD_OPEN_STD_IN_SPEC,
        THREAD_CLOSE_STD_IN_SPEC,
        NET_LOOKUP_SPEC,
        NET_CONNECT_TCP_SPEC,
        NET_CONNECT_TCP_ADDR_SPEC,
        NET_LISTEN_TCP_SPEC,
        NET_ACCEPT_SPEC,
        NET_POLL_SPEC,
        NET_READ_SPEC,
        NET_READ_TEXT_SPEC,
        NET_WRITE_SPEC,
        NET_WRITE_TEXT_SPEC,
        NET_CLOSE_SPEC,
        NET_LOCAL_ADDRESS_SPEC,
        NET_REMOTE_ADDRESS_SPEC,
        NET_SET_READ_TIMEOUT_SPEC,
        NET_SET_WRITE_TIMEOUT_SPEC,
        NET_BIND_UDP_SPEC,
        NET_RECEIVE_FROM_SPEC,
        NET_RECEIVE_TEXT_FROM_SPEC,
        NET_SEND_TO_SPEC,
        NET_SEND_TEXT_TO_SPEC,
        TLS_CONNECT_SPEC,
        TLS_LISTEN_SPEC,
        TLS_ACCEPT_SPEC,
        TLS_READ_SPEC,
        TLS_READ_TEXT_SPEC,
        TLS_WRITE_SPEC,
        TLS_WRITE_TEXT_SPEC,
        TLS_CLOSE_SPEC,
        TLS_CLOSE_LISTENER_SPEC,
    ]
}

pub(crate) fn spec_for_symbol(symbol: &str) -> Option<&'static RuntimeHelperSpec> {
    supported_helper_specs()
        .iter()
        .find(|spec| spec.symbol == symbol)
}

pub(crate) fn spec_for_call(target: &str) -> Option<&'static RuntimeHelperSpec> {
    supported_helper_specs()
        .iter()
        .find(|spec| spec.call == target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::runtime::{helper_for_call, symbol_for_call};
    use std::collections::HashSet;

    // bug-329: every catalogued symbol is exactly what `symbol_for_call`
    // produces — the transcribed `symbol` field is derivable, not independent.
    // This is the gate that makes deleting the field safe; if a future spec
    // ever needs a non-derivable symbol, this test is what surfaces it.
    #[test]
    fn every_spec_symbol_is_derivable() {
        for spec in supported_helper_specs() {
            assert_eq!(
                spec.symbol,
                symbol_for_call(spec.helper, spec.call),
                "{} symbol must equal symbol_for_call",
                spec.call
            );
        }
    }

    // One table-driven parity test over the catalog itself (bug-329), replacing
    // the hand-copied per-family call arrays that used to live in
    // audio_specs.rs/os_specs.rs: a new spec is covered the moment it is added,
    // because there is no second list to maintain.
    #[test]
    fn catalog_is_consistent() {
        let specs = supported_helper_specs();
        let mut seen_symbols = HashSet::new();
        let mut families = HashSet::new();
        // Catalogued calls that `helper_for_call` must NOT classify: these are
        // synthesized inside the code layer (`builder_values` rewrites the
        // user-facing call into the direction/overload-specific queue or addr
        // variant; `thread.drop` is the handle-cleanup helper emitted by
        // codegen primitives), so they never exist at the NIR level where
        // `helper_for_call` routes calls. They are catalogued only so
        // `spec_for_call`/`spec_for_symbol` resolve them during code emission
        // and object planning.
        const CODE_LAYER_ONLY_CALLS: &[&str] = &[
            "thread.drop",
            "thread.read",
            "thread.emit",
            "net.connectTcpAddr",
        ];
        // Family round-trip: the front end routes each call to its helper
        // (except the code-layer-synthesized calls, which must stay invisible
        // to the NIR-level classifier). Collected so one failure reports the
        // whole set.
        let misrouted: Vec<String> = specs
            .iter()
            .filter_map(|spec| {
                let expected = if CODE_LAYER_ONLY_CALLS.contains(&spec.call) {
                    None
                } else {
                    Some(spec.helper)
                };
                let actual = helper_for_call(spec.call);
                (actual != expected)
                    .then(|| format!("{}: {:?} (expected {:?})", spec.call, actual, expected))
            })
            .collect();
        assert!(misrouted.is_empty(), "misrouted calls: {misrouted:#?}");
        for spec in specs {
            // Call round-trip (also proves call strings are unique: a duplicate
            // would resolve to the first entry and fail here for the second).
            assert!(
                std::ptr::eq(spec_for_call(spec.call).unwrap(), spec),
                "spec_for_call {}",
                spec.call
            );
            // Symbol round-trip + uniqueness.
            let symbol = symbol_for_call(spec.helper, spec.call);
            assert!(
                std::ptr::eq(spec_for_symbol(&symbol).unwrap(), spec),
                "spec_for_symbol {symbol}"
            );
            assert!(
                seen_symbols.insert(symbol),
                "duplicate symbol for {}",
                spec.call
            );
            // `returns` is the load-bearing abi field; every code-plan consumer
            // reads it.
            assert!(!spec.abi.returns.is_empty(), "{} returns", spec.call);
            families.insert(spec.helper);
        }
        // Every RuntimeHelper family is catalogued except General and Math,
        // which are fully native-direct (lowered inline; no `_mfb_rt_*` helper
        // is ever emitted for them). A variant missing here with no catalogued
        // spec is the dead-catalog situation bug-326 removed for `strings`.
        for helper in [
            RuntimeHelper::Audio,
            RuntimeHelper::Crypto,
            RuntimeHelper::Datetime,
            RuntimeHelper::Fs,
            RuntimeHelper::Io,
            RuntimeHelper::Net,
            RuntimeHelper::Os,
            RuntimeHelper::Term,
            RuntimeHelper::Thread,
            RuntimeHelper::Tls,
        ] {
            assert!(
                families.contains(&helper),
                "family {} has no catalogued spec",
                helper.name()
            );
        }
        assert_eq!(families.len(), 10, "unexpected extra catalogued family");
    }
}
