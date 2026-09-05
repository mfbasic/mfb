//! `encoding::punycodeDecode` is bounded in the label length (bug-510,
//! audit-3 DEC-05/06).
//!
//! DEC-05: the decoder inserted each code point by rebuilding its whole output list
//! with `append`, so a label of `n` code points cost `n^2` appends *and* left every
//! intermediate list in the arena — a 32 KB label took 47 s and 4.3 GB. The
//! insertion is quadratic in the label's length, so a label past 1024 octets is
//! refused as `ErrInvalidFormat` before it is decoded — sixteen times the 63-octet
//! DNS label limit (RFC 1034 §3.1; RFC 5890 §2.3.1) and past RFC 3492's own longest
//! sample (74 octets), which a 63-octet cap would have refused; within the cap the
//! insertion is the in-place shift RFC 3492 describes. DEC-06: the RFC's overflow checks on the
//! variable-length integer were missing, so an overflowing label surfaced as
//! `ErrOverflow` from the multiply rather than as malformed Punycode.
//!
//! **The oracle is Python's `punycode` codec**, an independent implementation of
//! RFC 3492: every label the tests expect to decode is decoded by both and the two
//! must agree, so a mistyped sample cannot silently pin a wrong answer.

mod common;

use std::process::Command;
use std::time::Duration;

/// Decode every line of `labels.txt` (each a full `xn--` label) and print the
/// result, or `raised <code>`.
const DECODER: &str = r#"IMPORT io
IMPORT fs
IMPORT encoding
IMPORT strings

FUNC one(label AS String) AS String
  LET d AS String = encoding::punycodeDecode(label)
  RETURN "ok " & d
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  FOR EACH line IN strings::split(fs::readText("labels.txt"), "\n")
    IF len(line) > 0 THEN
      io::print(one(line))
    END IF
  NEXT
END SUB
"#;

/// `ErrInvalidFormat` — the one code `punycodeDecode` documents for malformed input.
const ERR_INVALID_FORMAT: &str = "77050003";

