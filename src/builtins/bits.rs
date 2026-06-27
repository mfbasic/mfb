use std::borrow::Cow;

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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_bits_call(name: &str) -> bool {
    matches!(
        name,
        BAND | BOR
            | BXOR
            | BNOT
            | SL
            | SR
            | SRA
            | RL32
            | RR32
            | RL64
            | RR64
            | CLZ
            | CTZ
            | POP_COUNT
            | BSWAP16
            | BSWAP32
            | BSWAP64
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        BAND | BOR | BXOR => Some(&[&["a"], &["b"]]),
        BNOT => Some(&[&["a"]]),
        SL | SR | SRA | RL32 | RR32 | RL64 | RR64 => Some(&[&["value"], &["count"]]),
        CLZ | CTZ | POP_COUNT | BSWAP16 | BSWAP32 | BSWAP64 => Some(&[&["value"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    is_bits_call(name).then_some("Integer")
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        BNOT | CLZ | CTZ | POP_COUNT | BSWAP16 | BSWAP32 | BSWAP64 => Some((1, 1)),
        BAND | BOR | BXOR | SL | SR | SRA | RL32 | RR32 | RL64 | RR64 => Some((2, 2)),
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

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let (min, max) = arity(name)?;
    if !(min..=max).contains(&arg_types.len()) {
        return None;
    }
    if arg_types.iter().any(|type_| type_ != "Integer") {
        return None;
    }
    Some(ResolvedCall {
        return_type: Cow::Borrowed("Integer"),
    })
}
