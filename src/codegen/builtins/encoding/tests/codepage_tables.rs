use super::helper_codepage_table::{BODY, VARIANTS};
use crate::codegen::registry::registry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The scalar a table carries for a byte its codepage leaves unmapped.
const SENTINEL: u32 = 0xFFFD;

fn index_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("codepage-index")
}

/// Parse one vendored index file into `pointer -> code point`. The format is
/// `<pointer>\t0x<hex>\t<char> (<NAME>)`, with `#` comment lines; the trailing
/// prose column is dropped.
fn read_index(label: &str) -> BTreeMap<u32, u32> {
    let path = index_dir().join(format!("index-{label}.txt"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut rows = BTreeMap::new();
    for line in text.lines() {
        let body = line.split('#').next().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        let mut fields = body.split_whitespace();
        let pointer: u32 = fields.next().expect("pointer").parse().expect("pointer");
        let hex = fields.next().expect("code point");
        let cp =
            u32::from_str_radix(hex.strip_prefix("0x").unwrap_or(hex), 16).expect("code point");
        rows.insert(pointer, cp);
    }
    rows
}

/// Decode an MFBASIC `\u{XXXX}`-escaped table literal into its scalars. The
/// generator emits every entry as an escape, so anything else in the literal is
/// a generator bug.
fn scalars(literal: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut rest = literal;
    while !rest.is_empty() {
        let (hex, tail) = rest
            .strip_prefix("\\u{")
            .and_then(|r| r.split_once('}'))
            .unwrap_or_else(|| panic!("table literal is not all escapes at {rest}"));
        out.push(u32::from_str_radix(hex, 16).expect("escape hex"));
        rest = tail;
    }
    out
}

/// `(variant name, table literal)` read out of the generated `MATCH` body, in
/// declaration order.
fn arms() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = BODY.lines();
    while let Some(line) = lines.next() {
        let Some(name) = line.trim().strip_prefix("CASE Codepage.") else {
            continue;
        };
        let ret = lines.next().expect("a CASE arm has a RETURN").trim();
        let literal = ret
            .strip_prefix("RETURN \"")
            .and_then(|r| r.strip_suffix('"'))
            .unwrap_or_else(|| panic!("CASE {name} does not return a string literal"));
        out.push((name.to_string(), literal.to_string()));
    }
    out
}

fn table_of(name: &str) -> String {
    arms()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no CASE arm for {name}"))
        .1
}

/// Every scalar of every table equals the vendored index file's mapping for that
/// byte, or the sentinel where the file has no row. This is the plan's real gate:
/// it re-derives the answer from the upstream data rather than trusting the
/// generated file, so an off-by-one anywhere in any table fails here.
#[test]
fn codepage_tables_match_the_vendored_index_files() {
    let arms = arms();
    assert_eq!(
        arms.len(),
        VARIANTS.len(),
        "every variant needs exactly one MATCH arm"
    );

    let mut checked = 0usize;
    for ((arm_name, literal), (name, label, _desc)) in arms.iter().zip(VARIANTS.iter()) {
        assert_eq!(
            arm_name.as_str(),
            *name,
            "arm order must match variant order"
        );
        if label.is_empty() {
            // `Utf8` has no single-byte table: both callers branch on it before
            // the lookup, so its arm answers with the empty string.
            assert_eq!(literal, "", "{name} has no index file, so no table");
            continue;
        }
        let table = scalars(literal);
        assert_eq!(table.len(), 128, "{name}: a table is 128 scalars");
        let index = read_index(label);
        for (i, got) in table.iter().enumerate() {
            let want = index.get(&(i as u32)).copied().unwrap_or(SENTINEL);
            assert_eq!(
                *got,
                want,
                "{name} (index-{label}.txt) byte 0x{:02X}: table has U+{got:04X}, \
                     the index file has U+{want:04X}",
                128 + i
            );
            checked += 1;
        }
        // The sentinel must never collide with a real mapping, or an unmapped
        // byte would decode to U+FFFD instead of raising.
        for cp in index.values() {
            assert_ne!(
                *cp, SENTINEL,
                "index-{label}.txt maps a byte to the U+FFFD hole sentinel"
            );
        }
    }
    assert_eq!(checked, 28 * 128, "28 single-byte variants x 128 bytes");
}

/// No index file is orphaned: every vendored table is reachable from a variant.
/// Without this, adding a file and forgetting the variant is silent.
#[test]
fn every_vendored_index_file_has_a_codepage_variant() {
    let used: BTreeSet<String> = VARIANTS
        .iter()
        .map(|(_n, label, _d)| (*label).to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let mut vendored = BTreeSet::new();
    for entry in std::fs::read_dir(index_dir()).expect("read tools/codepage-index") {
        let name = entry.expect("dir entry").file_name();
        let name = name.to_string_lossy().into_owned();
        if let Some(label) = name
            .strip_prefix("index-")
            .and_then(|n| n.strip_suffix(".txt"))
        {
            vendored.insert(label.to_string());
        }
    }
    assert_eq!(vendored, used, "vendored index files vs. variants in use");
    assert_eq!(vendored.len(), 27, "27 distinct WHATWG single-byte tables");
}

/// The registered enum is exactly `VARIANTS`, in order. Variant order fixes the
/// discriminants and is a compatibility surface, so the pinned positions below
/// also assert that a later change appends rather than reorders.
#[test]
fn codepage_enum_is_registered_in_generator_order() {
    let pkg = registry().resolve_package("encoding").expect("encoding");
    let cp = pkg
        .enums()
        .iter()
        .find(|e| e.name == "Codepage")
        .expect("Codepage enum");
    assert!(cp.export, "Codepage must be visible to importers");
    let names: Vec<&str> = cp.variants.iter().map(|v| v.name).collect();
    let want: Vec<&str> = VARIANTS.iter().map(|(n, _l, _d)| *n).collect();
    assert_eq!(names, want);
    assert_eq!(
        names.len(),
        29,
        "UTF-8 plus the 28 WHATWG single-byte labels"
    );
    assert_eq!(names[0], "Utf8", "discriminant 0 is pinned");
    assert_eq!(names[21], "Windows1252", "discriminant 21 is pinned");
    assert_eq!(names[28], "MacCyrillic", "discriminant 28 is pinned");
    assert!(
        cp.variants.iter().all(|v| !v.description.is_empty()),
        "every variant renders a description in `mfb man encoding types`"
    );
}

/// ISO-8859-8-I has no index file of its own (upstream serves HTTP 404 for it)
/// and shares ISO-8859-8's mapping — the two differ only in display direction.
/// Pinned so a later edit neither invents a table for it nor drops the variant.
#[test]
fn iso_8859_8_i_shares_the_iso_8859_8_table() {
    assert_eq!(table_of("Iso8859_8I"), table_of("Iso8859_8"));
    assert!(!index_dir().join("index-iso-8859-8-i.txt").exists());
}

/// The rt fixture's per-codepage digest lines, recomputed here from the vendored
/// index files.
///
/// `func_encoding_codepageDecode_rt` walks all 128 high bytes of every
/// single-byte codepage and prints `<name> mapped=<n> sum=<s>`, where `sum` is
/// **position-weighted** — byte `b` contributes `(b - 127) * code point`. That
/// weight is what makes the line differential: a table shifted by one byte keeps
/// the same code points and the same mapped count but cannot keep the same sum.
/// Recomputing every line here is what stops the golden from being blessed wrong;
/// without it the fixture would only pin whatever the compiler happened to do.
#[test]
fn codepage_digests_match_the_vendored_index_files() {
    let log = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/rt-behavior/encoding/func_encoding_codepageDecode_rt/golden/build.log");
    let text =
        std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("read {}: {e}", log.display()));

    let mut got: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(" mapped=") else {
            continue;
        };
        let (mapped, sum) = rest.split_once(" sum=").expect("digest line shape");
        got.insert(
            name.to_string(),
            (
                mapped.parse().expect("mapped count"),
                sum.parse().expect("weighted sum"),
            ),
        );
    }

    let mut want: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for (name, label, _desc) in VARIANTS {
        if label.is_empty() {
            // `Utf8` has no per-byte table, so the fixture does not digest it.
            continue;
        }
        let index = read_index(label);
        let sum: u64 = index
            .iter()
            .map(|(pointer, cp)| u64::from(pointer + 1) * u64::from(*cp))
            .sum();
        want.insert((*name).to_string(), (index.len(), sum));
    }

    assert_eq!(got, want, "rt fixture digests vs. the vendored index files");
    assert_eq!(got.len(), 28, "one digest line per single-byte codepage");
}

