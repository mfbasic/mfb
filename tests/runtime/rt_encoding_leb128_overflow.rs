//! A LEB128 sequence whose value does not fit a 64-bit `Integer` raises
//! `ErrInvalidFormat` instead of decoding to a wrong value (bug-619).
//!
//! The decoders checked `shift > 63` *before* reading each byte, so a tenth byte
//! was read at `shift = 63` and its payload shifted left by 63: every payload bit
//! above the lowest fell off the top of the register and the call returned a
//! value with no error. The tenth byte holds exactly one bit of a 64-bit pattern,
//! so what it may carry depends on the member:
//!
//! - `sleb128Decode`: the byte is bit 63 plus the sign extension of it, so only
//!   `0x00` (bit 63 clear) and `0x7F` (bit 63 set, negative) fit.
//! - `uleb128Decode`: the result is non-negative, so bit 63 must be clear — only
//!   `0x00` fits. Nine `0xFF` bytes then `0x01` is 2^64-1, which decoded to `-1`.
//! - `varintDecode`: the ZigZag pattern uses all 64 bits (`varintEncode` of the
//!   most negative `Integer` ends in `0x01`), so `0x00` and `0x01` fit.
//!
//! **The oracle is an independent Rust decode** of each byte list into an
//! `i128`, checked against the `Integer` range, so no expected line is copied
//! from the compiler under test.

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

const ERR_INVALID_FORMAT: i64 = 77050003;

#[derive(Clone, Copy)]
enum Member {
    Sleb,
    Uleb,
    Varint,
}

impl Member {
    fn name(self) -> &'static str {
        match self {
            Member::Sleb => "sleb128Decode",
            Member::Uleb => "uleb128Decode",
            Member::Varint => "varintDecode",
        }
    }

    /// Decode `bytes` as this member's format into the exact mathematical value,
    /// or `None` when the sequence is malformed or the value leaves `Integer`.
    fn oracle(self, bytes: &[u8]) -> Option<i64> {
        let mut unsigned: u128 = 0;
        let mut shift = 0u32;
        let mut last = None;
        for &b in bytes {
            if shift >= 70 {
                return None;
            }
            unsigned |= u128::from(b & 0x7F) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                last = Some(b);
                break;
            }
        }
        let last = last?;
        let value: i128 = match self {
            Member::Uleb => unsigned as i128,
            Member::Sleb => {
                if last & 0x40 != 0 {
                    unsigned as i128 - (1i128 << shift)
                } else {
                    unsigned as i128
                }
            }
            Member::Varint => {
                if unsigned > u128::from(u64::MAX) {
                    return None;
                }
                let u = unsigned as u64;
                return Some(((u >> 1) as i64) ^ -((u & 1) as i64));
            }
        };
        i64::try_from(value).ok()
    }
}

