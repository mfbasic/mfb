#![allow(dead_code)]

use std::sync::OnceLock;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const UTF8PROC_DATA: &str = include_str!("../third_party/utf8proc/utf8proc_data.c");
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

pub(crate) fn property_for_codepoint(codepoint: u32) -> PackedProperty {
    let tables = tables();
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
    let marker = format!("static const ");
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
        _ if value.starts_with("UTF8PROC_CATEGORY_") => category_value(value) as i64,
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

fn category_value(value: &str) -> u16 {
    match value {
        "UTF8PROC_CATEGORY_CN" => 0,
        "UTF8PROC_CATEGORY_LU" => 1,
        "UTF8PROC_CATEGORY_LL" => 2,
        "UTF8PROC_CATEGORY_LT" => 3,
        "UTF8PROC_CATEGORY_LM" => 4,
        "UTF8PROC_CATEGORY_LO" => 5,
        "UTF8PROC_CATEGORY_MN" => 6,
        "UTF8PROC_CATEGORY_MC" => 7,
        "UTF8PROC_CATEGORY_ME" => 8,
        "UTF8PROC_CATEGORY_ND" => 9,
        "UTF8PROC_CATEGORY_NL" => 10,
        "UTF8PROC_CATEGORY_NO" => 11,
        "UTF8PROC_CATEGORY_PC" => 12,
        "UTF8PROC_CATEGORY_PD" => 13,
        "UTF8PROC_CATEGORY_PS" => 14,
        "UTF8PROC_CATEGORY_PE" => 15,
        "UTF8PROC_CATEGORY_PI" => 16,
        "UTF8PROC_CATEGORY_PF" => 17,
        "UTF8PROC_CATEGORY_PO" => 18,
        "UTF8PROC_CATEGORY_SM" => 19,
        "UTF8PROC_CATEGORY_SC" => 20,
        "UTF8PROC_CATEGORY_SK" => 21,
        "UTF8PROC_CATEGORY_SO" => 22,
        "UTF8PROC_CATEGORY_ZS" => 23,
        "UTF8PROC_CATEGORY_ZL" => 24,
        "UTF8PROC_CATEGORY_ZP" => 25,
        "UTF8PROC_CATEGORY_CC" => 26,
        "UTF8PROC_CATEGORY_CF" => 27,
        "UTF8PROC_CATEGORY_CS" => 28,
        "UTF8PROC_CATEGORY_CO" => 29,
        other => panic!("unknown utf8proc category `{other}`"),
    }
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
