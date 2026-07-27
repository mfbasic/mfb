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
    // `stringify` only fails on a non-finite `Number`; a `String` value is
    // always representable, so this cannot return `Err` (plan-68-A Open Decision:
    // the former `.unwrap_or_else` fallback was dead — unreachable from any input).
    JsonValue::String(value.to_string())
        .stringify()
        .expect("JsonValue::String is always stringifiable")
}

/// A node that renders itself as a JSON fragment at a given indent depth. Shared
/// by the `-ast` and `-ir` dumpers, which were each carrying a byte-identical
/// copy of this trait and the `join_json` helper below under their own names
/// (`ToAstJson` + `join_indented`, `ToIrJson` + `join_json`).
pub(crate) trait ToJson {
    fn to_json(&self, indent: usize) -> String;
}

/// Render each of `items` and join the fragments with a comma — the array-body
/// helper both emitters use.
pub(crate) fn join_json<T: ToJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}
