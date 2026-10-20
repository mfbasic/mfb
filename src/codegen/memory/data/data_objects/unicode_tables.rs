//! plan-176-P: the Unicode runtime tables a program references. `string_symbols`, the error-message data and the raw/string data
//! objects live in `memory/data/data_objects/mod.rs`.

use super::*;

pub(crate) fn unicode_string_call_is_static(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> bool {
    matches!(
        target,
        "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.graphemes"
    ) && args.len() == 1
        && static_string_value_with_constants(&args[0], constants, types, fields).is_some()
}

/// The unicode runtime tables to emit. `referenced` is the set of
/// `_mfb_unicode_*` data symbols that some generated function actually relocates
/// against — the ground truth for which tables are live (plan-77 U5). Only those
/// tables are emitted, so e.g. a `strings::graphemes`-only program drops the six
/// case-mapping tables and the NFD/composition tables it never touches. `None`
/// emits every table (the coarse fallback for the — practically unreachable —
/// case where the NIR heuristic reports unicode use but no relocation names a
/// specific table, preserving the pre-split all-or-nothing behaviour rather than
/// risking an undefined symbol).
pub(crate) fn unicode_runtime_data_objects(
    referenced: Option<&std::collections::HashSet<&str>>,
) -> Vec<CodeDataObject> {
    let tables = crate::unicode::runtime_tables::tables();
    let keep = |symbol: &str| referenced.is_none_or(|set| set.contains(symbol));
    let mut objects = Vec::new();
    if keep(UNICODE_STAGE1_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_STAGE1_SYMBOL,
            "u16 utf8proc stage1 property index table",
            tables.stage1.len() * 2,
            crate::unicode::runtime_tables::stage1_hex(),
            2,
        ));
    }
    if keep(UNICODE_STAGE2_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_STAGE2_SYMBOL,
            "u16 utf8proc stage2 property index table",
            tables.stage2.len() * 2,
            crate::unicode::runtime_tables::stage2_hex(),
            2,
        ));
    }
    if keep(UNICODE_PROPERTIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_PROPERTIES_SYMBOL,
            "mfb.unicode.property.v1 records, 12 bytes each",
            tables.properties.len() * 12,
            crate::unicode::runtime_tables::properties_hex(),
            2,
        ));
    }
    // plan-118-B: the general-category and Script run tables. Unlike the utf8proc
    // objects above these are OUR generated artifacts, pinned to Unicode 16.0.0
    // independently of the vendored library (see `unicode::range_tables` for the
    // 4,804-scalar disagreement that makes them separate tables and not a field
    // on the trie).
    for (runs_symbol, names_symbol, table, property) in [
        (
            UNICODE_GENCAT_RANGES_SYMBOL,
            UNICODE_GENCAT_NAMES_SYMBOL,
            crate::unicode::range_tables::gencat(),
            "general category",
        ),
        (
            UNICODE_SCRIPT_RANGES_SYMBOL,
            UNICODE_SCRIPT_NAMES_SYMBOL,
            crate::unicode::range_tables::script(),
            "Script",
        ),
    ] {
        if keep(runs_symbol) {
            objects.push(raw_data_object(
                runs_symbol,
                &format!(
                    "mfb.unicode.range.v1 {{ u32 lastCodepoint; u32 nameIndex }} ({property})"
                ),
                table.runs.len() * 8,
                table.runs_hex(),
                4,
            ));
        }
        if keep(names_symbol) {
            objects.push(raw_data_object(
                names_symbol,
                &format!(
                    "mfb.string.v1 records, {} bytes each ({property} names)",
                    table.name_stride()
                ),
                table.names.len() * table.name_stride(),
                table.names_hex(),
                8,
            ));
        }
    }
    // plan-162-E: the Line_Break runs carry their value, so they are the one object.
    if keep(UNICODE_LINEBREAK_RANGES_SYMBOL) {
        let table = crate::unicode::range_tables::linebreak();
        objects.push(raw_data_object(
            UNICODE_LINEBREAK_RANGES_SYMBOL,
            "mfb.unicode.range.v1 { u32 lastCodepoint; u32 value } (UAX #14 Line_Break)",
            table.runs.len() * 8,
            table.values_hex(),
            4,
        ));
    }
    if keep(UNICODE_COMBINATIONS_SECOND_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_COMBINATIONS_SECOND_SYMBOL,
            "u32 utf8proc composition second codepoint table",
            tables.combinations_second.len() * 4,
            crate::unicode::runtime_tables::combinations_second_hex(),
            4,
        ));
    }
    if keep(UNICODE_COMBINATIONS_COMBINED_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_COMBINATIONS_COMBINED_SYMBOL,
            "u32 utf8proc composition combined codepoint table",
            tables.combinations_combined.len() * 4,
            crate::unicode::runtime_tables::combinations_combined_hex(),
            4,
        ));
    }
    if keep(UNICODE_NFD_ENTRIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_NFD_ENTRIES_SYMBOL,
            "mfb.unicode.nfd_entry.v1 records, 16 bytes each",
            tables.nfd_entries.len() * 16,
            crate::unicode::runtime_tables::nfd_entries_hex(),
            4,
        ));
    }
    if keep(UNICODE_NFD_SEQUENCES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_NFD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode NFD sequence table",
            tables.nfd_sequences.len() * 4,
            crate::unicode::runtime_tables::nfd_sequences_hex(),
            4,
        ));
    }
    if keep(UNICODE_UPPERCASE_ENTRIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_UPPERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 uppercase records, 16 bytes each",
            tables.uppercase_entries.len() * 16,
            crate::unicode::runtime_tables::uppercase_entries_hex(),
            4,
        ));
    }
    if keep(UNICODE_UPPERCASE_SEQUENCES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_UPPERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode uppercase sequence table",
            tables.uppercase_sequences.len() * 4,
            crate::unicode::runtime_tables::uppercase_sequences_hex(),
            4,
        ));
    }
    if keep(UNICODE_LOWERCASE_ENTRIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_LOWERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 lowercase records, 16 bytes each",
            tables.lowercase_entries.len() * 16,
            crate::unicode::runtime_tables::lowercase_entries_hex(),
            4,
        ));
    }
    if keep(UNICODE_LOWERCASE_SEQUENCES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_LOWERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode lowercase sequence table",
            tables.lowercase_sequences.len() * 4,
            crate::unicode::runtime_tables::lowercase_sequences_hex(),
            4,
        ));
    }
    if keep(UNICODE_CASEFOLD_ENTRIES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_CASEFOLD_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 casefold records, 16 bytes each",
            tables.casefold_entries.len() * 16,
            crate::unicode::runtime_tables::casefold_entries_hex(),
            4,
        ));
    }
    if keep(UNICODE_CASEFOLD_SEQUENCES_SYMBOL) {
        objects.push(raw_data_object(
            UNICODE_CASEFOLD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode casefold sequence table",
            tables.casefold_sequences.len() * 4,
            crate::unicode::runtime_tables::casefold_sequences_hex(),
            4,
        ));
    }
    objects
}
