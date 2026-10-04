//! The format-neutral symbol list a `--debug` build writes into its native
//! symbol table (plan-166): the Mach-O `LC_SYMTAB`, the ELF `.symtab`, and the
//! PE COFF symbol table all list exactly these entries, in this order.
//!
//! Every linker already receives the names in `EncodedImage.symbols` to resolve
//! relocations. This module only orders them and sizes them; each writer maps
//! `(section, offset)` to its own address with the rule `symbol_vmaddr` uses.
//! Names are written verbatim (the native `_mfb_fn_…` spelling `--ncode` shows),
//! so they are unique by construction and are what a developer types into a
//! debugger.

use crate::arch::image::{EncodedImage, EncodedSection};
use std::collections::HashSet;

/// What a debug symbol names: code in the text section, or a data object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DebugSymbolKind {
    Function,
    Object,
}

/// One symbol-table entry, located by section and offset within that section.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DebugSymbol<'a> {
    pub(crate) name: &'a str,
    pub(crate) section: EncodedSection,
    pub(crate) offset: usize,
    pub(crate) size: usize,
    pub(crate) kind: DebugSymbolKind,
}

/// `image.symbols` ordered by (section, offset, name), each sized to the next
/// distinct offset in its section (the last to `image.text.len()` /
/// `image.data.len()`). Text → Function, Data → Object.
///
/// Symbols that share an offset all get the size up to the next *distinct*
/// offset, so an alias is never reported as zero-sized. A data object's size
/// includes any alignment padding before the next object.
///
/// Refuses a duplicate name (two entries for one name would make every tool's
/// lookup ambiguous) and an offset past its section's end (it would point at
/// linker-appended stubs or past the image).
pub(crate) fn debug_symbols(image: &EncodedImage) -> Result<Vec<DebugSymbol<'_>>, String> {
    let mut seen = HashSet::new();
    for symbol in &image.symbols {
        if !seen.insert(symbol.name.as_str()) {
            return Err(format!("debug symbols: duplicate symbol '{}'", symbol.name));
        }
        let end = section_len(image, symbol.section);
        if symbol.offset > end {
            return Err(format!(
                "debug symbols: '{}' at offset {} is past the {} section end {end}",
                symbol.name,
                symbol.offset,
                section_name(symbol.section)
            ));
        }
    }
    let mut ordered: Vec<_> = image.symbols.iter().collect();
    ordered.sort_by(|a, b| {
        section_rank(a.section)
            .cmp(&section_rank(b.section))
            .then(a.offset.cmp(&b.offset))
            .then(a.name.cmp(&b.name))
    });
    let mut symbols = Vec::with_capacity(ordered.len());
    for (index, symbol) in ordered.iter().enumerate() {
        let next = ordered[index + 1..]
            .iter()
            .find(|next| next.section != symbol.section || next.offset != symbol.offset);
        let end = match next {
            Some(next) if next.section == symbol.section => next.offset,
            _ => section_len(image, symbol.section),
        };
        symbols.push(DebugSymbol {
            name: &symbol.name,
            section: symbol.section,
            offset: symbol.offset,
            size: end - symbol.offset,
            kind: match symbol.section {
                EncodedSection::Text => DebugSymbolKind::Function,
                EncodedSection::Data => DebugSymbolKind::Object,
            },
        });
    }
    Ok(symbols)
}

fn section_len(image: &EncodedImage, section: EncodedSection) -> usize {
    match section {
        EncodedSection::Text => image.text.len(),
        EncodedSection::Data => image.data.len(),
    }
}

fn section_rank(section: EncodedSection) -> u8 {
    match section {
        EncodedSection::Text => 0,
        EncodedSection::Data => 1,
    }
}

fn section_name(section: EncodedSection) -> &'static str {
    match section {
        EncodedSection::Text => "text",
        EncodedSection::Data => "data",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::image::EncodedSymbol;

    fn image(symbols: &[(&str, EncodedSection, usize)]) -> EncodedImage {
        EncodedImage {
            text: vec![0; 32],
            data: vec![0; 24],
            rodata_size: 0,
            symbols: symbols
                .iter()
                .map(|(name, section, offset)| EncodedSymbol {
                    name: name.to_string(),
                    section: *section,
                    offset: *offset,
                })
                .collect(),
            relocations: Vec::new(),
            imports: Vec::new(),
            entry: "_main".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
            rpaths: Vec::new(),
            debug_symbols: true,
        }
    }

    fn summary(symbols: &[DebugSymbol<'_>]) -> Vec<(String, usize, usize)> {
        symbols
            .iter()
            .map(|symbol| (symbol.name.to_string(), symbol.offset, symbol.size))
            .collect()
    }

    #[test]
    fn sizes_run_to_next_symbol_and_last_to_section_end() {
        // Declared out of order: the list is sorted by (section, offset).
        let img = image(&[
            ("_b", EncodedSection::Text, 12),
            ("_a", EncodedSection::Text, 0),
            ("_d1", EncodedSection::Data, 16),
            ("_d0", EncodedSection::Data, 0),
        ]);
        let symbols = debug_symbols(&img).unwrap();
        assert_eq!(
            summary(&symbols),
            vec![
                ("_a".to_string(), 0, 12),
                ("_b".to_string(), 12, 20),
                ("_d0".to_string(), 0, 16),
                ("_d1".to_string(), 16, 8),
            ]
        );
    }

    #[test]
    fn aliases_share_the_next_distinct_offset() {
        let img = image(&[
            ("_main", EncodedSection::Text, 8),
            ("_alias", EncodedSection::Text, 8),
            ("_first", EncodedSection::Text, 0),
            ("_last", EncodedSection::Text, 20),
        ]);
        let symbols = debug_symbols(&img).unwrap();
        assert_eq!(
            summary(&symbols),
            vec![
                ("_first".to_string(), 0, 8),
                ("_alias".to_string(), 8, 12),
                ("_main".to_string(), 8, 12),
                ("_last".to_string(), 20, 12),
            ]
        );
    }

    #[test]
    fn text_is_function_data_is_object() {
        let img = image(&[
            ("_d", EncodedSection::Data, 0),
            ("_f", EncodedSection::Text, 0),
        ]);
        let symbols = debug_symbols(&img).unwrap();
        assert_eq!(symbols[0].name, "_f");
        assert_eq!(symbols[0].section, EncodedSection::Text);
        assert_eq!(symbols[0].kind, DebugSymbolKind::Function);
        assert_eq!(symbols[1].name, "_d");
        assert_eq!(symbols[1].section, EncodedSection::Data);
        assert_eq!(symbols[1].kind, DebugSymbolKind::Object);
    }

    #[test]
    fn duplicate_name_is_rejected() {
        let img = image(&[
            ("_f", EncodedSection::Text, 0),
            ("_f", EncodedSection::Data, 0),
        ]);
        let err = debug_symbols(&img).unwrap_err();
        assert!(err.contains("duplicate symbol '_f'"), "{err}");
    }

    #[test]
    fn offset_past_section_end_is_rejected() {
        let img = image(&[("_d", EncodedSection::Data, 25)]);
        let err = debug_symbols(&img).unwrap_err();
        assert!(err.contains("past the data section end 24"), "{err}");
        // Exactly at the end is a zero-sized trailing label, not an error.
        let img = image(&[("_end", EncodedSection::Text, 32)]);
        assert_eq!(debug_symbols(&img).unwrap()[0].size, 0);
    }
}
