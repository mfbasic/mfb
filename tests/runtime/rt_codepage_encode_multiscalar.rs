//! bug-861: `encoding::codepageEncode` rejected every multi-scalar grapheme,
//! because it walked `strings::graphemes(text)` and failed any grapheme whose
//! `len` was not 1. In this runtime `"\r\n"` is a single two-scalar grapheme, so
//! every Windows-1252/KOI8/IBM866/etc. text with CRLF line endings failed to
//! encode — and so did a base letter followed by a combining mark that the
//! codepage defines as its own byte (Windows-1258's combining acute `U+0301` at
//! byte `0xEC`, Windows-1255's Hebrew points, ISO-8859-6's Arabic harakat).
//! `codepageDecode` produces exactly those strings from valid bytes, so the
//! spec's round-trip guarantee (`src/docs/spec/stdlib/08_encoding.md`
//! §Legacy single-byte codepages, "Round-trip") was false.
//!
//! A codepage maps Unicode SCALARS, not graphemes: the fix walks
//! `__encoding_codepoints(text)` instead of `strings::graphemes(text)`, so each
//! scalar of a multi-scalar grapheme is looked up (and emitted) independently.
//!
//! This covers `codepageEncode`'s one overload across every codepage it
//! supports: the CRLF case (universal — `\r`/`\n` are both below `U+0080` in
//! every codepage, so the grapheme-splitting bug hit all 28 single-byte
//! codepages identically) on all 28, the three combining-mark cases the bug
//! doc names specifically, the `Utf8` member (unaffected control), and the
//! non-goals that must NOT change (`U+FFFD` still rejected before the table
//! search, an unmappable scalar still raises, decode unaffected, the empty
//! string still encodes to no bytes).

#[path = "../common/mod.rs"]
mod common;

/// Every single-byte `Codepage` member (`src/codegen/builtins/encoding/helper_codepage_table.rs`),
/// in discriminant order — the same 28 the existing whole-range round-trip
/// fixture (`tests/rt-behavior/encoding/func_encoding_codepageEncode_rt`) walks.
const SINGLE_BYTE_CODEPAGES: &[&str] = &[
    "Ibm866",
    "Iso8859_2",
    "Iso8859_3",
    "Iso8859_4",
    "Iso8859_5",
    "Iso8859_6",
    "Iso8859_7",
    "Iso8859_8",
    "Iso8859_8I",
    "Iso8859_10",
    "Iso8859_13",
    "Iso8859_14",
    "Iso8859_15",
    "Iso8859_16",
    "Koi8R",
    "Koi8U",
    "Macintosh",
    "Windows874",
    "Windows1250",
    "Windows1251",
    "Windows1252",
    "Windows1253",
    "Windows1254",
    "Windows1255",
    "Windows1256",
    "Windows1257",
    "Windows1258",
    "MacCyrillic",
];

fn run(source: &str) -> String {
    let project = common::temp_project(&format!("b861_{}", common::unique_nonce()), source);
    let exe = common::build_project(&project);
    let out = common::run_bounded(&exe, std::time::Duration::from_secs(20), "bug-861 fixture");
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        out.0.success(),
        "program exited {}; output:\n{}",
        common::exit_description(&out.0),
        out.1
    );
    out.1
}

// ---------------------------------------------------------------------------
// CRLF round-trips on every single-byte codepage (the universal case: every
// codepage maps bytes 0x0D/0x0A to themselves, so the grapheme-splitting bug
// rejected this identically on all 28).
// ---------------------------------------------------------------------------

#[test]
fn crlf_round_trips_on_every_single_byte_codepage() {
    let mut body = String::from(
        "IMPORT io\nIMPORT encoding\n\n\
         FUNC crlf(cp AS encoding::Codepage) AS String\n\
        \x20 LET bs AS List OF Byte = [toByte(13), toByte(10)]\n\
        \x20 LET s AS String = encoding::codepageDecode(cp, bs)\n\
        \x20 RETURN encoding::hexEncode(encoding::codepageEncode(cp, s))\n\
        \x20 TRAP(e)\n\
        \x20\x20 RETURN \"FAIL \" & toString(e.code)\n\
        \x20 END TRAP\n\
         END FUNC\n\n\
         FUNC main AS Integer\n",
    );
    for cp in SINGLE_BYTE_CODEPAGES {
        body.push_str(&format!(
            "  io::print(\"{cp} \" & crlf(encoding::Codepage.{cp}))\n"
        ));
    }
    body.push_str("  RETURN 0\nEND FUNC\n");

    let stdout = run(&body);
    for cp in SINGLE_BYTE_CODEPAGES {
        let expected = format!("{cp} 0d0a");
        assert!(
            stdout.lines().any(|line| line == expected),
            "{cp}: CRLF must round-trip to 0d0a (a two-scalar grapheme, \
             both scalars below U+0080); got:\n{stdout}"
        );
    }
}

