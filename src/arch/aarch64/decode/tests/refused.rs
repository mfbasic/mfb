//! System-space words that name no instruction the decoder knows decode to
//! `None` (the caller prints them as raw words) rather than as a guessed
//! mnemonic.

use super::super::decode;
use super::PC;

#[test]
fn unnamed_system_space_words_are_not_decoded() {
    for (word, what) in [
        (
            0xD503_501F_u32,
            "op0=0 CRn=0101 with Rt=xzr: no hint/barrier/pstate",
        ),
        (0xD503_2000, "a hint-shaped word whose Rt is not xzr"),
        (
            0xD501_401F,
            "msr of an unassigned pstate field (op1=1, op2=0)",
        ),
        (0xD508_8000, "sys with CRn=8: no cache operation"),
        (0xD508_7500, "ic iallu naming a register it does not take"),
    ] {
        assert!(
            decode(word, PC).is_none(),
            "{word:#010x} ({what}) decoded as {:?}",
            decode(word, PC).map(|decoded| decoded.text)
        );
    }
    // The same encodings with the fields corrected do decode.
    assert_eq!(
        decode(0xD503_201F, PC).map(|d| d.text).as_deref(),
        Some("nop")
    );
    assert_eq!(
        decode(0xD508_751F, PC).map(|d| d.text).as_deref(),
        Some("ic     iallu")
    );
}