fn decode_all(name: &str, labels: &[String], timeout: Duration, hang: &str) -> Vec<String> {
    let project = common::temp_project(name, DECODER);
    std::fs::write(project.join("labels.txt"), labels.join("\n") + "\n").expect("labels");
    let binary = common::build_project(&project);
    // The program reads `labels.txt` from its cwd, which `run_bounded` sets to the
    // executable's directory — so put a copy there too.
    if let Some(dir) = binary.parent() {
        std::fs::write(dir.join("labels.txt"), labels.join("\n") + "\n").expect("labels");
    }
    let (status, stdout) = common::run_bounded(&binary, timeout, hang);
    assert!(
        status.success(),
        "{name}: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout.lines().map(str::to_string).collect()
}

/// Python's RFC 3492 codec, as the independent oracle: `xn--<payload>` → text.
fn python_decode(label: &str) -> String {
    let payload = label.strip_prefix("xn--").expect("an xn-- label");
    let out = Command::new(common::python_exe())
        .arg("-c")
        .arg("import sys, codecs; sys.stdout.write(codecs.decode(sys.argv[1].encode('ascii'), 'punycode'))")
        .arg(payload)
        .output()
        .expect("run python");
    assert!(
        out.status.success(),
        "python could not decode {label}: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("utf8")
}

/// Python's encoder, for building labels of an exact octet length.
fn python_encode(text: &str) -> String {
    let out = Command::new(common::python_exe())
        .arg("-c")
        .arg("import sys, codecs; sys.stdout.write('xn--' + codecs.encode(sys.argv[1], 'punycode').decode('ascii'))")
        .arg(text)
        .output()
        .expect("run python");
    assert!(out.status.success());
    String::from_utf8(out.stdout).expect("utf8")
}

#[test]
fn a_label_past_the_length_limit_is_refused_in_bounded_time() {
    // DEC-05. The audit's shape: `xn--td` followed by thirty-two thousand `a`s
    // decodes (in principle) to thirty-two thousand `u-umlaut`s; before the fix that
    // was 47 s and 4.3 GB. It is five hundred times the longest label DNS carries,
    // so it is malformed input and is refused at once.
    let label = format!("xn--td{}", "a".repeat(32_000));
    let lines = decode_all(
        "punycode_bounds_long",
        &[label],
        Duration::from_secs(10),
        "decoding a 32 KB punycode label did not finish",
    );
    assert_eq!(
        lines,
        vec![format!("raised {ERR_INVALID_FORMAT}")],
        "a 32 KB label must be refused, not decoded",
    );
}

#[test]
fn labels_up_to_the_limit_decode_and_one_octet_past_it_is_refused() {
    // The boundary, exactly. A run of `u-umlaut`s whose A-label is exactly 1024
    // octets long decodes to what Python decodes it to; one more octet is refused.
    // The cap is on the *encoded* label, which is what a wire carries, excluding
    // the `xn--` the caller strips.
    let mut k = 1;
    let (mut at_limit, mut past_limit) = (String::new(), String::new());
    while k < 4096 {
        let label = python_encode(&"\u{FC}".repeat(k));
        let payload = label.len() - 4;
        if payload == 1024 {
            at_limit = label;
        } else if payload == 1025 {
            past_limit = label;
            break;
        }
        k += 1;
    }
    assert!(
        !at_limit.is_empty() && !past_limit.is_empty(),
        "no label landed on the boundary"
    );
    let lines = decode_all(
        "punycode_bounds_edge",
        &[at_limit.clone(), past_limit],
        Duration::from_secs(60),
        "decoding boundary labels did not finish",
    );
    assert_eq!(
        lines.first().cloned().unwrap_or_default(),
        format!("ok {}", python_decode(&at_limit))
    );
    assert_eq!(
        lines.get(1).cloned().unwrap_or_default(),
        format!("raised {ERR_INVALID_FORMAT}"),
        "a label one octet past the limit must be refused",
    );
}

#[test]
fn a_label_longer_than_dns_allows_still_decodes() {
    // The cap is a cost bound, not a DNS validity check: RFC 3492's own Korean
    // sample is 74 octets, longer than any DNS label, and it decoded before bug-510
    // — so it must decode after. (A 63-octet cap refused it, which is how the cap
    // ended up at 1024.)
    let label =
        "xn--989aomsvi5e83db1d2a355cv1e0vak1dwrv93d5xbh15a0dt30a5jpsd879ccm6fea98c".to_string();
    assert!(label.len() > 63);
    let lines = decode_all(
        "punycode_bounds_over_dns",
        &[label.clone()],
        Duration::from_secs(30),
        "decoding a 74-octet label did not finish",
    );
    assert_eq!(lines, vec![format!("ok {}", python_decode(&label))]);
}

/// RFC 3492 §7.1's sample strings, as the A-labels they encode to (the RFC gives
/// the payloads; the mixed-case ones exercise case-insensitive digits), plus a
/// few ordinary IDNs.
const SAMPLES: &[&str] = &[
    "xn--egbpdaj6bu4bxfgehfvwxn",
    "xn--ihqwcrb4cv8a8dqg056pqjye",
    "xn--ihqwctvzc91f659drss3x8bo0yb",
    "xn--Proprostnemluvesky-uyb24dma41a",
    "xn--4dbcagdahymbxekheh6e0a7fei0b",
    "xn--i1baa7eci9glrd9b2ae1bj0hfcgg6iyaf8o0a1dig0cd",
    "xn--n8jok5ay5dzabd5bym9f0cm5685rrjetr6pdxa",
    "xn--989aomsvi5e83db1d2a355cv1e0vak1dwrv93d5xbh15a0dt30a5jpsd879ccm6fea98c",
    "xn--b1abfaaepdrnnbgefbaDotcwatmq2g4l",
    "xn--PorqunopuedensimplementehablarenEspaol-fmd56a",
    "xn--TisaohkhngthchnitingVit-kjcr8268qyxafd2f1b9g",
    "xn--3B-ww4c5e180e575a65lsy2b",
    "xn---with-SUPER-MONKEYS-pc58ag80a8qai00g7n9n",
    "xn--Hello-Another-Way--fc4qua05auwb3674vfr0b",
    "xn--2-u9tlzr9756bt3uc0v",
    "xn--MajiKoi5-783gue6qz075azm5e",
    "xn--de-jg4avhby1noc0d",
    "xn--d9juau41awczczp",
    "xn--mnchen-3ya",
    "xn--bcher-kva",
    "xn--80akhbyknj4f",
    "xn--fsqu00a",
];

#[test]
fn rfc_3492_sample_strings_decode_identically() {
    // The positive half: every sample decodes to exactly what the independent
    // implementation says, before and after the fix.
    let labels: Vec<String> = SAMPLES.iter().map(|s| s.to_string()).collect();
    let lines = decode_all(
        "punycode_bounds_samples",
        &labels,
        Duration::from_secs(60),
        "decoding the RFC samples did not finish",
    );
    assert_eq!(lines.len(), labels.len(), "{lines:?}");
    for (label, line) in labels.iter().zip(&lines) {
        assert_eq!(*line, format!("ok {}", python_decode(label)), "{label}");
    }
}

#[test]
fn an_overflowing_integer_is_malformed_punycode_not_an_arithmetic_error() {
    // DEC-06. RFC 3492 §6.2 requires the decoder to fail when `i + digit * w`
    // would overflow; this label's digits keep the variable-length integer going
    // until it does. Before the fix the multiply itself raised `ErrOverflow`
    // (77050010) — an arithmetic error from inside the decoder, not a verdict on
    // the input. Python refuses it too.
    let label = "xn--99999999999999999999999".to_string();
    let lines = decode_all(
        "punycode_bounds_overflow",
        &[label],
        Duration::from_secs(30),
        "decoding an overflowing label did not finish",
    );
    assert_eq!(lines, vec![format!("raised {ERR_INVALID_FORMAT}")]);
}