/// The round-trip fixture's per-codepage counts, recomputed here from the
/// vendored index files (plan-123-B).
///
/// `func_encoding_codepageEncode_rt` walks all 256 bytes of every single-byte
/// codepage, decodes each and re-encodes the result, and prints
/// `<name> ok=<n> hole=<n> bad=<n>`. `bad` counts a byte that came back as
/// something other than itself — the wrong-bytes failure the U+FFFD sentinel
/// guard exists to prevent, which no byte-count assertion would catch — so
/// **every line must report `bad=0`**. `ok` must be the 128 ASCII bytes plus the
/// codepage's mapped high bytes, and `hole` the rest; deriving both from
/// `tools/codepage-index/` here is what stops the golden from being blessed
/// wrong.
#[test]
fn codepage_roundtrip_counts_match_the_vendored_index_files() {
    let log = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/rt-behavior/encoding/func_encoding_codepageEncode_rt/golden/build.log");
    let text =
        std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("read {}: {e}", log.display()));

    let mut got: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(" ok=") else {
            continue;
        };
        let (ok, rest) = rest.split_once(" hole=").expect("round-trip line shape");
        let (hole, bad) = rest.split_once(" bad=").expect("round-trip line shape");
        got.insert(
            name.to_string(),
            (
                ok.parse().expect("ok"),
                hole.parse().expect("hole"),
                bad.parse().expect("bad"),
            ),
        );
    }

    let mut want: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for (name, label, _desc) in VARIANTS {
        if label.is_empty() {
            // `Utf8` is not a single-byte codepage: a lone high byte is not
            // valid UTF-8, so it has no per-byte round trip and the fixture
            // covers it separately.
            continue;
        }
        let mapped = read_index(label).len();
        want.insert((*name).to_string(), (128 + mapped, 128 - mapped, 0));
    }

    assert_eq!(
        got, want,
        "round-trip counts vs. the vendored index files (third number is `bad`, \
             which must be 0 everywhere)"
    );
    assert_eq!(got.len(), 28, "one line per single-byte codepage");
    assert!(
        got.values().all(|(_ok, _hole, bad)| *bad == 0),
        "a byte re-encoded to something other than itself"
    );
}
