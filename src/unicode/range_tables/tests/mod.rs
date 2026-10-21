use super::*;

#[test]
fn gencat_table_shape() {
    let table = gencat();
    assert_eq!(table.runs.len(), 4099);
    assert_eq!(table.names.len(), 30);
    // "Lu" -> 8 (len) + 2 + 1 NUL = 11, rounded to 16.
    assert_eq!(table.name_stride(), 16);
    assert_eq!(table.runs_bytes().len(), 4099 * 8);
    assert_eq!(table.names_bytes().len(), 30 * 16);
}

#[test]
fn linebreak_table_shape() {
    let table = linebreak();
    assert_eq!(table.runs.len(), 2787);
    assert_eq!(table.values_hex().len(), 2787 * 16);
    // U+0041 A is AL (27); U+3042 HIRAGANA A is ID (29) and East Asian Wide (+64).
    assert_eq!(table.lookup(0x41), "27");
    assert_eq!(table.lookup(0x3042), "93");
}

#[test]
fn script_table_shape() {
    let table = script();
    assert_eq!(table.runs.len(), 1708);
    assert_eq!(table.names.len(), 171);
    assert_eq!(table.runs_bytes().len(), 1708 * 8);
    assert_eq!(table.names_bytes().len(), 171 * table.name_stride());
}

/// A name record is a well-formed `mfb.string.v1`: little-endian length,
/// the bytes, a NUL. The emitted lookup hands this pointer back as a String,
/// so a wrong length or a missing terminator is a wrong string at runtime.
#[test]
fn name_records_are_well_formed_strings() {
    for table in [gencat(), script()] {
        let stride = table.name_stride();
        let bytes = table.names_bytes();
        for (index, name) in table.names.iter().enumerate() {
            let record = &bytes[index * stride..(index + 1) * stride];
            let length = u64::from_le_bytes(record[..8].try_into().expect("8 bytes"));
            assert_eq!(length as usize, name.len(), "length of `{name}`");
            assert_eq!(
                &record[8..8 + name.len()],
                name.as_bytes(),
                "bytes of `{name}`"
            );
            assert_eq!(record[8 + name.len()], 0, "NUL after `{name}`");
        }
    }
}

/// Boundary scalars, answered against the pinned tables.
#[test]
fn gencat_boundaries() {
    let table = gencat();
    assert_eq!(table.lookup(0), "Cc");
    assert_eq!(table.lookup(0x1F), "Cc");
    assert_eq!(table.lookup(0x20), "Zs");
    assert_eq!(table.lookup(0x41), "Lu");
    assert_eq!(table.lookup(0x61), "Ll");
    assert_eq!(table.lookup(0x30), "Nd");
    // Surrogates are Cs even though `char::from_u32` refuses them.
    assert_eq!(table.lookup(0xD800), "Cs");
    assert_eq!(table.lookup(0xDFFF), "Cs");
    assert_eq!(table.lookup(0x10FFFF), "Cn");
    // U+0295 is Ll in Unicode 16.0.0 and Lo in the vendored utf8proc data:
    // the one scalar that proves this table is not reading the trie.
    assert_eq!(table.lookup(0x295), "Ll");
}

/// The binary search answers what a linear scan of the same runs answers,
/// for **every** scalar `0 ..= 0x10FFFF`, in both tables.
///
/// Two different algorithms over one dataset: a bisection off-by-one at a run
/// boundary shows up here and essentially nowhere else, because every fixture
/// that exercises `\p{gc}` or `\p{Script}` touches a handful of scalars and
/// over a million of them are unassigned. That the runs themselves are the
/// pinned Unicode 16.0.0 answers is `scripts/check-generated.sh`'s job — it
/// regenerates both artifacts from `unicodedata` / the vendored UCD and
/// byte-compares.
///
/// When these tables replaced the generated `unicode_gencat.mfb` /
/// `unicode_script_of.mfb` IF-chains, the same exhaustive sweep ran against
/// those files' parsed arms and matched on all 1,114,112 scalars of both
/// tables. The IF-chains are deleted, so that form of the test went with
/// them; this is the part that stays checkable.
#[test]
fn binary_search_matches_a_linear_scan_for_every_scalar() {
    for table in [gencat(), script(), linebreak()] {
        let mut run = 0;
        for codepoint in 0..=0x10FFFFu32 {
            while table.runs[run].0 < codepoint {
                run += 1;
            }
            assert_eq!(
                table.lookup(codepoint),
                table.names[table.runs[run].1 as usize],
                "U+{codepoint:04X}"
            );
        }
    }
}

