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
}
