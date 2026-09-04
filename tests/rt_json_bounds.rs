//! `json::parse` scans bytes, not grapheme clusters (bug-510, audit-3 DEC-03/04).
//!
//! The parser tokenised its input with `strings::graphemes`, which has two costs.
//! DEC-04: a CR LF pair is one grapheme cluster, so a CRLF-formatted document —
//! the line ending every Windows editor writes — was rejected as "invalid JSON
//! format", because neither of JSON's two whitespace characters equals the
//! cluster made of both. DEC-03: the grapheme list is a one-character `String` per
//! character, ~42 bytes each, so a document's tokenisation alone cost forty times
//! its size before a single value was built.
//!
//! **The positive half is a pinned corpus.** `PARSE_CORPUS` runs sixty-three
//! documents — every scalar form, every escape, the surrogate and depth and
//! overflow errors, combining marks and astral characters inside strings and keys,
//! bare CR as whitespace — and its expected output was recorded before the fix.
//! Only the two CRLF documents, which the fix is meant to change, live outside it.

mod common;

use std::time::Duration;

fn run(name: &str, source: &str, timeout: Duration, hang_context: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(&binary, timeout, hang_context);
    assert!(
        status.success(),
        "{name}: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout.lines().map(str::to_string).collect()
}

#[test]
fn crlf_formatted_json_parses() {
    // DEC-04. `\r` and `\n` are each JSON whitespace, so `\r\n` between tokens is
    // whitespace twice over. A raw CR *inside a string* stays a control character
    // and stays rejected (the corpus pins that).
    let lines = run(
        "json_bounds_crlf",
        r#"IMPORT io
IMPORT json

FUNC one(doc AS String) AS String
  LET v AS json::Json = json::parse(doc)
  RETURN json::stringify(v)
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(one("\r\n{\"a\":1}\r\n"))
  io::print(one("{\"a\":\r\n1}"))
  io::print(one("[1,\r\n 2,\r\n 3]"))
END SUB
"#,
        Duration::from_secs(60),
        "parsing CRLF JSON did not finish",
    );
    assert_eq!(
        lines,
        vec!["{\"a\":1}", "{\"a\":1}", "[1,2,3]"],
        "CRLF-formatted JSON must parse like LF-formatted JSON",
    );
}

/// A document that is one three-megabyte string. Its tokenisation is the whole
/// cost: there is one value to build and no structure.
const LARGE_STRING: &str = r#"IMPORT io
IMPORT json

SUB main()
  MUT s AS String = "\""
  MUT i AS Integer = 0
  WHILE i < 3000000
    s = s & "x"
    i = i + 1
  END WHILE
  s = s & "\""
  LET v AS json::Json = json::parse(s)
  MATCH v
    CASE json::JsonStr(t)
      io::print("len=" & toString(len(t.value)))
    CASE ELSE
      io::print("not a string")
  END MATCH
END SUB
"#;

#[cfg(unix)]
#[test]
fn a_large_string_costs_a_bounded_multiple_of_its_size() {
    // DEC-03, the json share. Before the fix a 3 MB string cost a 126 MB grapheme
    // list plus a 126 MB list of one-character chunks (and their growth garbage)
    // — over 400 MB for 3 MB of text. Scanning bytes and accumulating bytes keeps
    // it within a small multiple; 96 MB is a generous ceiling and a fraction of
    // what the grapheme path spent.
    let project = common::temp_project("json_bounds_large_string", LARGE_STRING);
    let binary = common::build_project(&project);
    let (status, stdout, rss) = common::run_bounded_with_rss(
        &binary,
        Duration::from_secs(120),
        "parsing a 3 MB JSON string did not finish",
    );
    let _ = std::fs::remove_dir_all(&project);
    assert!(status.success(), "{}:\n{stdout}", common::exit_description(&status));
    assert_eq!(stdout.trim(), "len=3000000");
    let rss = rss.expect("unix reports ru_maxrss");
    assert!(
        rss < 96 * 1024 * 1024,
        "parsing a 3 MB JSON string peaked at {} MB of resident memory",
        rss / (1024 * 1024),
    );
}

/// Sixty-three documents through `json::parse` + `json::stringify`, or the raised
/// code. Generated once; the expected output was captured before bug-510's fix.
const PARSE_CORPUS: &str = r####"IMPORT io
IMPORT json

FUNC one(idx AS Integer, doc AS String) AS String
  LET v AS json::Json = json::parse(doc)
  RETURN toString(idx) & ": " & json::stringify(v)
  TRAP(e)
    RETURN toString(idx) & ": raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(one(0, "null"))
  io::print(one(1, "true"))
  io::print(one(2, "false"))
  io::print(one(3, "0"))
  io::print(one(4, "-0"))
  io::print(one(5, "1"))
  io::print(one(6, "-1"))
  io::print(one(7, "1.5"))
  io::print(one(8, "1e3"))
  io::print(one(9, "1E-2"))
  io::print(one(10, "-1.25e+2"))
  io::print(one(11, "123456789012"))
  io::print(one(12, "0.1"))
  io::print(one(13, "1e400"))
  io::print(one(14, "01"))
  io::print(one(15, "+1"))
  io::print(one(16, ".5"))
  io::print(one(17, "1."))
  io::print(one(18, "NaN"))
  io::print(one(19, "Infinity"))
  io::print(one(20, "\"\""))
  io::print(one(21, "\"a\""))
  io::print(one(22, "\"h\u{E9}llo\""))
  io::print(one(23, "\"\\u0041\""))
  io::print(one(24, "\"\\u00e9\""))
  io::print(one(25, "\"\\ud83d\\ude00\""))
  io::print(one(26, "\"\\ud83d\""))
  io::print(one(27, "\"\\ude00\""))
  io::print(one(28, "\"\\n\\t\\r\\b\\f\\/\\\\\\\"\""))
  io::print(one(29, "\"tab\there\""))
  io::print(one(30, "\"\\u004\""))
  io::print(one(31, "\"\\x41\""))
  io::print(one(32, "[]"))
  io::print(one(33, "[1]"))
  io::print(one(34, "[1,2,3]"))
  io::print(one(35, "[ 1 , 2 ]"))
  io::print(one(36, "[1,]"))
  io::print(one(37, "[,1]"))
  io::print(one(38, "[[[]]]"))
  io::print(one(39, "[1,[2,[3,[4]]]]"))
  io::print(one(40, "{}"))
  io::print(one(41, "{\"a\":1}"))
  io::print(one(42, "{\"a\":1,\"b\":[true,null]}"))
  io::print(one(43, "{\"a\":1,\"a\":2}"))
  io::print(one(44, "{\"a\":{\"b\":{\"c\":{}}}}"))
  io::print(one(45, "{\"a\" 1}"))
  io::print(one(46, "{a:1}"))
  io::print(one(47, "{\"a\":1,}"))
  io::print(one(48, " \t\n 1 \n\t "))
  io::print(one(49, "1 2"))
  io::print(one(50, "[1] x"))
  io::print(one(51, ""))
  io::print(one(52, "   "))
  io::print(one(53, "[1,\r2]"))
  io::print(one(54, "\"a\rb\""))
  io::print(one(55, "\"\u{E9}\u{301}\""))
  io::print(one(56, "\"e\u{301}\""))
  io::print(one(57, "[\"\u{1F600}\"]"))
  io::print(one(58, "{\"\u{1F600}\":1}"))
  io::print(one(59, "[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]"))
  io::print(one(60, "\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\""))
  io::print(one(61, "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]"))
  io::print(one(62, "{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":{\"k\":1}}}}}}}}}}}}}}}}}}}}}}}}}}}}}}"))
END SUB
"####;

const PARSE_CORPUS_EXPECTED: &str = r####"0: null
1: true
2: false
3: 0
4: 0
5: 1
6: -1
7: 1.5
8: 1000
9: 0.01
10: -125
11: 123456789012
12: 0.1
13: raised 77050010
14: raised 77050003
15: raised 77050003
16: raised 77050003
17: raised 77050003
18: raised 77050003
19: raised 77050003
20: ""
21: "a"
22: "héllo"
23: "A"
24: "é"
25: "😀"
26: raised 77050025
27: raised 77050025
28: "\n\t\r\b\f/\\\""
29: raised 77050003
30: raised 77050003
31: raised 77050003
32: []
33: [1]
34: [1,2,3]
35: [1,2]
36: raised 77050003
37: raised 77050003
38: [[[]]]
39: [1,[2,[3,[4]]]]
40: {}
41: {"a":1}
42: {"a":1,"b":[true,null]}
43: {"a":2}
44: {"a":{"b":{"c":{}}}}
45: raised 77050003
46: raised 77050003
47: raised 77050003
48: 1
49: raised 77050003
50: raised 77050003
51: raised 77050003
52: raised 77050003
53: [1,2]
54: raised 77050003
55: "é́"
56: "é"
57: ["😀"]
58: {"😀":1}
59: [1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]
60: "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
61: [[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]
62: {"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":{"k":1}}}}}}}}}}}}}}}}}}}}}}}}}}}}}}
"####;

#[test]
fn parsing_semantics_are_unchanged() {
    let lines = run(
        "json_bounds_corpus",
        PARSE_CORPUS,
        Duration::from_secs(120),
        "the parse corpus did not finish",
    );
    let got = lines.join("\n");
    let want = PARSE_CORPUS_EXPECTED.trim_end_matches('\n');
    if got != want {
        let first = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "the parse corpus changed at line {first}:\n  got:  {:?}\n  want: {:?}",
            got.lines().nth(first).unwrap_or(""),
            want.lines().nth(first).unwrap_or(""),
        );
    }
}
