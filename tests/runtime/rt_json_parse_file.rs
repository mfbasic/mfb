//! bug-815: `json::parse(RES value AS fs::File)` and the `(RES value AS
//! fs::File, reviver)` overload.
//!
//! Both read the file's remaining text with `fs::readAll` and hand it to the
//! existing `__json_parse`/`__json_parseRevive` bodies — there is still only
//! one parser. These tests pin that the File overloads are indistinguishable
//! from the String overloads for the same text (result, error code, and
//! reviver walk), over both a real on-disk file and an `fs::openData`
//! in-memory file, and that the File-specific contract holds: the file is not
//! closed, its position ends at end of input, a closed file raises
//! `ErrResourceClosed`, and a non-UTF-8 file raises `ErrEncoding`.

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

fn run(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_project(&project);
    let (status, stdout) =
        common::run_bounded(&binary, Duration::from_secs(60), "program did not finish");
    assert!(
        status.success(),
        "{name}: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout.lines().map(str::to_string).collect()
}

/// `json::parse(File)` on a real disk file gives exactly what `json::parse
/// (String)` gives for the same text, and leaves the file at end of input.
#[test]
fn parse_from_a_disk_file_matches_parse_from_the_same_string() {
    let out_dir = std::env::temp_dir().join(format!("mfb_bug815_disk_{}", common::unique_nonce()));
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let path = common::mfb_path_literal(&out_dir.join("doc.json"));
    let source = format!(
        r#"IMPORT fs
IMPORT io
IMPORT json

FUNC main() AS Nothing
  LET text AS String = "{{\"a\":1,\"b\":[true,null,\"x\"]}}"
  fs::writeText("{path}", text)

  LET fromString AS json::Json = json::parse(text)
  RES f = fs::openFile("{path}")
  LET fromFile AS json::Json = json::parse(f)

  io::print(json::stringify(fromFile))
  io::print(toString(json::stringify(fromFile) = json::stringify(fromString)))
  io::print(toString(fs::eof(f)))
END FUNC
"#
    );
    let lines = run("bug815_disk_file", &source);
    assert_eq!(
        lines,
        vec![
            "{\"a\":1,\"b\":[true,null,\"x\"]}".to_string(),
            "TRUE".to_string(),
            "TRUE".to_string(),
        ],
        "parse(File) must match parse(String) and leave the file at EOF"
    );
}

/// The same equivalence over `fs::openData`, the in-memory `File` bug-809
/// introduced — and the reviver overload, over both input kinds.
#[test]
fn parse_from_an_open_data_file_matches_parse_from_the_same_string_including_reviver() {
    let source = r#"IMPORT fs
IMPORT io
IMPORT json

FUNC double(key AS String, value AS json::Json) AS json::Json
  MATCH value
    CASE json::JsonNum(n)
      RETURN json::JsonNum[n.value * 2.0]
    CASE ELSE
      RETURN value
  END MATCH
END FUNC

FUNC main() AS Nothing
  LET text AS String = "{\"a\":1,\"b\":[2,3]}"

  RES f1 = fs::openData(text, "r")
  LET viaFile AS json::Json = json::parse(f1)
  LET viaString AS json::Json = json::parse(text)
  io::print(toString(json::stringify(viaFile) = json::stringify(viaString)))

  RES f2 = fs::openData(text, "r")
  LET revivedFile AS json::Json = json::parse(f2, double)
  LET revivedString AS json::Json = json::parse(text, double)
  io::print(json::stringify(revivedFile))
  io::print(toString(json::stringify(revivedFile) = json::stringify(revivedString)))
  io::print(toString(fs::eof(f2)))
END FUNC
"#;
    let lines = run("bug815_opendata_file", source);
    assert_eq!(
        lines,
        vec![
            "TRUE".to_string(),
            "{\"a\":2,\"b\":[4,6]}".to_string(),
            "TRUE".to_string(),
            "TRUE".to_string(),
        ],
    );
}

/// A malformed document raises the same error code through the File overload
/// as it does through the String overload — proving the shared parser sees
/// identical bytes either way.
#[test]
fn a_malformed_document_raises_the_same_code_through_either_overload() {
    let source = r#"IMPORT fs
IMPORT io
IMPORT json

FUNC viaFile(text AS String) AS String
  RES f = fs::openData(text, "r")
  LET doc AS json::Json = json::parse(f)
  RETURN "ok"
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC viaString(text AS String) AS String
  LET doc AS json::Json = json::parse(text)
  RETURN "ok"
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC main() AS Nothing
  LET text AS String = "{\"a\": }"
  io::print(viaFile(text))
  io::print(viaString(text))
END FUNC
"#;
    let lines = run("bug815_malformed", source);
    assert_eq!(lines.len(), 2, "both calls must raise: {lines:?}");
    assert_eq!(
        lines[0], lines[1],
        "File and String overloads must raise the same code for the same text"
    );
    assert!(lines[0].starts_with("raised 77050003"), "{lines:?}");
}

/// A closed `File` raises the usual resource-closed error rather than, say,
/// silently reading nothing or panicking.
#[test]
fn a_closed_file_raises_the_resource_closed_error() {
    let source = r#"IMPORT fs
IMPORT io
IMPORT json

FUNC closeIt(RES x AS fs::File) AS Nothing
  fs::close(x)
END FUNC

FUNC tryParse(RES f AS fs::File) AS String
  LET doc AS json::Json = json::parse(f)
  RETURN "ok"
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC main() AS Nothing
  RES f = fs::openData("{\"a\":1}", "r")
  closeIt(f)
  io::print(tryParse(f))
END FUNC
"#;
    let lines = run("bug815_closed_file", source);
    // ErrResourceClosed is 77030004.
    assert_eq!(lines, vec!["raised 77030004".to_string()]);
}

/// A file that is not valid UTF-8 raises what `fs::readAll` raises for the
/// same bytes: `ErrEncoding`.
#[test]
fn a_non_utf8_file_raises_the_encoding_error() {
    let out_dir =
        std::env::temp_dir().join(format!("mfb_bug815_badenc_{}", common::unique_nonce()));
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let file_path = out_dir.join("bad.json");
    // A lone continuation byte: never valid UTF-8 on its own.
    std::fs::write(&file_path, [0x7bu8, 0xFF, 0x7d]).expect("write non-UTF-8 file");
    let path = common::mfb_path_literal(&file_path);
    let source = format!(
        r#"IMPORT fs
IMPORT io
IMPORT json

FUNC tryParse(RES f AS fs::File) AS String
  LET doc AS json::Json = json::parse(f)
  RETURN "ok"
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC main() AS Nothing
  RES f = fs::openFile("{path}")
  io::print(tryParse(f))
END FUNC
"#
    );
    let lines = run("bug815_bad_encoding", &source);
    // ErrEncoding is 77020004.
    assert_eq!(lines, vec!["raised 77020004".to_string()]);
}

/// `json::parse(File)` reads only from the file's current position, not from
/// its start — so a caller that already consumed a prefix (e.g. with
/// `fs::readLine`) parses only what remains, exactly like `fs::readAll`.
#[test]
fn parse_starts_at_the_files_current_position() {
    let source = r#"IMPORT fs
IMPORT io
IMPORT json

FUNC main() AS Nothing
  RES f = fs::openData("garbage line not json\n{\"a\":1}", "r")
  LET skipped AS String = fs::readLine(f)
  LET doc AS json::Json = json::parse(f)
  io::print(json::stringify(doc))
END FUNC
"#;
    let lines = run("bug815_from_position", source);
    assert_eq!(lines, vec!["{\"a\":1}".to_string()]);
}
