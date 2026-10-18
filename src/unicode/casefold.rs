//! Unicode full case folding (the C and F mappings of CaseFolding.txt), read from
//! the generated `casefold_cf.txt` (`tools/unicode-tables/gen_casefold_table.py`
//! over the vendored `third_party/unicode/CaseFolding-16.0.0.txt`). The one source
//! for both the compile-time fold (`backend::case_fold`) and the runtime casefold
//! tables (`runtime_tables`), so the two cannot disagree.
use std::collections::HashMap;
use std::sync::OnceLock;

const DATA: &str = include_str!("casefold_cf.txt");

fn table() -> &'static HashMap<char, Vec<char>> {
    static TABLE: OnceLock<HashMap<char, Vec<char>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let scalar = |hex: &str| {
            char::from_u32(u32::from_str_radix(hex, 16).expect("casefold hex scalar"))
                .expect("casefold scalar")
        };
        DATA.lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .map(|line| {
                let (from, to) = line.split_once(';').expect("casefold line");
                (scalar(from), to.split(' ').map(scalar).collect())
            })
            .collect()
    })
}

/// The full case folding of one scalar (the scalar itself when it has none).
pub(crate) fn fold_char(ch: char) -> impl Iterator<Item = char> {
    match table().get(&ch) {
        Some(folded) => folded.clone().into_iter(),
        None => vec![ch].into_iter(),
    }
}
