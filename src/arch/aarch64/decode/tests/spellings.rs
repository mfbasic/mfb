//! Words the decoder once spelled differently from the LLVM disassembler that lldb
//! uses (or took for another instruction); each row is the text lldb prints.

use super::decode;

#[test]
fn words_lldb_spells_differently_from_their_neighbours_decode_as_it_does() {
    for (word, text) in [
        // `bfc` is `bfm` from the zero register, including a lsb of 0.
        (0x3300_1fe0, "bfc    w0, #0, #8"),
        (0x3305_0fe0, "bfc    w0, #27, #4"),
        (0x3300_7fff, "bfc    wzr, #0, #32"),
        (0x3301_3fe0, "bfxil  w0, wzr, #1, #15"),
        // SM3PARTW1/2 and SM4EKEY sit beside SHA-512 and are not it.
        (0xce62_c020, "sm3partw1.4s v0, v1, v2"),
        (0xce62_c420, "sm3partw2.4s v0, v1, v2"),
        (0xce61_c801, "sm4ekey.4s v1, v0, v1"),
        (0xce62_8020, "sha512h.2d q0, q1, v2"),
        // The register-offset prefetch with a `11xxx` operation is `rprfm`.
        (0xf8af_ca9f, "rprfm  #0x27, x15, [x20]"),
        (0xf8bf_4a9f, "rprfm  #0x7, xzr, [x20]"),
        (0xf8a0_7a9f, "rprfm  #0x1f, x0, [x20]"),
        (0xf8a0_ca17, "prfm   pstslcstrm, [x16, w0, sxtw]"),
        // Barrier operands are decimal when they have no name; `dsb #12` is `dfb`.
        (0xd503_33df, "isb    #3"),
        (0xd503_30bf, "dmb    #0"),
        (0xd503_389f, "dsb    #8"),
        (0xd503_3c9f, "dfb"),
        // Hints LLVM names.
        (0xd503_227f, "gcsb   dsync"),
        (0xd503_22df, "clrbhb"),
        (0xd503_24ff, "pacm"),
        (0xd503_251f, "chkfeat x16"),
    ] {
        let decoded = decode(word, 0x1_0000_1000).map(|instruction| instruction.text);
        assert_eq!(decoded.as_deref(), Some(text), "{word:#010x}");
    }
}
