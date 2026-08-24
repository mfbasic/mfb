//! The built-in `bits` package (plan-95 migration).
//!
//! `bits` provides the integer bitwise/shift/rotate operations the language
//! operator set intentionally omits (`AND`/`OR`/`XOR`/`NOT` are reserved *logical*
//! keywords, so the bit-level forms are named `band`/`bor`/`bxor`/`bnot`). Every
//! member takes and returns raw two's-complement 64-bit `Integer` bit patterns and
//! lowers to one (or a few) native instructions inline — there is no source
//! companion, no value type, no resource, and no runtime helper.
//!
//! Each member is a `Body::abi_inline(lower_bits_*)` intrinsic: its
//! `Implementation::Native` `abi_inline` slot points at a
//! [`crate::codegen::registry::AbiInline`] function that lives in that member's own
//! `func_*.rs` and emits its unique instruction sequence directly through `abi::`.
//! (`bits` was the first package migrated off the legacy `common`/`NativeLower`
//! slot onto the unified `AbiInline` lowering.) The `AbiInline` dispatch hands each
//! body its **pre-lowered, stabilized** `ValueResult` operands, so each member is
//! self-contained — it type-checks its operands inline and emits its op, with no
//! shared per-arity helper. The three variable-shift members
//! (`sl`/`sr`/`sra`) declare `ErrInvalidArgument` (an out-of-range `count`), which
//! is what routes an inline `TRAP` on them through the raw-capture path; the other
//! 14 members declare no error and are infallible — a distinction the inline-`TRAP`
//! fallibility census now reads off this registry data rather than a `bits.` name
//! predicate.

use crate::codegen::registry::{Registry, RegistryPackage};

mod func_band;
mod func_bnot;
mod func_bor;
mod func_bswap16;
mod func_bswap32;
mod func_bswap64;
mod func_bxor;
mod func_clz;
mod func_ctz;
mod func_pop_count;
mod func_rl32;
mod func_rl64;
mod func_rr32;
mod func_rr64;
mod func_sl;
mod func_sr;
mod func_sra;

const MODULE_INTRO: &str = r#"Integer bitwise, shift, and rotate operations"#;
const MODULE_DESC: &str = r#"The `bits` package provides the bitwise integer operations the language operator
set intentionally omits. The reserved words `AND`, `OR`, `XOR`, and `NOT` are
logical (Boolean) operators, so byte-level codecs and other bit-twiddling are
written with these functions instead. The Boolean operations are named
`band`/`bor`/`bxor`/`bnot` precisely because `and`/`or`/`xor`/`not` are reserved
logical keywords and cannot be package member identifiers.

Every operand and result is a raw two's-complement 64-bit `Integer` bit pattern.
The functions do not interpret sign except where a signature says so — `sra`, the
arithmetic right shift. Every function takes and returns `Integer`, never Float,
String, or a collection.

Each function lowers to one (or a few) native instructions inline, like
`math::abs`, rather than calling a runtime helper, and produces identical results
on the native and Binary Representation execution paths.

Shifts (`sl`, `sr`, `sra`) validate their `count` argument. Rotates come in four
named width variants — `rl32`/`rr32` rotate the low 32 bits (for word-oriented
algorithms such as ChaCha20) and `rl64`/`rr64` rotate all 64 bits. Rotate counts
are reduced modulo the rotate width, so any count is defined and the rotates do
not raise. `clz`/`ctz`/`popCount` count leading zeros, trailing zeros, and set
bits; `bswap16`/`bswap32`/`bswap64` reverse the bytes of the low 16/32 or all 64
bits. All functions are total except the three shifts.

`bits` is a built-in package: `IMPORT bits` needs no manifest dependency."#;

/// Register the `bits` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("bits", MODULE_INTRO, MODULE_DESC);

    func_band::register(&mut pkg);
    func_bor::register(&mut pkg);
    func_bxor::register(&mut pkg);
    func_bnot::register(&mut pkg);
    func_sl::register(&mut pkg);
    func_sr::register(&mut pkg);
    func_sra::register(&mut pkg);
    func_rl32::register(&mut pkg);
    func_rr32::register(&mut pkg);
    func_rl64::register(&mut pkg);
    func_rr64::register(&mut pkg);
    func_clz::register(&mut pkg);
    func_ctz::register(&mut pkg);
    func_pop_count::register(&mut pkg);
    func_bswap16::register(&mut pkg);
    func_bswap32::register(&mut pkg);
    func_bswap64::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;

    #[test]
    fn bits_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("bits").expect("bits package");
        assert_eq!(pkg.functions().len(), 17);
        // `bits` injects no source (no records/unions/enums/Mfb bodies/helpers).
        assert!(pkg.get_mfb().is_empty());
        // No value types, no resources.
        assert!(!registry().is_builtin_type("bits"));
    }

    #[test]
    fn shift_ops_are_fallible_the_rest_total() {
        // The three variable shifts declare `ErrInvalidArgument`; every other member
        // declares no error. This is the fact the inline-`TRAP` fallibility census
        // now keys on.
        for name in ["sl", "sr", "sra"] {
            let func = registry()
                .resolve_func(&format!("bits.{name}"))
                .expect("member")
                .function;
            assert!(
                func.declares_error("ErrInvalidArgument"),
                "{name} should declare ErrInvalidArgument"
            );
        }
        for name in [
            "band", "bor", "bxor", "bnot", "rl32", "rr32", "rl64", "rr64", "clz", "ctz",
            "popCount", "bswap16", "bswap32", "bswap64",
        ] {
            let func = registry()
                .resolve_func(&format!("bits.{name}"))
                .expect("member")
                .function;
            assert!(
                !func.declares_error("ErrInvalidArgument"),
                "{name} should be total"
            );
        }
    }

    #[test]
    fn every_member_is_an_abi_inline_intrinsic() {
        // Each member owns a `Body::abi_inline` call-site lowering, so the
        // generic AbiInline dual-path (`try_abi_inline_lower`) reaches it.
        for name in [
            "band", "bor", "bxor", "bnot", "sl", "sr", "sra", "rl32", "rr32", "rl64", "rr64",
            "clz", "ctz", "popCount", "bswap16", "bswap32", "bswap64",
        ] {
            assert!(
                crate::codegen::registry::abi_inline_lower(&format!("bits.{name}")).is_some(),
                "{name} should have an abi_inline lowering"
            );
        }
    }
}