fn run_cases(name: &str, cases: &[(Member, Vec<u8>)]) -> Vec<(String, String)> {
    let mut body = String::new();
    for (index, (member, bytes)) in cases.iter().enumerate() {
        let list = bytes
            .iter()
            .map(|b| format!("toByte({b})"))
            .collect::<Vec<_>>()
            .join(", ");
        body.push_str(&format!(
            "  LET b{index} AS List OF Byte = [{list}]\n  io::print(one{m}(b{index}))\n",
            m = member.name(),
        ));
    }
    let source = format!(
        r#"IMPORT io
IMPORT encoding

FUNC onesleb128Decode(data AS List OF Byte) AS String
  RETURN "ok " & toString(encoding::sleb128Decode(data))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC oneuleb128Decode(data AS List OF Byte) AS String
  RETURN "ok " & toString(encoding::uleb128Decode(data))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC onevarintDecode(data AS List OF Byte) AS String
  RETURN "ok " & toString(encoding::varintDecode(data))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
{body}END SUB
"#
    );
    let project = common::temp_project(name, &source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(60),
        "LEB128 decode cases must finish",
    );
    assert!(
        status.success(),
        "{name}: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), cases.len(), "{name}: one line per case:\n{stdout}");
    cases
        .iter()
        .zip(lines)
        .map(|((member, bytes), actual)| {
            let expected = match member.oracle(bytes) {
                Some(v) => format!("ok {v}"),
                None => format!("raised {ERR_INVALID_FORMAT}"),
            };
            let label = format!(
                "{}([{}])",
                member.name(),
                bytes.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(", ")
            );
            assert!(
                actual == expected || actual.starts_with("ok ") || actual.starts_with("raised "),
                "{label}: unparseable line {actual:?}"
            );
            (format!("{label}: expected {expected}"), actual.to_string())
        })
        .collect()
}

fn assert_all(results: &[(String, String)]) {
    let wrong: Vec<String> = results
        .iter()
        .filter(|(label, actual)| !label.ends_with(&format!("expected {actual}")))
        .map(|(label, actual)| format!("{label}, got {actual}"))
        .collect();
    assert!(wrong.is_empty(), "wrong decodes:\n{}", wrong.join("\n"));
}

/// Nine continuation bytes carrying `low` (each `0x80 | low`), then `tenth`.
fn ten(low: u8, tenth: u8) -> Vec<u8> {
    let mut bytes = vec![0x80 | low; 9];
    bytes.push(tenth);
    bytes
}

#[test]
fn a_tenth_byte_whose_bits_do_not_fit_an_integer_raises() {
    let mut cases = Vec::new();
    for member in [Member::Sleb, Member::Uleb, Member::Varint] {
        // The filed reproduction: bit 64 set, every lower bit clear.
        cases.push((member, ten(0x00, 0x02)));
        // Every higher payload bit on its own, and all of them.
        for tenth in [0x04, 0x08, 0x10, 0x20, 0x40, 0x7E] {
            cases.push((member, ten(0x00, tenth)));
            cases.push((member, ten(0x7F, tenth)));
        }
    }
    // uleb128: bit 63 is 2^63, past `Integer` max; with every lower bit set it is
    // 2^64-1, which decoded to -1.
    cases.push((Member::Uleb, ten(0x00, 0x01)));
    cases.push((Member::Uleb, ten(0x7F, 0x01)));
    // sleb128: bit 63 without its sign extension (a positive 2^63), and a sign
    // extension without bit 63 (below the most negative `Integer`).
    cases.push((Member::Sleb, ten(0x00, 0x01)));
    cases.push((Member::Sleb, ten(0x7F, 0x01)));
    cases.push((Member::Sleb, ten(0x00, 0x3F)));
    cases.push((Member::Sleb, ten(0x00, 0x40)));

    for (member, bytes) in &cases {
        assert_eq!(member.oracle(bytes), None, "{} case must be out of range", member.name());
    }
    assert_all(&run_cases("leb128_overflow_raises", &cases));
}

#[test]
fn every_in_range_ten_byte_sequence_still_decodes() {
    let cases = vec![
        // The extremes of `Integer` in their canonical and padded forms.
        (Member::Sleb, ten(0x7F, 0x00)),  // not canonical, but 2^63-1 fits: bits 0..62 set
        (Member::Sleb, ten(0x00, 0x7F)),  // -2^63
        (Member::Sleb, ten(0x7F, 0x7F)),  // -1, padded to ten bytes
        (Member::Sleb, ten(0x00, 0x00)),  // 0, padded to ten bytes
        (Member::Uleb, ten(0x7F, 0x00)),  // 2^63-1
        (Member::Uleb, ten(0x00, 0x00)),  // 0, padded
        (Member::Uleb, vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]), // 2^63-1, nine bytes
        (Member::Varint, ten(0x7F, 0x01)), // ZigZag 2^64-1 = most negative Integer
        (Member::Varint, ten(0x7E & 0x7F, 0x01)),
        (Member::Varint, vec![0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]), // ZigZag 2^64-2 = Integer max
        (Member::Varint, ten(0x00, 0x00)),
        // Short sequences are unchanged.
        (Member::Sleb, vec![0x7E]),
        (Member::Uleb, vec![0xAC, 0x02]),
        (Member::Varint, vec![0x95, 0x01]),
        // An eleventh byte and a truncated sequence still raise.
        (Member::Sleb, { let mut b = vec![0x80; 10]; b.push(0x00); b }),
        (Member::Uleb, { let mut b = vec![0x80; 10]; b.push(0x00); b }),
        (Member::Varint, { let mut b = vec![0x80; 10]; b.push(0x00); b }),
        (Member::Uleb, vec![0x80]),
    ];
    assert_all(&run_cases("leb128_in_range_decodes", &cases));
}
