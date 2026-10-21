use super::*;
use std::collections::HashMap;
use tinyjson::JsonValue;

fn object(text: &str) -> Vec<LosslessMember> {
    match parse_lossless(text).expect("parses") {
        LosslessValue::Object(members) => members,
        other => panic!("expected an object, got {other:?}"),
    }
}

#[test]
fn number_lexemes_are_kept_exactly() {
    for lexeme in [
        "3",
        "3.0",
        "-0",
        "1e5",
        "2E-3",
        "-12",
        "1.25e+10",
        "9223372036854775808",
    ] {
        assert_eq!(
            parse_lossless(lexeme),
            Ok(LosslessValue::Number(lexeme.to_string())),
            "{lexeme}"
        );
    }
}

#[test]
fn duplicate_keys_are_kept_in_order() {
    let members = object(r#"{"a": 1, "b": 2, "a": 3}"#);
    let keys: Vec<_> = members.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, ["a", "b", "a"]);
    assert_eq!(members[2].value, LosslessValue::Number("3".to_string()));
}

#[test]
fn member_lines_are_the_key_lines() {
    let text = "{\n  \"a\": 1,\n\n  \"b\": {\n    \"c\": \"x\"\n  },\n  \"d\":\n    2\n}";
    let members = object(text);
    assert_eq!(
        members.iter().map(|m| m.line).collect::<Vec<_>>(),
        [2, 4, 7]
    );
    let LosslessValue::Object(inner) = &members[1].value else {
        panic!("b is an object");
    };
    assert_eq!(inner[0].line, 5);
}

#[test]
fn escapes_and_surrogate_pairs_are_decoded() {
    assert_eq!(
        parse_lossless(r#""q\" b\\ s\/ \b\f\n\r\t \u00e9 \ud83d\ude00""#),
        Ok(LosslessValue::String(
            "q\" b\\ s/ \u{8}\u{c}\n\r\t é 😀".to_string()
        ))
    );
    // Non-ASCII text passes through unescaped.
    assert_eq!(
        parse_lossless("\"héllo ✓\""),
        Ok(LosslessValue::String("héllo ✓".to_string()))
    );
}

#[test]
fn lone_surrogates_are_rejected() {
    for text in [
        r#""\ud83d""#,
        r#""\ud83dx""#,
        r#""\ude00""#,
        r#""\ud83d\u0041""#,
    ] {
        assert!(parse_lossless(text).is_err(), "{text}");
    }
}

#[test]
fn non_rfc_8259_input_is_rejected() {
    for text in [
        r#"{"a": 1,}"#,
        "[1, 2,]",
        "{\"a\": 1 // c\n}",
        "/* c */ {}",
        "01",
        "+1",
        ".5",
        "1.",
        "1e",
        "-",
        "{'a': 1}",
        "\"tab\there\"",
        "\"a\nb\"",
        "\"\\x\"",
        "{} {}",
        "tru",
        "",
        "{\"a\" 1}",
        "[1 2]",
    ] {
        assert!(parse_lossless(text).is_err(), "should reject {text:?}");
    }
}

#[test]
fn errors_carry_their_line() {
    let err = parse_lossless("{\n  \"a\": 1,\n  \"b\": 01\n}").expect_err("leading zero");
    assert_eq!(err.line, 3);
}

#[test]
fn nesting_is_capped_like_check_json_depth() {
    let at_cap = "[".repeat(MAX_JSON_NESTING_DEPTH) + &"]".repeat(MAX_JSON_NESTING_DEPTH);
    assert!(parse_lossless(&at_cap).is_ok());
    let past = "[".repeat(MAX_JSON_NESTING_DEPTH + 1) + &"]".repeat(MAX_JSON_NESTING_DEPTH + 1);
    let err = parse_lossless(&past).expect_err("past the cap");
    assert!(err.message.contains("nested too deeply"), "{}", err.message);
    // A huge depth fails at the cap, never by overflowing the stack.
    let huge = "[".repeat(120_000);
    assert!(parse_lossless(&huge).is_err());
}

/// The test-only bridge to `tinyjson`'s model: numbers become `f64` and a
/// repeated key keeps its last value, which is what `tinyjson` does.
fn to_tinyjson(value: &LosslessValue) -> JsonValue {
    match value {
        LosslessValue::Null => JsonValue::Null,
        LosslessValue::Bool(b) => JsonValue::Boolean(*b),
        LosslessValue::Number(lexeme) => {
            JsonValue::Number(lexeme.parse::<f64>().expect("a JSON number lexeme"))
        }
        LosslessValue::String(s) => JsonValue::String(s.clone()),
        LosslessValue::Array(items) => JsonValue::Array(items.iter().map(to_tinyjson).collect()),
        LosslessValue::Object(members) => {
            let mut map = HashMap::new();
            for member in members {
                map.insert(member.key.clone(), to_tinyjson(&member.value));
            }
            JsonValue::Object(map)
        }
    }
}

fn project_json_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            project_json_files(&path, out);
        } else if path.file_name().is_some_and(|name| name == "project.json") {
            out.push(path);
        }
    }
}

#[test]
fn agrees_with_tinyjson_on_every_project_json_under_tests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    project_json_files(&root, &mut files);
    assert!(files.len() > 1000, "found only {} manifests", files.len());
    let mut parsed = 0;
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let tiny = text.parse::<JsonValue>();
        let lossless = parse_lossless(&text);
        match (tiny, lossless) {
            (Ok(tiny), Ok(lossless)) => {
                assert_eq!(to_tinyjson(&lossless), tiny, "{}", path.display());
                parsed += 1;
            }
            (Err(_), Err(_)) => {}
            (tiny, lossless) => panic!(
                "{}: tinyjson ok={} lossless ok={}",
                path.display(),
                tiny.is_ok(),
                lossless.is_ok()
            ),
        }
    }
    assert!(parsed > 1000, "parsed only {parsed}");
}

#[test]
fn a_short_unicode_escape_or_an_unterminated_string_is_refused() {
    let message = |text: &str| parse_lossless(text).expect_err("refused").message;
    assert_eq!(
        message("\"\\u12\""),
        "expected four hex digits after `\\u`"
    );
    assert_eq!(message("\"abc"), "unterminated string");
    // A backslash with nothing after it.
    assert_eq!(message("\"abc\\"), "unterminated string");
}
