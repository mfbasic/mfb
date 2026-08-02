use std::sync::OnceLock;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const UTF8PROC_DATA: &str = include_str!("../../third_party/utf8proc/utf8proc_data.c");
const U16_MAX: u16 = u16::MAX;

pub(crate) struct UnicodeRuntimeTables {
    pub(crate) sequences: Vec<u16>,
    pub(crate) stage1: Vec<u16>,
    pub(crate) stage2: Vec<u16>,
    pub(crate) properties: Vec<PackedProperty>,
    pub(crate) combinations_second: Vec<u32>,
    pub(crate) combinations_combined: Vec<u32>,
    pub(crate) nfd_entries: Vec<NfdEntry>,
    pub(crate) nfd_sequences: Vec<u32>,
    pub(crate) uppercase_entries: Vec<NfdEntry>,
    pub(crate) uppercase_sequences: Vec<u32>,
    pub(crate) lowercase_entries: Vec<NfdEntry>,
    pub(crate) lowercase_sequences: Vec<u32>,
    pub(crate) casefold_entries: Vec<NfdEntry>,
    pub(crate) casefold_sequences: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackedProperty {
    pub(crate) combining_class: u16,
    pub(crate) decomp_type: u16,
    pub(crate) decomp_seqindex: u16,
    pub(crate) casefold_seqindex: u16,
    pub(crate) uppercase_seqindex: u16,
    pub(crate) lowercase_seqindex: u16,
    pub(crate) comb_index: u16,
    pub(crate) comb_length: u16,
    pub(crate) flags: u16,
    pub(crate) boundclass: u16,
    pub(crate) indic_conjunct_break: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NfdEntry {
    pub(crate) codepoint: u32,
    pub(crate) sequence_offset: u32,
    pub(crate) sequence_length: u32,
}

impl PackedProperty {
    const COMB_IS_SECOND: u16 = 1 << 0;
    const COMP_EXCLUSION: u16 = 1 << 1;
    const IGNORABLE: u16 = 1 << 2;
    const CONTROL_BOUNDARY: u16 = 1 << 3;
    // plan-70-A: the vendored `charwidth:2` (0/1/2 terminal columns) and
    // `ambiguous_width:1` (East Asian Ambiguous) fields pack into the previously
    // unused `flags` bits 4–6, so the 24-byte record and every asserted table
    // size are unchanged — only the `flags` byte values move.
    const CHARWIDTH_SHIFT: u16 = 4;
    const CHARWIDTH_MASK: u16 = 0b11 << 4;
    const AMBIGUOUS: u16 = 1 << 6;

    /// The terminal display width (0/1/2 columns) of this codepoint, from the
    /// utf8proc `charwidth` field packed into `flags` bits 4–5. Used by the
    /// compile-time folding of `strings::displayWidth`; the runtime reads the
    /// same bits directly out of the embedded table.
    pub(crate) fn charwidth(&self) -> u16 {
        (self.flags & Self::CHARWIDTH_MASK) >> Self::CHARWIDTH_SHIFT
    }

    fn encode_le(&self, output: &mut Vec<u8>) {
        for value in [
            self.combining_class,
            self.decomp_type,
            self.decomp_seqindex,
            self.casefold_seqindex,
            self.uppercase_seqindex,
            self.lowercase_seqindex,
            self.comb_index,
            self.comb_length,
            self.flags,
            self.boundclass,
            self.indic_conjunct_break,
        ] {
            output.extend_from_slice(&value.to_le_bytes());
        }
        output.extend_from_slice(&0_u16.to_le_bytes());
    }
}

pub(crate) fn tables() -> &'static UnicodeRuntimeTables {
    static TABLES: OnceLock<UnicodeRuntimeTables> = OnceLock::new();
    TABLES.get_or_init(parse_tables)
}

pub(crate) fn stage1_hex() -> String {
    u16_hex(&tables().stage1)
}

pub(crate) fn stage2_hex() -> String {
    u16_hex(&tables().stage2)
}

pub(crate) fn sequences_hex() -> String {
    u16_hex(&tables().sequences)
}

pub(crate) fn properties_hex() -> String {
    let mut bytes = Vec::new();
    for property in &tables().properties {
        property.encode_le(&mut bytes);
    }
    bytes_hex(&bytes)
}

/// The two-stage trie lookup, in Rust.
///
/// The emitted runtime performs this same lookup itself, against the same tables,
/// in generated code. This Rust copy is the executable statement of the algorithm:
/// the specification cites it by name (`unicode/01_tables-and-algorithms.md:57`,
/// `[[src/unicode/runtime_tables.rs:property_for_codepoint]]`), its own tests check
/// the shipped tables actually decode, and the compile-time folding of
/// `strings::displayWidth` (plan-70-A) reads `charwidth()` through it so the folded
/// and runtime paths share one table. Deleting it would leave the spec citing
/// nothing and the tables checked only indirectly (bug-326-D4).
pub(crate) fn property_for_codepoint(codepoint: u32) -> PackedProperty {
    let tables = tables();
    // The two-stage trie only covers U+0000..=U+10FFFF; anything above has no
    // stage1 slot. utf8proc's own `utf8proc_get_property` returns the index-0
    // (unassigned) property for `uc >= 0x110000` rather than indexing OOB, so
    // the reference lookup matches instead of panicking (bug-394 item 11).
    if codepoint > 0x10FFFF {
        return tables.properties[0];
    }
    let stage1 = tables.stage1[(codepoint >> 8) as usize] as usize;
    let stage2 = tables.stage2[stage1 + (codepoint & 0xff) as usize] as usize;
    tables.properties[stage2]
}

pub(crate) fn combinations_second_hex() -> String {
    u32_hex(&tables().combinations_second)
}

pub(crate) fn combinations_combined_hex() -> String {
    u32_hex(&tables().combinations_combined)
}

pub(crate) fn nfd_entries_hex() -> String {
    let mut bytes = Vec::with_capacity(tables().nfd_entries.len() * 16);
    for entry in &tables().nfd_entries {
        bytes.extend_from_slice(&entry.codepoint.to_le_bytes());
        bytes.extend_from_slice(&entry.sequence_offset.to_le_bytes());
        bytes.extend_from_slice(&entry.sequence_length.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
    }
    bytes_hex(&bytes)
}

pub(crate) fn nfd_sequences_hex() -> String {
    u32_hex(&tables().nfd_sequences)
}

pub(crate) fn uppercase_entries_hex() -> String {
    mapping_entries_hex(&tables().uppercase_entries)
}

pub(crate) fn uppercase_sequences_hex() -> String {
    u32_hex(&tables().uppercase_sequences)
}

pub(crate) fn lowercase_entries_hex() -> String {
    mapping_entries_hex(&tables().lowercase_entries)
}

pub(crate) fn lowercase_sequences_hex() -> String {
    u32_hex(&tables().lowercase_sequences)
}

pub(crate) fn casefold_entries_hex() -> String {
    mapping_entries_hex(&tables().casefold_entries)
}

pub(crate) fn casefold_sequences_hex() -> String {
    u32_hex(&tables().casefold_sequences)
}

fn parse_tables() -> UnicodeRuntimeTables {
    let (nfd_entries, nfd_sequences) = build_nfd_tables();
    let (uppercase_entries, uppercase_sequences) =
        build_mapping_tables(|value| value.to_uppercase().map(|ch| ch as u32).collect());
    let (lowercase_entries, lowercase_sequences) =
        build_mapping_tables(|value| value.to_lowercase().map(|ch| ch as u32).collect());
    let (casefold_entries, casefold_sequences) =
        build_mapping_tables(|value| value.to_string().case_fold().map(|ch| ch as u32).collect());
    UnicodeRuntimeTables {
        sequences: parse_numeric_array("utf8proc_sequences")
            .into_iter()
            .map(to_u16)
            .collect(),
        stage1: parse_numeric_array("utf8proc_stage1table")
            .into_iter()
            .map(to_u16)
            .collect(),
        stage2: parse_numeric_array("utf8proc_stage2table")
            .into_iter()
            .map(to_u16)
            .collect(),
        properties: parse_properties(),
        combinations_second: parse_numeric_array("utf8proc_combinations_second")
            .into_iter()
            .map(to_u32)
            .collect(),
        combinations_combined: parse_numeric_array("utf8proc_combinations_combined")
            .into_iter()
            .map(to_u32)
            .collect(),
        nfd_entries,
        nfd_sequences,
        uppercase_entries,
        uppercase_sequences,
        lowercase_entries,
        lowercase_sequences,
        casefold_entries,
        casefold_sequences,
    }
}

fn build_nfd_tables() -> (Vec<NfdEntry>, Vec<u32>) {
    build_mapping_tables(|value| value.to_string().nfd().map(|ch| ch as u32).collect())
}

fn build_mapping_tables<F>(mut mapped: F) -> (Vec<NfdEntry>, Vec<u32>)
where
    F: FnMut(char) -> Vec<u32>,
{
    let mut entries = Vec::new();
    let mut sequences = Vec::new();
    for codepoint in 0..=0x10ffff {
        let Some(value) = char::from_u32(codepoint) else {
            continue;
        };
        let sequence = mapped(value);
        if sequence.len() == 1 && sequence[0] == codepoint {
            continue;
        }
        let sequence_offset = sequences.len() as u32;
        let sequence_length = sequence.len() as u32;
        sequences.extend(sequence);
        entries.push(NfdEntry {
            codepoint,
            sequence_offset,
            sequence_length,
        });
    }
    (entries, sequences)
}

fn mapping_entries_hex(entries: &[NfdEntry]) -> String {
    let mut bytes = Vec::with_capacity(entries.len() * 16);
    for entry in entries {
        bytes.extend_from_slice(&entry.codepoint.to_le_bytes());
        bytes.extend_from_slice(&entry.sequence_offset.to_le_bytes());
        bytes.extend_from_slice(&entry.sequence_length.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
    }
    bytes_hex(&bytes)
}

fn parse_numeric_array(name: &str) -> Vec<i64> {
    let body = array_body(name);
    body.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(parse_value)
        .collect()
}

fn parse_properties() -> Vec<PackedProperty> {
    let body = array_body("utf8proc_properties");
    body.lines()
        .filter_map(|line| {
            let start = line.find('{')?;
            let end = line.rfind('}')?;
            let fields = line[start + 1..end]
                .split(',')
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .collect::<Vec<_>>();
            assert_eq!(fields.len(), 21, "unexpected utf8proc property field count");

            let mut flags = 0_u16;
            if parse_bool(fields[11]) {
                flags |= PackedProperty::COMB_IS_SECOND;
            }
            if parse_bool(fields[13]) {
                flags |= PackedProperty::COMP_EXCLUSION;
            }
            if parse_bool(fields[14]) {
                flags |= PackedProperty::IGNORABLE;
            }
            if parse_bool(fields[15]) {
                flags |= PackedProperty::CONTROL_BOUNDARY;
            }
            // charwidth (field 16) is an integer 0/1/2; ambiguous_width (field 17)
            // is emitted inconsistently in utf8proc_data.c — the index-0 record
            // uses the integer `0`, every other row uses `false`/`true` — so it
            // must go through `parse_value` (which maps both), NOT `parse_bool`
            // (which panics on `0`). Verified by reading rows 0/1 of the table.
            let charwidth = to_u16(parse_value(fields[16]));
            flags |=
                (charwidth << PackedProperty::CHARWIDTH_SHIFT) & PackedProperty::CHARWIDTH_MASK;
            if parse_value(fields[17]) != 0 {
                flags |= PackedProperty::AMBIGUOUS;
            }

            Some(PackedProperty {
                combining_class: to_u16(parse_value(fields[1])),
                decomp_type: to_u16(parse_value(fields[3])),
                decomp_seqindex: to_u16(parse_value(fields[4])),
                casefold_seqindex: to_u16(parse_value(fields[5])),
                uppercase_seqindex: to_u16(parse_value(fields[6])),
                lowercase_seqindex: to_u16(parse_value(fields[7])),
                comb_index: to_u16(parse_value(fields[9])),
                comb_length: to_u16(parse_value(fields[10])),
                flags,
                boundclass: to_u16(parse_value(fields[19])),
                indic_conjunct_break: to_u16(parse_value(fields[20])),
            })
        })
        .collect()
}

fn array_body(name: &str) -> &'static str {
    let marker = "static const ".to_string();
    let start = UTF8PROC_DATA
        .find(&marker)
        .and_then(|index| {
            UTF8PROC_DATA[index..]
                .find(&format!("{name}[] = {{"))
                .map(|offset| index + offset)
        })
        .unwrap_or_else(|| panic!("utf8proc table `{name}` not found"));
    let body_start = UTF8PROC_DATA[start..]
        .find('{')
        .map(|offset| start + offset + 1)
        .expect("utf8proc table open brace");
    let body_end = UTF8PROC_DATA[body_start..]
        .find("};")
        .map(|offset| body_start + offset)
        .expect("utf8proc table close brace");
    &UTF8PROC_DATA[body_start..body_end]
}

fn parse_value(value: &str) -> i64 {
    match value {
        "UINT16_MAX" => U16_MAX as i64,
        "true" => 1,
        "false" => 0,
        // The general-category (field 0) and bidi-class (field 2) columns are
        // never consumed — `parse_properties` reads only fields 1,3–7,9–11,
        // 13–15,19,20 — so a `UTF8PROC_CATEGORY_*` string never reaches here.
        // Bidi class still maps (to 0) for symmetry; the 30-arm category lookup
        // it used to need was dead and was removed (bug-343 A4).
        _ if value.starts_with("UTF8PROC_BIDI_CLASS_") => 0,
        _ if value.starts_with("UTF8PROC_DECOMP_TYPE_") => decomp_type_value(value) as i64,
        _ if value.starts_with("UTF8PROC_BOUNDCLASS_") => boundclass_value(value) as i64,
        _ if value.starts_with("UTF8PROC_INDIC_CONJUNCT_BREAK_") => {
            indic_conjunct_break_value(value) as i64
        }
        _ => {
            if let Some(hex) = value.strip_prefix("0x") {
                i64::from_str_radix(hex, 16).expect("utf8proc hex integer")
            } else {
                value.parse::<i64>().expect("utf8proc integer")
            }
        }
    }
}

fn parse_bool(value: &str) -> bool {
    match value {
        "true" => true,
        "false" => false,
        other => panic!("utf8proc boolean field `{other}` is not true/false"),
    }
}

fn to_u16(value: i64) -> u16 {
    u16::try_from(value).expect("utf8proc value fits u16")
}

fn to_u32(value: i64) -> u32 {
    u32::try_from(value).expect("utf8proc value fits u32")
}

fn u16_hex(values: &[u16]) -> String {
    let mut bytes = Vec::with_capacity(values.len() * 2);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes_hex(&bytes)
}

fn u32_hex(values: &[u32]) -> String {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes_hex(&bytes)
}

fn bytes_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn decomp_type_value(value: &str) -> u16 {
    match value {
        "UTF8PROC_DECOMP_TYPE_FONT" => 1,
        "UTF8PROC_DECOMP_TYPE_NOBREAK" => 2,
        "UTF8PROC_DECOMP_TYPE_INITIAL" => 3,
        "UTF8PROC_DECOMP_TYPE_MEDIAL" => 4,
        "UTF8PROC_DECOMP_TYPE_FINAL" => 5,
        "UTF8PROC_DECOMP_TYPE_ISOLATED" => 6,
        "UTF8PROC_DECOMP_TYPE_CIRCLE" => 7,
        "UTF8PROC_DECOMP_TYPE_SUPER" => 8,
        "UTF8PROC_DECOMP_TYPE_SUB" => 9,
        "UTF8PROC_DECOMP_TYPE_VERTICAL" => 10,
        "UTF8PROC_DECOMP_TYPE_WIDE" => 11,
        "UTF8PROC_DECOMP_TYPE_NARROW" => 12,
        "UTF8PROC_DECOMP_TYPE_SMALL" => 13,
        "UTF8PROC_DECOMP_TYPE_SQUARE" => 14,
        "UTF8PROC_DECOMP_TYPE_FRACTION" => 15,
        "UTF8PROC_DECOMP_TYPE_COMPAT" => 16,
        other => panic!("unknown utf8proc decomposition type `{other}`"),
    }
}

fn boundclass_value(value: &str) -> u16 {
    match value {
        "UTF8PROC_BOUNDCLASS_START" => 0,
        "UTF8PROC_BOUNDCLASS_OTHER" => 1,
        "UTF8PROC_BOUNDCLASS_CR" => 2,
        "UTF8PROC_BOUNDCLASS_LF" => 3,
        "UTF8PROC_BOUNDCLASS_CONTROL" => 4,
        "UTF8PROC_BOUNDCLASS_EXTEND" => 5,
        "UTF8PROC_BOUNDCLASS_L" => 6,
        "UTF8PROC_BOUNDCLASS_V" => 7,
        "UTF8PROC_BOUNDCLASS_T" => 8,
        "UTF8PROC_BOUNDCLASS_LV" => 9,
        "UTF8PROC_BOUNDCLASS_LVT" => 10,
        "UTF8PROC_BOUNDCLASS_REGIONAL_INDICATOR" => 11,
        "UTF8PROC_BOUNDCLASS_SPACINGMARK" => 12,
        "UTF8PROC_BOUNDCLASS_PREPEND" => 13,
        "UTF8PROC_BOUNDCLASS_ZWJ" => 14,
        "UTF8PROC_BOUNDCLASS_E_BASE" => 15,
        "UTF8PROC_BOUNDCLASS_E_MODIFIER" => 16,
        "UTF8PROC_BOUNDCLASS_GLUE_AFTER_ZWJ" => 17,
        "UTF8PROC_BOUNDCLASS_E_BASE_GAZ" => 18,
        "UTF8PROC_BOUNDCLASS_EXTENDED_PICTOGRAPHIC" => 19,
        "UTF8PROC_BOUNDCLASS_E_ZWG" => 20,
        other => panic!("unknown utf8proc boundclass `{other}`"),
    }
}

fn indic_conjunct_break_value(value: &str) -> u16 {
    match value {
        "UTF8PROC_INDIC_CONJUNCT_BREAK_NONE" => 0,
        "UTF8PROC_INDIC_CONJUNCT_BREAK_LINKER" => 1,
        "UTF8PROC_INDIC_CONJUNCT_BREAK_CONSONANT" => 2,
        "UTF8PROC_INDIC_CONJUNCT_BREAK_EXTEND" => 3,
        other => panic!("unknown utf8proc indic conjunct break `{other}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utf8proc_runtime_tables() {
        let tables = tables();
        assert_eq!(tables.stage1.len(), 4352);
        assert_eq!(tables.stage2.len(), 46336);
        assert_eq!(tables.properties.len(), 8385);
        assert_eq!(tables.sequences.len(), 12961);
        assert_eq!(
            tables.combinations_second.len(),
            tables.combinations_combined.len()
        );
        assert_eq!(tables.combinations_second.len(), 961);
        assert!(!tables.nfd_entries.is_empty());
        assert!(!tables.nfd_sequences.is_empty());
    }

    #[test]
    fn packs_properties_as_fixed_size_records() {
        let hex = properties_hex();
        assert_eq!(hex.len(), tables().properties.len() * 24 * 2);
    }

    #[test]
    fn looks_up_grapheme_break_properties() {
        assert_eq!(property_for_codepoint('a' as u32).boundclass, 1);
        assert_eq!(property_for_codepoint(0x0301).boundclass, 5);
        assert_eq!(property_for_codepoint(0x200d).boundclass, 14);
        assert_eq!(property_for_codepoint(0x1f1fa).boundclass, 11);
        assert_eq!(property_for_codepoint(0x1f468).boundclass, 19);
    }

    #[test]
    fn charwidth_field_is_parsed_from_the_utf8proc_table() {
        // plan-70-A Phase 1 falsification: the `charwidth`/`ambiguous_width`
        // fields are addressed positionally (16/17) in the C initializer, so the
        // index must be proven against real codepoints before any codegen depends
        // on it. Wide CJK/emoji = 2, ASCII/Latin = 1, a combining mark = 0.
        assert_eq!(property_for_codepoint('日' as u32).charwidth(), 2);
        assert_eq!(property_for_codepoint('本' as u32).charwidth(), 2);
        assert_eq!(property_for_codepoint('A' as u32).charwidth(), 1);
        assert_eq!(property_for_codepoint('e' as u32).charwidth(), 1);
        assert_eq!(property_for_codepoint(0x0301).charwidth(), 0); // COMBINING ACUTE
        assert_eq!(property_for_codepoint(0x1f44d).charwidth(), 2); // 👍
        assert_eq!(property_for_codepoint(0x200b).charwidth(), 0); // ZERO WIDTH SPACE
        assert_eq!(property_for_codepoint(0x200d).charwidth(), 0); // ZWJ
    }

    #[test]
    fn ambiguous_width_bit_is_parsed_and_carried() {
        // Field 17 (ambiguous_width) plumbs into flags bit 6. It is dormant
        // (policy: East-Asian Ambiguous = narrow), but must be stored so a future
        // policy flip is a one-line codegen change. Prove the bit is set for a
        // known East-Asian Ambiguous codepoint (U+00A7 SECTION SIGN) and clear for
        // a plain ASCII letter.
        assert_ne!(
            property_for_codepoint(0x00a7).flags & PackedProperty::AMBIGUOUS,
            0
        );
        assert_eq!(
            property_for_codepoint('A' as u32).flags & PackedProperty::AMBIGUOUS,
            0
        );
    }

    #[test]
    fn out_of_range_codepoints_return_the_default_property() {
        // A codepoint above U+10FFFF has no trie entry; utf8proc's own
        // `utf8proc_get_property` returns the index-0 (unassigned) property for
        // `uc >= 0x110000` rather than indexing out of bounds. The reference
        // lookup must match, not panic (bug-394 item 11).
        let default = tables().properties[0];
        assert_eq!(property_for_codepoint(0x110000), default);
        assert_eq!(property_for_codepoint(u32::MAX), default);
    }

    #[test]
    fn every_hex_serializer_emits_even_length_hex() {
        // Each `*_hex` serializer is only used by codegen, never by the runtime
        // tests, so drive them all once. A hex string is two chars per byte, so it
        // is always even length; the underlying tables are all non-empty.
        for (label, hex) in [
            ("stage1", stage1_hex()),
            ("stage2", stage2_hex()),
            ("sequences", sequences_hex()),
            ("properties", properties_hex()),
            ("combinations_second", combinations_second_hex()),
            ("combinations_combined", combinations_combined_hex()),
            ("nfd_entries", nfd_entries_hex()),
            ("nfd_sequences", nfd_sequences_hex()),
            ("uppercase_entries", uppercase_entries_hex()),
            ("uppercase_sequences", uppercase_sequences_hex()),
            ("lowercase_entries", lowercase_entries_hex()),
            ("lowercase_sequences", lowercase_sequences_hex()),
            ("casefold_entries", casefold_entries_hex()),
            ("casefold_sequences", casefold_sequences_hex()),
        ] {
            assert!(!hex.is_empty(), "{label} hex should be non-empty");
            assert_eq!(hex.len() % 2, 0, "{label} hex should be even length");
            assert!(
                hex.bytes().all(|b| b.is_ascii_hexdigit()),
                "{label} hex should be all hex digits"
            );
        }
    }

    #[test]
    fn u16_and_u32_hex_encode_little_endian() {
        assert_eq!(u16_hex(&[0x0102]), "0201");
        assert_eq!(u32_hex(&[0x0102_0304]), "04030201");
    }

    #[test]
    fn field_value_tables_map_last_arm_and_bools() {
        // The `E_ZWG` boundclass and the boolean parser's success arms are not
        // exercised by every corpus walk; assert them directly.
        assert_eq!(boundclass_value("UTF8PROC_BOUNDCLASS_E_ZWG"), 20);
        assert_eq!(decomp_type_value("UTF8PROC_DECOMP_TYPE_COMPAT"), 16);
        assert_eq!(
            indic_conjunct_break_value("UTF8PROC_INDIC_CONJUNCT_BREAK_EXTEND"),
            3
        );
        assert!(parse_bool("true"));
        assert!(!parse_bool("false"));
    }

    #[test]
    #[should_panic(expected = "not true/false")]
    fn parse_bool_rejects_non_boolean() {
        let _ = parse_bool("maybe");
    }

    #[test]
    #[should_panic(expected = "unknown utf8proc decomposition type")]
    fn decomp_type_rejects_unknown() {
        let _ = decomp_type_value("UTF8PROC_DECOMP_TYPE_BOGUS");
    }

    #[test]
    #[should_panic(expected = "unknown utf8proc boundclass")]
    fn boundclass_rejects_unknown() {
        let _ = boundclass_value("UTF8PROC_BOUNDCLASS_BOGUS");
    }

    #[test]
    #[should_panic(expected = "unknown utf8proc indic conjunct break")]
    fn indic_conjunct_break_rejects_unknown() {
        let _ = indic_conjunct_break_value("UTF8PROC_INDIC_CONJUNCT_BREAK_BOGUS");
    }

    #[test]
    fn builds_flattened_nfd_tables() {
        let tables = tables();
        let entry = tables
            .nfd_entries
            .iter()
            .find(|entry| entry.codepoint == 'é' as u32)
            .expect("NFD entry for e acute");
        let start = entry.sequence_offset as usize;
        let end = start + entry.sequence_length as usize;
        assert_eq!(&tables.nfd_sequences[start..end], &['e' as u32, 0x0301]);

        let hangul = tables
            .nfd_entries
            .iter()
            .find(|entry| entry.codepoint == '가' as u32)
            .expect("NFD entry for Hangul syllable");
        let start = hangul.sequence_offset as usize;
        let end = start + hangul.sequence_length as usize;
        assert_eq!(&tables.nfd_sequences[start..end], &[0x1100, 0x1161]);
    }
}
