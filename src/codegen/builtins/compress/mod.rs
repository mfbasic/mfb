//! Built-in `compress::` package — in-memory gzip / zlib / raw DEFLATE and the
//! checksums those formats carry, on the clean-room registry
//! (`crate::codegen::registry`).
//!
//! Every member is MFBASIC source over `bits` and `collections`: a public member
//! rewrites onto its `__compress_*` body via [`Body::Rewrite`], and each body lives in
//! a `helper_*.rs` registered with `add_helper`. There is no `Body::abi_function` /
//! `abi_inline` member, no runtime-helper family, no `dlopen` and no vendored
//! library, so the package produces the same bytes and the same errors on every
//! target (plan-137-A §1 non-goals).
//!
//! Every helper is gated [`HelperGate::WhenUsed`] on the members that need it, so a
//! program that writes `IMPORT compress` pays only for the members it calls.
//!
//! [`HelperGate::WhenUsed`]: crate::codegen::registry::HelperGate::WhenUsed

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, Registry, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const MODULE_INTRO: &str = r#"Checksums for compressed data formats, computed in MFBASIC with no system library"#;
const MODULE_DESC: &str = r#"The `compress` package works on data held in memory: every member takes a whole
`List OF Byte` and returns its result in one call. It is a built-in package, so
`IMPORT compress` needs no manifest dependency.

`compress::crc32` computes the CRC-32 checksum that gzip, zip and PNG store
alongside their data, and can continue a checksum across data that arrives in
pieces.

The package is written in MFBASIC and calls no system compression library, so a
program gets the same results on macOS, Linux and Windows, and a program that
imports `compress` carries only the members it calls."#;

// Man-page + spec citation anchor: `COMPRESS`. The `compress/*` man pages and
// `mfb spec stdlib compress` ground their surface facts in this descriptor with
// `[[src/codegen/builtins/compress/mod.rs:COMPRESS]]`.

/// Register the `compress` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("compress", MODULE_INTRO, MODULE_DESC);

    // Injected `IMPORT`s, rendered before the helpers.
    pkg.add_imports(vec!["compress", "bits", "collections"]);

    // CRC-32 (gated on `crc32`): the table first, then the loop that reads it.
    helper_crc32_table::register(&mut pkg);
    helper_crc32::register(&mut pkg);

    // DEFLATE decoding (gated on the decoders): the RFC 1951 length/distance tables, the
    // Huffman table builder, the decoder core, then the `inflate` wrapper over it.
    helper_deflate_tables::register(&mut pkg);
    helper_huffman_table::register(&mut pkg);
    helper_inflate_core::register(&mut pkg);
    helper_inflate::register(&mut pkg);

    func_crc32::register(&mut pkg);
    func_inflate::register(&mut pkg);

    r.add_package(pkg);
}

mod func_crc32;
mod func_inflate;
mod helper_crc32;
mod helper_crc32_table;
mod helper_deflate_tables;
mod helper_huffman_table;
mod helper_inflate;
mod helper_inflate_core;

