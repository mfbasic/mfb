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

const MODULE_INTRO: &str = r#"Compress and decompress gzip, zlib and raw DEFLATE data, and compute CRC-32 checksums, in MFBASIC with no system library"#;
const MODULE_DESC: &str = r#"The `compress` package works on data held in memory: every member takes a whole
`List OF Byte` and returns its result in one call. It is a built-in package, so
`IMPORT compress` needs no manifest dependency.

Three members decompress the formats built on DEFLATE, and they differ only in the
wrapper around the data:

- `compress::gzipDecode` — gzip: `.gz` files and gzip-encoded HTTP responses. It reads
  every member of a multi-member file.
- `compress::zlibDecode` — zlib: HTTP's `deflate` encoding and the data inside a PNG.
- `compress::inflate` — raw DEFLATE with no wrapper, as stored inside zip entries.

Each refuses data that is not valid with `ErrInvalidFormat`, accepting exactly what
zlib's own decoder accepts; checks the checksum the format carries (you can skip that
comparison with `ignoreChecksum`); and stops with `ErrTooLarge` rather than build a
result larger than `maxBytes`, which defaults to 64 MiB. Bytes after the end of the data
are ignored.

Three members compress into the same three formats:

- `compress::gzipEncode` — one gzip member, with no file name or time in its header.
- `compress::zlibEncode` — zlib data.
- `compress::deflate` — raw DEFLATE with no wrapper.

Each takes a `level` from `0` (store without compressing, fastest) to `9` (search hardest),
defaulting to `6`. Any zlib-compatible decoder reads the result, and the same input and
level give the same bytes on every platform, though not the bytes zlib itself would write.

`compress::crc32` computes the CRC-32 checksum that gzip, zip and PNG store
alongside their data, and can continue a checksum across data that arrives in
pieces.

The package is written in MFBASIC and calls no system compression library, so a
program gets the same results on macOS, Linux and Windows, and a program that
imports `compress` carries only the members it calls."#;

// Man-page + spec citation anchor: `COMPRESS`. The `compress/*` man pages and
// `mfb spec stdlib compress` ground their surface facts in this descriptor with
// `[[src/codegen/builtins/compress/mod.rs:COMPRESS]]`.

/// Synthetic path/doc labels for the `compress` companion a late pass injects.
const SOURCE_LABEL: &str = "<builtin-compress>";
const SOURCE_DOC: &str = "builtins/compress.mfb";

/// Inject `compress`'s gated helpers when a program — **or another injected built-in**
/// — calls the members that need them.
///
/// `canvas`'s injected PNG decoder inflates through `compress::zlibDecode` (plan-137-C).
/// A canvas program writes only `IMPORT canvas`, so the generic
/// `registry::augment_project`, which examines the pre-injection AST, neither sees the
/// `compress` import nor opens the `WhenUsed` gates of the helpers `zlibDecode` rewrites
/// onto. This pass runs after it and sees canvas's companion. Unlike `color`'s, it is not
/// skipped by the generic pass: a program that imports `compress` itself keeps the helpers
/// the generic pass injects, in the same position, and this pass adds only the ones the
/// injected source newly reaches (`registry::late_pass_files`).
pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    crate::codegen::registry::inject_late_pass(ast, "compress", SOURCE_LABEL, SOURCE_DOC)
}

/// The same injection onto the elaborated project the former source checker consumes.
#[cfg(test)] // the HIR-domain chain serves the in-process tests only (plan-107-D)
pub(crate) fn augmented_hir_project(
    hir: &crate::hir::HirProject,
) -> Result<crate::hir::HirProject, ()> {
    crate::codegen::registry::inject_late_pass_hir(hir, "compress", SOURCE_LABEL, SOURCE_DOC)
}

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
    helper_end_position::register(&mut pkg);
    helper_inflate::register(&mut pkg);
    // Framing (gated per decoder): Adler-32 and the zlib wrapper, then the gzip wrapper.
    helper_adler32::register(&mut pkg);
    helper_zlib_frame::register(&mut pkg);
    helper_gzip_frame::register(&mut pkg);
    // DEFLATE encoding (gated on the encoders): the fixed Huffman codes, the encoder core, then
    // the `deflate` wrapper over it.
    helper_deflate_codes::register(&mut pkg);
    // Dynamic Huffman blocks (plan-137-E): package-merge code lengths and canonical codes, the
    // block header, and the per-block cost of each encoding.
    helper_package_merge::register(&mut pkg);
    helper_dynamic_block::register(&mut pkg);
    helper_block_cost::register(&mut pkg);
    helper_deflate_core::register(&mut pkg);
    helper_deflate::register(&mut pkg);
    // Encoder framing (gated per encoder): the zlib wrapper, then the gzip wrapper.
    helper_zlib_encode::register(&mut pkg);
    helper_gzip_encode::register(&mut pkg);

    func_crc32::register(&mut pkg);
    func_inflate::register(&mut pkg);
    func_zlib_decode::register(&mut pkg);
    func_gzip_decode::register(&mut pkg);
    func_deflate::register(&mut pkg);
    func_zlib_encode::register(&mut pkg);
    func_gzip_encode::register(&mut pkg);

    r.add_package(pkg);
}

