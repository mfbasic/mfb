//! plan-176-U: the standard and native-link error message tables. The builder types and shared
//! imports live in `engine/builder/mod.rs`.

pub(crate) fn native_link_error_messages() -> Vec<(&'static str, &'static str, &'static str)> {
    // plan-88-D: the `(code, message, symbol)` triples are sourced from
    // `ERRORCODE_CONSTANTS` by name (order preserved: it drives data-object layout).
    // Boundary validations (plan-linker.md §12.3/§12.4) are the last four.
    [
        "ErrNativeBindingUnavailable",
        "ErrNativeBindingCallFailed",
        "ErrNativeBufferOverrun",
        "ErrOutOfMemory",
        "ErrOverflow",
        "ErrEncoding",
        "ErrFloatNaN",
        "ErrFloatInf",
    ]
    .into_iter()
    .map(|name| {
        crate::codegen::registry::runtime_error_triple(name)
            .expect("native-link error name is an errorCode constant")
    })
    .collect()
}

pub(crate) fn standard_error_messages() -> Vec<(&'static str, &'static str, &'static str)> {
    // plan-88-D: `(code, message, symbol)` triples sourced from `ERRORCODE_CONSTANTS`
    // by name; order preserved (it drives the emitted data-object layout).
    [
        "ErrInvalidArgument",
        "ErrInvalidFormat",
        "ErrOverflow",
        "ErrUnderflow",
        "ErrFloatDomain",
        "ErrFloatNaN",
        "ErrFloatInf",
        "ErrFloatOverflow",
        "ErrOutOfMemory",
        "ErrIndexOutOfRange",
        "ErrNotFound",
        "ErrTimeout",
        "ErrInterrupted",
        "ErrUnknown",
        "ErrReadFailed",
        "ErrAlreadyExists",
        "ErrAccessDenied",
        "ErrResourceBusy",
        "ErrCloseFailed",
        "ErrPathNotFound",
        "ErrInvalidPath",
        "ErrResourceClosed",
        "ErrResourceMoved",
        "ErrUnsupported",
        "ErrWriteFailed",
        "ErrEndOfFile",
        "ErrEncoding",
        "ErrInputFailed",
        "ErrInvalidContext",
        "ErrWrongMode",
        "ErrAddressInvalid",
        "ErrAddressNotFound",
        "ErrNetworkFailed",
        "ErrConnectionClosed",
        "ErrMessageTooLarge",
        "ErrTlsFailed",
        "ErrAudioUnavailable",
        "ErrAudioDevice",
        "ErrSpawnFailed",
    ]
    .into_iter()
    .map(|name| {
        crate::codegen::registry::runtime_error_triple(name)
            .expect("standard error name is an errorCode constant")
    })
    .collect()
}

pub(crate) fn standard_error_message_symbol(message: &str) -> Option<&'static str> {
    standard_error_messages()
        .iter()
        .find_map(|(_, candidate, symbol)| (*candidate == message).then_some(*symbol))
}
