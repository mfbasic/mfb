//! The decoder held to lldb over words the encoder never writes.
//!
//! `swept_words.txt` was produced by `tools/oracles/aarch64-decode` (`sweep.py`
//! writes candidate words, `lldb-corpus.sh` has lldb disassemble them, `pick.py`
//! keeps one row per mnemonic and operand shape the sample did not have, plus
//! the words lldb refuses). The candidates are every word within two flipped
//! bits of a row of `lldb_words.txt`, and the whole system, branch-register and
//! exception classes. They reach the scalar and vector arms, the load/store
//! forms and the system instructions the three-per-mnemonic sample cannot, and
//! the refusals each decoder arm makes for a malformed word.
//!
//! Rows the decoder spells differently are not here: where lldb knows a system
//! register the decoder only prints as `S<op0>_<op1>_C<n>_C<m>_<op2>`, where the
//! decoder is more lenient than LLVM about an encoding's should-be-zero fields,
//! and the extensions the decoder does not know (SVE, SME, MTE, FEAT_LUT, ...).

use super::{decode, lldb_rows};

/// Every row of `listing` the decoder does not spell as lldb does.
fn disagreements(listing: &str) -> Vec<String> {
    lldb_rows(listing)
        .filter_map(|(pc, word, text)| {
            let decoded = decode(word, pc).map(|instruction| instruction.text);
            // A word lldb will not disassemble is one the decoder must refuse.
            let expected = (!text.starts_with(".long")).then_some(text);
            (decoded.as_deref() != expected).then(|| match expected {
                Some(text) => format!("{word:#010x}: lldb `{text}`, decoded {decoded:?}"),
                None => format!("{word:#010x}: lldb refuses it, decoded {decoded:?}"),
            })
        })
        .collect()
}

#[test]
fn decodes_swept_words_as_lldb_does() {
    let listing = include_str!("swept_words.txt");
    let refused = lldb_rows(listing)
        .filter(|(_, _, text)| text.starts_with(".long"))
        .count();
    assert!(refused > 1000, "the sweep keeps its refused-word rows");
    let wrong = disagreements(listing);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A candidate listing from `tools/oracles/aarch64-decode`, every disagreement
/// printed (the decoder's refusals of words lldb knows included) so a sweep can
/// be pruned to `swept_words.txt`.
#[test]
#[ignore = "needs a listing from tools/oracles/aarch64-decode"]
fn lists_a_sweeps_disagreements_with_lldb() {
    let path = std::env::var("DECODE_CORPUS").expect("DECODE_CORPUS names a listing");
    let listing = std::fs::read_to_string(path).expect("the listing reads");
    for line in disagreements(&listing) {
        println!("{line}");
    }
}