mod func_crc32;
mod func_deflate;
mod func_gzip_decode;
mod func_gzip_encode;
mod func_inflate;
mod func_zlib_decode;
mod func_zlib_encode;
mod helper_adler32;
mod helper_block_cost;
mod helper_crc32;
mod helper_crc32_table;
mod helper_deflate;
mod helper_deflate_codes;
mod helper_deflate_core;
mod helper_deflate_tables;
mod helper_dynamic_block;
mod helper_end_position;
mod helper_gzip_encode;
mod helper_gzip_frame;
mod helper_huffman_table;
mod helper_inflate;
mod helper_inflate_core;
mod helper_package_merge;
mod helper_zlib_encode;
mod helper_zlib_frame;

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
        // 7 members: `crc32`, `inflate`, `zlibDecode`, `gzipDecode`, `deflate`, `zlibEncode`,
        // `gzipEncode`.
        assert_eq!(pkg.functions().len(), 7);
    }

    /// RFC 1951 §3.2.6, transcribed from the RFC text fetched for plan-137-B: the fixed
    /// literal/length code as `(first symbol, bits, first code)` per range — 0–143 are 8 bits from
    /// `00110000`, 144–255 are 9 bits from `110010000`, 256–279 are 7 bits from `0000000`, 280–287
    /// are 8 bits from `11000000`. The ranges must describe a complete code (Kraft sum 1), and the
    /// encoder's table builder must spell each range's boundary, start code and length.
    #[test]
    fn fixed_code_builder_matches_rfc1951() {
        const RANGES: [(u32, u32, u32); 4] = [
            (0, 8, 0b0011_0000),
            (144, 9, 0b1_1001_0000),
            (256, 7, 0b000_0000),
            (280, 8, 0b1100_0000),
        ];
        let ends = [144u32, 256, 280, 288];
        let kraft: f64 = RANGES
            .iter()
            .zip(ends)
            .map(|((first, bits, _), end)| f64::from(end - first) / f64::from(1u32 << bits))
            .sum();
        assert_eq!(kraft, 1.0);

        let body = super::helper_deflate_codes::BODY;
        for ((first, bits, code), end) in RANGES.iter().zip(ends) {
            let assign = match (*code, *first) {
                (0, f) => format!("code = sym - {f}"),
                (c, 0) => format!("code = {c} + sym"),
                (c, f) => format!("code = {c} + sym - {f}"),
            };
            assert!(
                body.contains(&format!("{assign}\n      length = {bits}")),
                "range starting at {first}: {assign}, {bits} bits"
            );
            if end < 288 {
                assert!(body.contains(&format!("sym < {end} THEN")), "boundary {end}");
            }
        }
        assert!(body.contains("WHILE sym < 288"));
    }

    /// The paths of every file the build's augmentation chain leaves in a one-file project.
    fn injected_paths(source: &str) -> Vec<String> {
        let project = crate::testutil::project_from_src(source);
        crate::resolver::augment_project(&project, false)
            .expect("builtin sources parse")
            .files
            .into_iter()
            .map(|file| file.path)
            .collect()
    }

    fn count(paths: &[String], path: &str) -> usize {
        paths.iter().filter(|p| p.as_str() == path).count()
    }

    /// plan-137-C: canvas's injected PNG decoder calls `compress::zlibDecode`. A program that
    /// imports only `canvas` gets the helper it rewrites onto from `compress`'s late pass —
    /// the generic pass never sees that call — and only the helpers that call reaches.
    #[test]
    fn a_canvas_program_gets_the_zlib_decoder_without_importing_compress() {
        let paths = injected_paths("IMPORT canvas\n\nSUB main()\nEND SUB\n");
        assert_eq!(count(&paths, "builtins/compress_zlib_frame.mfb"), 1);
        assert_eq!(count(&paths, "builtins/compress_crc32.mfb"), 0);
    }

    /// The generic pass injects the checksum helper for the program's own call; the late
    /// pass adds the decoder canvas reaches and does not inject the checksum helper again.
    #[test]
    fn a_program_importing_compress_and_canvas_gets_each_helper_once() {
        let paths = injected_paths(
            "IMPORT canvas\nIMPORT compress\n\nSUB main()\n  LET c AS Integer = compress::crc32([toByte(1)])\nEND SUB\n",
        );
        assert_eq!(count(&paths, "builtins/compress_crc32.mfb"), 1);
        assert_eq!(count(&paths, "builtins/compress_zlib_frame.mfb"), 1);
    }

    /// Without canvas, a program that only checksums carries no decoder.
    #[test]
    fn a_compress_program_that_only_checksums_carries_no_decoder() {
        let paths = injected_paths(
            "IMPORT compress\n\nSUB main()\n  LET c AS Integer = compress::crc32([toByte(1)])\nEND SUB\n",
        );
        assert_eq!(count(&paths, "builtins/compress_crc32.mfb"), 1);
        assert_eq!(count(&paths, "builtins/compress_zlib_frame.mfb"), 0);
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
