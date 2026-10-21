//! Codepoint `len()` caching (`optimizer::opt1::lencache`) when the repeated
//! scans sit inside other values — a constructor's argument, a list, a map, a
//! record update, a closure's captures, a call's arguments, a union wrap — and
//! when two Strings are first read by the same statement.
//!
//! The row's contract (its module comment): every cacheable `len(s)` in a
//! straight-line run is replaced by one read of a cache bound just before the
//! statement holding the first, and none is left behind — a partial rewrite
//! would leave a read of a local nothing binds. That is what each case below
//! checks, at a different nesting.

use super::nir_build::*;
use crate::operators::BinaryOp;
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::{NirOp, NirRecordUpdate, NirValue};
use crate::types::ParameterType;

fn string_bind(name: &str) -> NirOp {
    NirOp::Bind {
        mutable: false,
        name: name.to_string(),
        type_: ParameterType::String,
        value: Some(NirValue::Const {
            type_: ParameterType::String,
            value: "héllo".to_string(),
        }),
    }
}

fn len_of(name: &str) -> NirValue {
    NirValue::Call {
        target: "len".to_string(),
        args: vec![local(name)],
        loc: Default::default(),
    }
}

fn boxed(value: NirValue) -> Box<NirValue> {
    Box::new(value)
}

/// Every `len(...)` call left anywhere in `ops`, by the local it reads.
fn len_reads(ops: &[NirOp]) -> Vec<String> {
    #[derive(Default)]
    struct Lens(Vec<String>);
    impl NirVisitor for Lens {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Call { target, args, .. } = value {
                if let [NirValue::Local(name)] = args.as_slice() {
                    if target == "len" {
                        self.0.push(name.clone());
                    }
                }
            }
            walk_value(self, value);
        }
    }
    let mut lens = Lens::default();
    lens.visit_ops(ops);
    lens.0
}

#[test]
fn nested_scans_of_two_strings_are_each_cached_once() {
    let body = run_row(
        3,
        vec![
            string_bind("s"),
            string_bind("t"),
            bind("a", binary(BinaryOp::Add, len_of("t"), len_of("s"))),
            bind(
                "b",
                NirValue::ListLiteral {
                    type_: ParameterType::list_of(ParameterType::Integer),
                    values: vec![len_of("s"), len_of("t")],
                },
            ),
            bind(
                "c",
                NirValue::MapLiteral {
                    type_: ParameterType::map_of(ParameterType::Integer, ParameterType::Integer),
                    entries: vec![(len_of("t"), len_of("s"))],
                },
            ),
            bind(
                "d",
                NirValue::ResultIsOk {
                    value: boxed(NirValue::CallResult {
                        target: "f".to_string(),
                        args: vec![len_of("s")],
                        loc: Default::default(),
                    }),
                },
            ),
            bind(
                "e",
                NirValue::Constructor {
                    type_: ParameterType::named("Box"),
                    args: vec![len_of("t")],
                },
            ),
            bind(
                "f",
                NirValue::Closure {
                    name: "$lambda1".to_string(),
                    type_: ParameterType::Unknown,
                    captures: vec![len_of("s")],
                },
            ),
            bind(
                "g",
                NirValue::WithUpdate {
                    type_: ParameterType::named("Box"),
                    target: boxed(local("r")),
                    updates: vec![NirRecordUpdate {
                        field: "n".to_string(),
                        value: len_of("t"),
                    }],
                },
            ),
            bind(
                "h",
                NirValue::UnionWrap {
                    union_type: ParameterType::named("U"),
                    member_type: ParameterType::Integer,
                    value: boxed(len_of("s")),
                },
            ),
            bind(
                "i",
                NirValue::RuntimeCall {
                    helper: crate::target::shared::runtime::RuntimeHelper::Abi,
                    target: "abi.f".to_string(),
                    args: vec![len_of("t")],
                    loc: Default::default(),
                },
            ),
        ],
        crate::optimizer::opt1::lencache::cache,
    );
    // Both caches open the run, in name order (both are first read by `a`),
    // and they are the only `len` calls left.
    let cache_of = |op: &NirOp| match op {
        NirOp::Bind {
            name,
            value: Some(NirValue::Call { target, args, .. }),
            ..
        } if target == "len" => match args.as_slice() {
            [NirValue::Local(read)] => Some((name.clone(), read.clone())),
            _ => None,
        },
        _ => None,
    };
    assert_eq!(
        cache_of(&body[2]),
        Some(("__len_cache_0".to_string(), "s".to_string()))
    );
    assert_eq!(
        cache_of(&body[3]),
        Some(("__len_cache_1".to_string(), "t".to_string()))
    );
    assert_eq!(
        len_reads(&body),
        ["s", "t"],
        "every other scan was replaced"
    );
    assert_eq!(body.len(), 13);
}