/// `List OF Byte` — the pervasive `compress` argument/return type.
fn bytes() -> ParameterType {
    ParameterType::list_of(ParameterType::Byte)
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;

    #[test]
    fn compress_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("compress")
            .expect("compress package");
        // 2 members: `crc32`, `inflate`.
        assert_eq!(pkg.functions().len(), 2);
    }

    /// The one magic number in the table builder is the reflected CRC-32/ISO-HDLC
    /// polynomial. It is derived here from the catalogue's normal form (CRC RevEng:
    /// `poly=0x04c11db7 refin=true`, fetched 2026-09-14) rather than restated, and the
    /// builder must spell exactly that decimal value. The table entries it produces are
    /// judged against independent implementations by `tests/interop/rt_compress_interop.rs`.
    #[test]
    fn crc32_table_builder_uses_the_reflected_polynomial() {
        let reflected = 0x04C1_1DB7u32.reverse_bits();
        let body = super::helper_crc32_table::BODY;
        assert!(body.contains(&format!("c = bits::bxor(bits::sr(c, 1), {reflected})")));
        // 256 byte-table entries, then 7 x 256 derived entries.
        assert!(body.contains("WHILE i < 256") && body.contains("WHILE i < 2048"));
    }

    /// RFC 1951 §3.2.5, transcribed from the fetched text (rfc-editor.org/rfc/rfc1951.txt,
    /// 2026-09-14): `(base, extra bits, last value)` for length codes 257..285 and distance
    /// codes 0..29. Code 284's range is printed as 227-257, not 227-258: length 258 has its
    /// own code, 285.
    const RFC1951_LENGTHS: [(u32, u32, u32); 29] = [
        (3, 0, 3), (4, 0, 4), (5, 0, 5), (6, 0, 6), (7, 0, 7), (8, 0, 8), (9, 0, 9), (10, 0, 10),
        (11, 1, 12), (13, 1, 14), (15, 1, 16), (17, 1, 18), (19, 2, 22), (23, 2, 26), (27, 2, 30),
        (31, 2, 34), (35, 3, 42), (43, 3, 50), (51, 3, 58), (59, 3, 66), (67, 4, 82), (83, 4, 98),
        (99, 4, 114), (115, 4, 130), (131, 5, 162), (163, 5, 194), (195, 5, 226), (227, 5, 257),
        (258, 0, 258),
    ];
    const RFC1951_DISTANCES: [(u32, u32, u32); 30] = [
        (1, 0, 1), (2, 0, 2), (3, 0, 3), (4, 0, 4), (5, 1, 6), (7, 1, 8), (9, 2, 12), (13, 2, 16),
        (17, 3, 24), (25, 3, 32), (33, 4, 48), (49, 4, 64), (65, 5, 96), (97, 5, 128),
        (129, 6, 192), (193, 6, 256), (257, 7, 384), (385, 7, 512), (513, 8, 768), (769, 8, 1024),
        (1025, 9, 1536), (1537, 9, 2048), (2049, 10, 3072), (3073, 10, 4096), (4097, 11, 6144),
        (6145, 11, 8192), (8193, 12, 12288), (12289, 12, 16384), (16385, 13, 24576),
        (24577, 13, 32768),
    ];

    /// The builder in `helper_deflate_tables.rs` is MFBASIC, which a unit test cannot run, so
    /// this pins it two ways: the formula it spells, recomputed here, must reproduce the RFC
    /// table row for row; and the helper body must spell exactly that formula's constants.
    #[test]
    fn deflate_tables_builder_matches_rfc1951() {
        let mut lengths = Vec::new();
        let mut base = 3u32;
        for i in 0..28u32 {
            let extra = if i >= 8 { (i - 4) / 4 } else { 0 };
            lengths.push((base, extra));
            base += 1 << extra;
        }
        lengths.push((258, 0));
        for (i, ((b, e), (rb, re, last))) in lengths.iter().zip(RFC1951_LENGTHS).enumerate() {
            assert_eq!((*b, *e), (rb, re), "length code {}", 257 + i);
            // Every code covers base..base + 2^extra - 1, except 284, which stops at 257.
            let top = if i == 27 { last + 1 } else { last };
            assert_eq!(b + (1 << e) - 1, top, "length code {} range", 257 + i);
        }

        let mut distances = Vec::new();
        let mut base = 1u32;
        for i in 0..30u32 {
            let extra = if i >= 4 { (i - 2) / 2 } else { 0 };
            distances.push((base, extra));
            base += 1 << extra;
        }
        for (i, ((b, e), (rb, re, last))) in distances.iter().zip(RFC1951_DISTANCES).enumerate() {
            assert_eq!((*b, *e), (rb, re), "distance code {i}");
            assert_eq!(b + (1 << e) - 1, last, "distance code {i} range");
        }

        let body = super::helper_deflate_tables::BODY;
        for spelled in [
            "MUT base AS Integer = 3",
            "WHILE i < 28",
            "IF i >= 8 THEN",
            "extra = (i - 4) / 4",
            "t = collections::append(t, 258)",
            "MUT base AS Integer = 1",
            "WHILE i < 30",
            "IF i >= 4 THEN",
            "extra = (i - 2) / 2",
            "base = base + bits::sl(1, extra)",
        ] {
            assert!(body.contains(spelled), "helper_deflate_tables body no longer spells `{spelled}`");
        }
    }
}