// ---------------------------------------------------------------------------
// The bug doc's named combining-mark cases: a base scalar plus a combining
// mark the codepage defines as its own byte.
// ---------------------------------------------------------------------------

#[test]
fn combining_mark_sequences_named_in_the_bug_doc_round_trip() {
    let source = r#"IMPORT io
IMPORT encoding

FUNC rt(label AS String, cp AS encoding::Codepage, b0 AS Integer, b1 AS Integer) AS Nothing
  LET bs AS List OF Byte = [toByte(b0), toByte(b1)]
  LET s AS String = encoding::codepageDecode(cp, bs)
  LET back AS List OF Byte = encoding::codepageEncode(cp, s) TRAP(e)
    io::print(label & " FAIL " & toString(e.code))
    RETURN
  END TRAP
  io::print(label & " " & encoding::hexEncode(back))
END FUNC

FUNC main AS Integer
  ' a (0x61) + combining acute U+0301, which Windows-1258 spells at 0xEC.
  rt("1258_a_acute", encoding::Codepage.Windows1258, 97, 236)
  ' bet (0xE1) + qamats U+05B8, which Windows-1255 spells at 0xC8.
  rt("1255_bet_qamats", encoding::Codepage.Windows1255, 225, 200)
  ' alef (0xC7) + fatha U+064E, which ISO-8859-6 spells at 0xEE.
  rt("8859_6_alef_fatha", encoding::Codepage.Iso8859_6, 199, 238)
  RETURN 0
END FUNC
"#;
    let stdout = run(source);
    assert!(
        stdout.contains("1258_a_acute 61ec"),
        "expected 1258_a_acute 61ec; got:\n{stdout}"
    );
    assert!(
        stdout.contains("1255_bet_qamats e1c8"),
        "expected 1255_bet_qamats e1c8; got:\n{stdout}"
    );
    assert!(
        stdout.contains("8859_6_alef_fatha c7ee"),
        "expected 8859_6_alef_fatha c7ee; got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Utf8 is unaffected (it never walked graphemes to begin with) — a control.
// ---------------------------------------------------------------------------

#[test]
fn utf8_multiscalar_grapheme_was_already_unaffected() {
    let source = r#"IMPORT io
IMPORT encoding

FUNC main AS Integer
  LET bs AS List OF Byte = [toByte(101), toByte(204), toByte(129)]
  LET s AS String = encoding::codepageDecode(encoding::Codepage.Utf8, bs)
  LET back AS List OF Byte = encoding::codepageEncode(encoding::Codepage.Utf8, s)
  io::print(encoding::hexEncode(back))
  RETURN 0
END FUNC
"#;
    let stdout = run(source);
    assert!(
        stdout.contains("65cc81"),
        "Utf8 e+U+0301 must round-trip unchanged; got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Non-goals: must NOT change.
// ---------------------------------------------------------------------------

#[test]
fn non_goals_are_unaffected() {
    let source = r#"IMPORT io
IMPORT encoding
IMPORT errorCode

FUNC encodeOr(cp AS encoding::Codepage, text AS String) AS String
  RETURN encoding::hexEncode(encoding::codepageEncode(cp, text))
  TRAP(err)
    IF err.code = errorCode::ErrInvalidFormat THEN
      RETURN "raised"
    END IF
    RETURN "other " & toString(err.code)
  END TRAP
END FUNC

FUNC main AS Integer
  ' U+FFFD is still rejected before the table search.
  io::print("fffd_windows874 " & encodeOr(encoding::Codepage.Windows874, "\u{FFFD}"))
  io::print("fffd_windows1252 " & encodeOr(encoding::Codepage.Windows1252, "\u{FFFD}"))
  ' An unmappable scalar still raises -- U+4E16 has no byte in Windows-1252.
  io::print("unmappable_1252 " & encodeOr(encoding::Codepage.Windows1252, "\u{4E16}"))
  ' The empty string still encodes to no bytes.
  io::print("empty [" & encodeOr(encoding::Codepage.Iso8859_5, "") & "]")
  ' decodeBody/codepageDecode is byte-wise and unaffected: a single mapped high
  ' byte still decodes and re-encodes to itself.
  io::print("ascii " & encodeOr(encoding::Codepage.Windows1252, "cafe"))
  RETURN 0
END FUNC
"#;
    let stdout = run(source);
    assert!(stdout.contains("fffd_windows874 raised"), "got:\n{stdout}");
    assert!(stdout.contains("fffd_windows1252 raised"), "got:\n{stdout}");
    assert!(stdout.contains("unmappable_1252 raised"), "got:\n{stdout}");
    assert!(stdout.contains("empty []"), "got:\n{stdout}");
    assert!(stdout.contains("ascii 63616665"), "got:\n{stdout}");
}
