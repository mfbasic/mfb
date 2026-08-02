//! Shared JSON emission helpers used by the artifact dumpers (`-ast`, `-ir`,
//! `-nir`, `-nplan`, …), the manifest tooling, and the object writers.
//!
//! These live at the crate root rather than inside any one emitter because the
//! escaper alone has ~15 consumers across `ast`, `ir`, `target`, `os`,
//! `manifest`, `cli`, and `coverage`; keeping it in the binary entrypoint forced
//! every one of them to reach into `main.rs` for a string utility.

use tinyjson::JsonValue;

/// The maximum JSON nesting depth accepted by [`parse_json_bounded`].
///
/// `tinyjson` is a recursive-descent parser with no depth limit of its own, so a
/// deeply nested *untrusted* document — `project.json`, `mfb.lock`, or a
/// dependency's package manifest — would recurse once per nesting level and
/// overflow the native thread stack, aborting the process (SIGABRT) *before* any
/// schema validation ran (bug-398). We pre-scan the input's bracket depth
/// iteratively and reject anything past this cap, which no legitimate manifest or
/// lockfile approaches. 256 matches the front-end `MAX_EXPR_DEPTH` /
/// `MAX_IR_NESTING_DEPTH` ceilings.
pub(crate) const MAX_JSON_NESTING_DEPTH: usize = 256;

/// Parse `source` as JSON, rejecting input nested deeper than
/// [`MAX_JSON_NESTING_DEPTH`] with a bounded error instead of letting `tinyjson`
/// recurse off the stack (bug-398).
///
/// The depth guard is an iterative byte scan that counts only the `[`/`{` … `]`/`}`
/// structure *outside* string literals; it runs before `tinyjson` sees the input,
/// so no depth of nesting can drive the recursive parser into a stack overflow.
/// Every compiler-side decode of untrusted JSON must route through here rather
/// than calling `str::parse::<JsonValue>()` directly.
pub(crate) fn parse_json_bounded(source: &str) -> Result<JsonValue, String> {
    check_json_depth(source)?;
    source
        .parse::<JsonValue>()
        .map_err(|err: tinyjson::JsonParseError| err.to_string())
}

/// Reject `source` if its bracket nesting exceeds [`MAX_JSON_NESTING_DEPTH`].
///
/// This is a structural pre-scan, not a JSON validator: it counts array/object
/// openers outside of string literals (honouring `\"` and `\\` escapes) and bails
/// the instant the running depth passes the cap. `tinyjson`'s own recursion is
/// driven by exactly those openers, so bounding them bounds the recursion; any
/// other malformity is left for `tinyjson` to diagnose.
///
/// Exposed for the one decode site (`manifest::load` project.json validation)
/// that wants to keep `tinyjson`'s positional parse error for ordinary malformed
/// input and so guards depth separately rather than through [`parse_json_bounded`].
pub(crate) fn check_json_depth(source: &str) -> Result<(), String> {
    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in source.as_bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_JSON_NESTING_DEPTH {
                    return Err(format!(
                        "JSON nested too deeply (maximum {MAX_JSON_NESTING_DEPTH} levels)"
                    ));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_parse_accepts_a_normal_object() {
        let value = parse_json_bounded(r#"{"name": "app", "deps": [1, 2, 3]}"#)
            .expect("well-formed manifest parses");
        assert!(value
            .get::<std::collections::HashMap<String, JsonValue>>()
            .is_some());
    }

    #[test]
    fn bounded_parse_accepts_nesting_up_to_the_cap() {
        // A document nested exactly at the cap must still parse — the guard must
        // not reject any legitimate manifest.
        let json = "[".repeat(MAX_JSON_NESTING_DEPTH) + &"]".repeat(MAX_JSON_NESTING_DEPTH);
        assert!(parse_json_bounded(&json).is_ok());
    }

    #[test]
    fn bounded_parse_rejects_nesting_past_the_cap() {
        let json = "[".repeat(MAX_JSON_NESTING_DEPTH + 1) + &"]".repeat(MAX_JSON_NESTING_DEPTH + 1);
        let err = parse_json_bounded(&json).expect_err("over-cap nesting is rejected");
        assert!(err.contains("nested too deeply"), "unexpected error: {err}");
    }

    #[test]
    fn bounded_parse_rejects_overflow_depth_without_crashing() {
        // bug-398: the exact shape that used to abort the process (SIGABRT via
        // stack overflow) must now return a bounded error, iteratively, with no
        // recursion into tinyjson.
        let json = "[".repeat(120_000) + &"]".repeat(120_000);
        let err = parse_json_bounded(&json).expect_err("overflow-depth input is rejected");
        assert!(err.contains("nested too deeply"), "unexpected error: {err}");
    }

    #[test]
    fn depth_scan_ignores_brackets_inside_strings() {
        // Brackets and braces inside a string literal are data, not structure,
        // and must not count toward the nesting depth.
        let payload = format!(r#"{{"k": "{}"}}"#, "[{".repeat(10_000));
        assert!(parse_json_bounded(&payload).is_ok());
    }

    #[test]
    fn depth_scan_honours_escaped_quote_in_a_string() {
        // An escaped quote does not close the string, so the following `[` stays
        // inside the literal and is not counted.
        let payload = r#"{"k": "a\"[[[[b"}"#;
        assert!(parse_json_bounded(payload).is_ok());
    }
}
