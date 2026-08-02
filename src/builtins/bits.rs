use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, DefaultResolver, Implementation,
    Lowering, Parameter, ReturnType,
};

// Integer bitwise/shift/rotate operations. Each lowers to one (or a few) native
// AArch64 instructions inline (see `builder_bits.rs`); none is a runtime helper.
// All operands and results are raw two's-complement 64-bit `Integer` bit
// patterns. See `plan-02-encoding.md` Part A.

const BAND: &str = "bits.band";
const BOR: &str = "bits.bor";
const BXOR: &str = "bits.bxor";
const BNOT: &str = "bits.bnot";
const SL: &str = "bits.sl";
const SR: &str = "bits.sr";
const SRA: &str = "bits.sra";
const RL32: &str = "bits.rl32";
const RR32: &str = "bits.rr32";
const RL64: &str = "bits.rl64";
const RR64: &str = "bits.rr64";
const CLZ: &str = "bits.clz";
const CTZ: &str = "bits.ctz";
const POP_COUNT: &str = "bits.popCount";
const BSWAP16: &str = "bits.bswap16";
const BSWAP32: &str = "bits.bswap32";
const BSWAP64: &str = "bits.bswap64";

// plan-72-D: `BITS` is the descriptor authority for this package. Every op takes
// and returns raw two's-complement `Integer` bit patterns, lowers inline (no
// runtime helper, no implementation rewrite), and has a single fixed-arity
// overload. bits has no builtin types, source companion, or custom resolver.
const P_AB: &[Parameter] = &[
    Parameter::required("a", "Integer"),
    Parameter::required("b", "Integer"),
];
const P_A: &[Parameter] = &[Parameter::required("a", "Integer")];
const P_VC: &[Parameter] = &[
    Parameter::required("value", "Integer"),
    Parameter::required("count", "Integer"),
];
const P_V: &[Parameter] = &[Parameter::required("value", "Integer")];

const OV_AB: &[BuiltinOverload] = &[BuiltinOverload {
    params: P_AB,
    return_type: ReturnType::Fixed("Integer"),
}];
const OV_A: &[BuiltinOverload] = &[BuiltinOverload {
    params: P_A,
    return_type: ReturnType::Fixed("Integer"),
}];
const OV_VC: &[BuiltinOverload] = &[BuiltinOverload {
    params: P_VC,
    return_type: ReturnType::Fixed("Integer"),
}];
const OV_V: &[BuiltinOverload] = &[BuiltinOverload {
    params: P_V,
    return_type: ReturnType::Fixed("Integer"),
}];

const fn bits_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Inline,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const BITS_FUNCTIONS: &[BuiltinFunction] = &[
    bits_fn(BAND, "band", OV_AB),
    bits_fn(BOR, "bor", OV_AB),
    bits_fn(BXOR, "bxor", OV_AB),
    bits_fn(BNOT, "bnot", OV_A),
    bits_fn(SL, "sl", OV_VC),
    bits_fn(SR, "sr", OV_VC),
    bits_fn(SRA, "sra", OV_VC),
    bits_fn(RL32, "rl32", OV_VC),
    bits_fn(RR32, "rr32", OV_VC),
    bits_fn(RL64, "rl64", OV_VC),
    bits_fn(RR64, "rr64", OV_VC),
    bits_fn(CLZ, "clz", OV_V),
    bits_fn(CTZ, "ctz", OV_V),
    bits_fn(POP_COUNT, "popCount", OV_V),
    bits_fn(BSWAP16, "bswap16", OV_V),
    bits_fn(BSWAP32, "bswap32", OV_V),
    bits_fn(BSWAP64, "bswap64", OV_V),
];

pub(crate) static BITS: BuiltinModule = BuiltinModule {
    name: "bits",
    functions: BITS_FUNCTIONS,
    types: &[],
    source: None,
    resolver: None,
};

pub(crate) fn is_bits_call(name: &str) -> bool {
    DefaultResolver::contains(&BITS, name)
}

