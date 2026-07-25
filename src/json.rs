//! Shared JSON emission helpers used by the artifact dumpers (`-ast`, `-ir`,
//! `-nir`, `-nplan`, …), the manifest tooling, and the object writers.
//!
//! These live at the crate root rather than inside any one emitter because the
//! escaper alone has ~15 consumers across `ast`, `ir`, `target`, `os`,
//! `manifest`, `cli`, and `coverage`; keeping it in the binary entrypoint forced
//! every one of them to reach into `main.rs` for a string utility.

use tinyjson::JsonValue;

/// Escape and quote `value` as a JSON string literal.
pub(crate) fn json_string(value: &str) -> String {
    JsonValue::String(value.to_string())
        .stringify()
        .unwrap_or_else(|_| "\"mfb_project\"".to_string())
}
