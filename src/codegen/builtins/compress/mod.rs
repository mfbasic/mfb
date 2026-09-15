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

    func_crc32::register(&mut pkg);

    r.add_package(pkg);
}

mod func_crc32;
mod helper_crc32;
mod helper_crc32_table;

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
        // 1 member: `crc32`.
        assert_eq!(pkg.functions().len(), 1);
    }

    /// The eight slicing-by-8 tables, computed from the reflected polynomial
    /// `0xEDB88320`: table 0 is the byte table, table `k` is table `k-1` advanced by
    /// one zero byte.
    fn crc32_tables() -> Vec<u64> {
        let mut t = vec![0u64; 8 * 256];
        for i in 0..256u64 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            }
            t[i as usize] = c;
        }
        for k in 1..8 {
            for i in 0..256 {
                let prev = t[(k - 1) * 256 + i];
                t[k * 256 + i] = (prev >> 8) ^ t[(prev & 255) as usize];
            }
        }
        t
    }

    #[test]
    fn crc32_tables_literal_matches_the_polynomial() {
        let body = super::helper_crc32_table::BODY;
        let open = body.find('[').expect("table literal opens");
        let close = body.rfind(']').expect("table literal closes");
        let literal: Vec<u64> = body[open + 1..close]
            .split(',')
            // Each row break is a `_` line continuation, which lands on the next entry.
            .map(|entry| entry.replace('_', ""))
            .map(|entry| entry.trim().parse().expect("decimal table entry"))
            .collect();
        assert_eq!(literal.len(), 2048);
        assert_eq!(literal, crc32_tables());
    }

    /// Table 0 against the one value everyone publishes: the CRC-32/ISO-HDLC check
    /// of "123456789" is 0xCBF43926 (CRC RevEng catalogue, fetched 2026-09-14),
    /// computed here byte-at-a-time from the generated table.
    #[test]
    fn crc32_tables_produce_the_catalogue_check_value() {
        let t = crc32_tables();
        let mut crc = 0xFFFF_FFFFu64;
        for &b in b"123456789" {
            crc = (crc >> 8) ^ t[((crc ^ b as u64) & 255) as usize];
        }
        assert_eq!(crc ^ 0xFFFF_FFFF, 0xCBF4_3926);
    }
}