/// The variable-shift ops (`sl`/`sr`/`sra`) are the only `bits::` calls that can
/// raise a user-trappable error: an out-of-range `count` (outside `0 .. 63`) fails
/// `ErrInvalidArgument` (see `builder_bits.rs::lower_bits_shift`). Every other
/// `bits::` op is total. This split drives the inline-`TRAP` fallibility census
/// (`super::inline_builtin_is_infallible` / `inline_builtin_raw_supported`).
pub(crate) fn is_bits_shift(name: &str) -> bool {
    matches!(name, SL | SR | SRA)
}

// `call_param_names` and `expected_arguments` return `&'static` borrowed shapes
// that the owned `DefaultResolver` output (`Vec`/`String`) cannot be coerced to,
// and their consumers require the borrow, so they stay as static literals PINNED
// equal to `BITS` by `parity_matches_descriptor` until plan-72-BB moves the
// consumers onto the owned descriptor API.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        BAND | BOR | BXOR => Some(&[&["a"], &["b"]]),
        BNOT => Some(&[&["a"]]),
        SL | SR | SRA | RL32 | RR32 | RL64 | RR64 => Some(&[&["value"], &["count"]]),
        CLZ | CTZ | POP_COUNT | BSWAP16 | BSWAP32 | BSWAP64 => Some(&[&["value"]]),
        _ => None,
    }
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        BNOT => Some("Integer"),
        CLZ | CTZ | POP_COUNT | BSWAP16 | BSWAP32 | BSWAP64 => Some("Integer"),
        BAND | BOR | BXOR => Some("Integer, Integer"),
        SL | SR | SRA | RL32 | RR32 | RL64 | RR64 => Some("Integer, Integer"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNARY: &[&str] = &[BNOT, CLZ, CTZ, POP_COUNT, BSWAP16, BSWAP32, BSWAP64];
    const BINARY: &[&str] = &[BAND, BOR, BXOR, SL, SR, SRA, RL32, RR32, RL64, RR64];

    fn all() -> Vec<&'static str> {
        UNARY.iter().chain(BINARY.iter()).copied().collect()
    }

    #[test]
    fn is_bits_call_recognizes_all_and_rejects_others() {
        for name in all() {
            assert!(is_bits_call(name), "{name}");
        }
        assert!(!is_bits_call("bits.unknown"));
        assert!(!is_bits_call("strings.trim"));
        assert!(!is_bits_call(""));
    }

    #[test]
    fn param_names_by_group() {
        for name in [BAND, BOR, BXOR] {
            assert_eq!(
                call_param_names(name),
                Some(&[&["a"][..], &["b"][..]][..]),
                "{name}"
            );
        }
        assert_eq!(call_param_names(BNOT), Some(&[&["a"][..]][..]));
        for name in [SL, SR, SRA, RL32, RR32, RL64, RR64] {
            assert_eq!(
                call_param_names(name),
                Some(&[&["value"][..], &["count"][..]][..]),
                "{name}"
            );
        }
        for name in [CLZ, CTZ, POP_COUNT, BSWAP16, BSWAP32, BSWAP64] {
            assert_eq!(
                call_param_names(name),
                Some(&[&["value"][..]][..]),
                "{name}"
            );
        }
        assert_eq!(call_param_names("bits.nope"), None);
    }

    #[test]
    fn expected_arguments_by_group() {
        assert_eq!(expected_arguments(BNOT), Some("Integer"));
        for name in [CLZ, CTZ, POP_COUNT, BSWAP16, BSWAP32, BSWAP64] {
            assert_eq!(expected_arguments(name), Some("Integer"), "{name}");
        }
        for name in [BAND, BOR, BXOR] {
            assert_eq!(expected_arguments(name), Some("Integer, Integer"), "{name}");
        }
        for name in [SL, SR, SRA, RL32, RR32, RL64, RR64] {
            assert_eq!(expected_arguments(name), Some("Integer, Integer"), "{name}");
        }
        assert_eq!(expected_arguments("bits.nope"), None);
    }

}