/// plan-162-E: every scalar's Line_Break value, re-derived here from the vendored UCD
/// files by an independent reading of UAX #14 LB1 and the flag rules, equals what the
/// bisection over the generated table answers. The Python generator and this test
/// share no code, so the artifact is checked against the source data, not only
/// against itself.
#[test]
fn linebreak_matches_the_ucd_for_every_scalar() {
    const CLASSES: [&str; 43] = [
        "BK", "CR", "LF", "NL", "SP", "ZW", "ZWJ", "CM", "WJ", "GL", "BA", "BB", "B2", "HY", "CB",
        "CL", "CP", "EX", "IN", "NS", "OP", "QU", "IS", "NU", "PO", "PR", "SY", "AL", "HL", "ID",
        "EB", "EM", "H2", "H3", "JL", "JV", "JT", "RI", "AK", "AP", "AS", "VF", "VI",
    ];
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/third_party/unicode/");
    let property = |file: &str, default: &str| -> Vec<String> {
        let mut values = vec![default.to_string(); 0x110000];
        let text = std::fs::read_to_string(format!("{root}{file}")).expect(file);
        for line in text.lines() {
            let data = line.split('#').next().unwrap_or("").trim();
            let Some((range, value)) = data.split_once(';') else {
                continue;
            };
            let (lo, hi) = match range.trim().split_once("..") {
                Some((lo, hi)) => (lo, hi),
                None => (range.trim(), range.trim()),
            };
            let lo = usize::from_str_radix(lo, 16).expect("hex");
            let hi = usize::from_str_radix(hi, 16).expect("hex");
            for value_slot in &mut values[lo..=hi] {
                *value_slot = value.trim().to_string();
            }
        }
        values
    };
    let lb = property("LineBreak-16.0.0.txt", "XX");
    let ea = property("EastAsianWidth-16.0.0.txt", "N");
    let emoji = property("emoji-data-16.0.0.txt", "");
    let cats = gencat();
    let table = linebreak();
    for cp in 0..=0x10FFFFu32 {
        let cat = cats.lookup(cp);
        let raw = lb[cp as usize].as_str();
        let class = match raw {
            "AI" | "SG" | "XX" => "AL",
            "SA" if cat == "Mn" || cat == "Mc" => "CM",
            "SA" => "AL",
            "CJ" => "NS",
            other => other,
        };
        let mut value = CLASSES
            .iter()
            .position(|c| *c == class)
            .unwrap_or_else(|| panic!("U+{cp:04X}: class {class}")) as u32;
        if matches!(ea[cp as usize].as_str(), "F" | "W" | "H") {
            value += 64;
        }
        if emoji[cp as usize] == "Extended_Pictographic" && cat == "Cn" {
            value += 128;
        }
        if class == "QU" && cat == "Pi" {
            value += 256;
        }
        if class == "QU" && cat == "Pf" {
            value += 512;
        }
        assert_eq!(table.lookup(cp), value.to_string(), "U+{cp:04X}");
    }
}

#[test]
fn script_boundaries() {
    let table = script();
    assert_eq!(table.lookup(0), "Common");
    assert_eq!(table.lookup(0x41), "Latin");
    assert_eq!(table.lookup(0x3B1), "Greek");
    assert_eq!(table.lookup(0x410), "Cyrillic");
    assert_eq!(table.lookup(0x10FFFF), "Unknown");
}
