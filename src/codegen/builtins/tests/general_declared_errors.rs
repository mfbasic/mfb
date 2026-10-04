//! Every `general` member (the bare-name globals `toString`, `toInt`, `len`, …)
//! declares exactly the error codes its lowering can raise (bug-807).
//!
//! The `general` members are `Body::Intrinsic`: their registry entry carries no
//! lowering — codegen is the builder's own bare-name dispatch
//! (`builder_conversions.rs`, `builder_strings.rs`) — so neither
//! `declared_errors_all.rs` (which lowers `AbiFunction`/`AbiInline` bodies) nor
//! `mfb_body_declared_errors.rs` (which scans `Body::Mfb` source) could see
//! them, and `toString`/`toInt` declared no errors while raising four.
//!
//! So this guard compiles a real program per OVERLOAD — one `probe` function
//! whose parameters have that overload's argument types and whose body is the
//! single call — on all five backends at `-O0` (no inlining, so `probe` is its
//! own function), and collects every error code `probe` and the internal
//! functions it transitively calls load, the same 8-digit-immediate signal
//! `declared_errors_sweep.rs` uses. A parameter is never a literal, so nothing
//! folds, and a `Float` parameter is finite by the language's observation rule —
//! exactly the argument a real call site sees, except for an arithmetic
//! `Float` argument, which is observed at the call (`ErrFloatOverflow`/
//! `ErrFloatNaN`, spec `04_types.md`), a property of the operator and not the
//! member, and so not part of any member's raise set.
//!
//! Two assertions per member: the union over its overloads equals the
//! descriptor's `errors` (what the Errors table renders), and each overload's
//! own set equals the per-overload row below (what the prose must state).
//! The rows are the evidence for the prose; a lowering change that adds or
//! removes a code fails here first.

use crate::codegen::engine::types::{NativeCodePlan, RelocIntent};
use crate::codegen::registry::registry;
use crate::testutil::{code_for_src_at, CodeTarget};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::declared_errors_sweep::codes_by_value;

/// `(member, argument types, return type, codes that overload raises)` — one
/// row per overload `general/mod.rs:resolve_call` accepts that a program can
/// spell. The rows are the per-overload evidence each member's prose states,
/// and `mfb_body_declared_errors.rs` reads them to know what a bare call in a
/// package body can raise once it knows the argument's type.
///
/// An argument type is spelled as a program writes it; `Color` stands for any
/// `ENUM` (the probe declares one). `toString`'s `AttributedString` arm is not
/// a row: the type checker rejects `toString(astrings::AttributedString)` in a
/// program (`TYPE_CALL_ARGUMENT_MISMATCH`), so no call can reach it.
pub(super) const OVERLOADS: &[(&str, &[&str], &str, &[&str])] = &[
    // toString: every arm builds a new String (the integer/float formatters, the
    // out-of-line Scalar/Fixed/Money helpers whose failure the call site
    // re-raises as one code, `copy_flat_block` for String/enum/Boolean), so every
    // arm can run out of memory; only the `List OF Byte` arm decodes UTF-8.
    ("toString", &["Integer"], "String", &["ErrOutOfMemory"]),
    ("toString", &["Float"], "String", &["ErrOutOfMemory"]),
    (
        "toString",
        &["Float", "Byte"],
        "String",
        &["ErrOutOfMemory"],
    ),
    ("toString", &["Fixed"], "String", &["ErrOutOfMemory"]),
    (
        "toString",
        &["Fixed", "Byte"],
        "String",
        &["ErrOutOfMemory"],
    ),
    ("toString", &["Money"], "String", &["ErrOutOfMemory"]),
    (
        "toString",
        &["Money", "Byte"],
        "String",
        &["ErrOutOfMemory"],
    ),
    ("toString", &["Boolean"], "String", &["ErrOutOfMemory"]),
    ("toString", &["String"], "String", &["ErrOutOfMemory"]),
    ("toString", &["Byte"], "String", &["ErrOutOfMemory"]),
    ("toString", &["Scalar"], "String", &["ErrOutOfMemory"]),
    (
        "toString",
        &["List OF Byte"],
        "String",
        &["ErrEncoding", "ErrOutOfMemory"],
    ),
    ("toString", &["Color"], "String", &["ErrOutOfMemory"]),
    // toInt: the two parses and the Float truncation raise; a Fixed, Money, Byte,
    // Scalar or enum always fits (a move, a shift or a division by 100000).
    // A Float is finite (see UNREACHABLE), so it can only be out of range.
    (
        "toInt",
        &["String"],
        "Integer",
        &["ErrInvalidFormat", "ErrOverflow"],
    ),
    (
        "toInt",
        &["String", "Integer"],
        "Integer",
        &["ErrInvalidFormat", "ErrOverflow"],
    ),
    ("toInt", &["Float"], "Integer", &["ErrOverflow"]),
    ("toInt", &["Fixed"], "Integer", &[]),
    ("toInt", &["Money"], "Integer", &[]),
    ("toInt", &["Byte"], "Integer", &[]),
    ("toInt", &["Scalar"], "Integer", &[]),
    ("toInt", &["Color"], "Integer", &[]),
    // error builds the `Error` record and its message; `FAIL error(...)` is the
    // FAIL statement's own raise, not this call.
    (
        "error",
        &["Integer", "String"],
        "Error",
        &["ErrOutOfMemory"],
    ),
    // len/isEmpty/isNotEmpty read a length field.
    ("len", &["String"], "Integer", &[]),
    ("isEmpty", &["String"], "Boolean", &[]),
    ("isNotEmpty", &["String"], "Boolean", &[]),
    ("len", &["List OF Integer"], "Integer", &[]),
    ("isEmpty", &["List OF Integer"], "Boolean", &[]),
    ("isNotEmpty", &["List OF Integer"], "Boolean", &[]),
    ("len", &["Map OF String TO Integer"], "Integer", &[]),
    ("isEmpty", &["Map OF String TO Integer"], "Boolean", &[]),
    ("isNotEmpty", &["Map OF String TO Integer"], "Boolean", &[]),
    ("len", &["Set OF Integer"], "Integer", &[]),
    ("isEmpty", &["Set OF Integer"], "Boolean", &[]),
    ("isNotEmpty", &["Set OF Integer"], "Boolean", &[]),
    // typeName folds to a string constant (the probe reads it with `len`, so no copy is made).
    ("typeName", &["Integer"], "String", &[]),
    ("typeName", &["String"], "String", &[]),
    ("typeName", &["Color"], "String", &[]),
    ("typeName", &["List OF Integer"], "String", &[]),
    // toFloat: only the parse fails.
    (
        "toFloat",
        &["String"],
        "Float",
        &["ErrInvalidFormat", "ErrOverflow"],
    ),
    ("toFloat", &["Integer"], "Float", &[]),
    ("toFloat", &["Fixed"], "Float", &[]),
    ("toFloat", &["Money"], "Float", &[]),
    // toFixed: Fixed's integer part is 32 bits, so every source but a parse failure
    // can only overflow.
    (
        "toFixed",
        &["String"],
        "Fixed",
        &["ErrInvalidFormat", "ErrOverflow"],
    ),
    ("toFixed", &["Integer"], "Fixed", &["ErrOverflow"]),
    ("toFixed", &["Float"], "Fixed", &["ErrOverflow"]),
    ("toFixed", &["Money"], "Fixed", &["ErrOverflow"]),
    // toByte: anything past 255 overflows.
    ("toByte", &["Integer"], "Byte", &["ErrOverflow"]),
    ("toByte", &["Money"], "Byte", &["ErrOverflow"]),
    ("toByte", &["Scalar"], "Byte", &["ErrOverflow"]),
    // toScalar: a surrogate or out-of-range code point, or a String that is not one
    // scalar, is an invalid argument; every Byte is a scalar.
    ("toScalar", &["Integer"], "Scalar", &["ErrInvalidArgument"]),
    ("toScalar", &["String"], "Scalar", &["ErrInvalidArgument"]),
    ("toScalar", &["Byte"], "Scalar", &[]),
    // toMoney: Money holds about ±9.2e13, so an Integer, Float or String can
    // overflow; a Fixed (below ±2^31) or a Byte always fits.
    (
        "toMoney",
        &["String"],
        "Money",
        &["ErrInvalidFormat", "ErrOverflow"],
    ),
    ("toMoney", &["Integer"], "Money", &["ErrOverflow"]),
    ("toMoney", &["Float"], "Money", &["ErrOverflow"]),
    ("toMoney", &["Fixed"], "Money", &[]),
    ("toMoney", &["Byte"], "Money", &[]),
    // The predicates compare in registers and never fail.
    ("isNumeric", &["String"], "Boolean", &[]),
    ("isEven", &["Integer"], "Boolean", &[]),
    ("isOdd", &["Integer"], "Boolean", &[]),
    ("isPositive", &["Integer"], "Boolean", &[]),
    ("isPositive", &["Float"], "Boolean", &[]),
    ("isPositive", &["Fixed"], "Boolean", &[]),
    ("isNegative", &["Integer"], "Boolean", &[]),
    ("isNegative", &["Float"], "Boolean", &[]),
    ("isNegative", &["Fixed"], "Boolean", &[]),
    ("isZero", &["Integer"], "Boolean", &[]),
    ("isZero", &["Float"], "Boolean", &[]),
    ("isZero", &["Fixed"], "Boolean", &[]),
];

/// A code an overload's lowering loads that can never be raised, with the
/// data-flow proof — the same bar as `declared_errors_sweep.rs`'s
/// `unreachable_own_raise`. `(member, argument types, code, proof)`.
const UNREACHABLE: &[(&str, &[&str], &str, &str)] = &[
    (
        "toInt",
        &["Float"],
        "ErrInvalidFormat",
        "emit_float_to_int_value's `invalid` label is its NaN/infinity exit. A Float \
         argument is finite: a name, field or call result holds a finite value by the \
         language's invariant (spec 04_types.md, Float finiteness), and an arithmetic \
         argument is checked by lower_to_int's observe_float before the conversion runs.",
    ),
    (
        "toFixed",
        &["Float"],
        "ErrInvalidFormat",
        "emit_float_bits_to_fixed_value's `invalid` label is its NaN/infinity exit; the \
         argument is finite for the same reason as toInt(Float) (lower_to_fixed's \
         observe_float).",
    ),
    (
        "toMoney",
        &["Float"],
        "ErrInvalidFormat",
        "lower_to_money's Float arm raises ErrInvalidFormat only for a non-finite value; \
         the argument is finite for the same reason as toInt(Float) (lower_to_money's \
         observe_float).",
    ),
    (
        "toMoney",
        &["Fixed"],
        "ErrOverflow",
        "lower_to_money's Fixed arm is emit_fixed_multiply(raw, 100000), whose range check \
         fires only when the product leaves the Money range. A Fixed's magnitude is below \
         2^31, so the result is below 2^31 * 100000 < 2^48 raw units, far inside i64.",
    ),
    (
        "toFixed",
        &["Money"],
        "ErrInvalidArgument",
        "emit_fixed_divide's emit_nonzero_or_invalid guards its divisor, and \
         lower_to_fixed's Money arm passes the constant 100000 \
         (move_immediate(&scale, \"Integer\", \"100000\")), which is never zero.",
    ),
];

/// The shared runtime helpers a conversion calls that never raise a code of
/// their own — the same classification `declared_errors_all.rs` and
/// `fs_declared_errors.rs` give them. Every other internal callee (the
/// out-of-line `_mfb_rt_*_to_string` formatters) is followed and scanned.
const RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_make_error_result",
        "packages an error code the caller already loaded",
    ),
    (
        "_mfb_rt_park_error",
        "parks an error the caller already raised",
    ),
    (
        "_mfb_rt_validate_utf8",
        "returns 0/1 in x0; the caller raises ErrEncoding",
    ),
];

/// The probe program for one overload.
fn probe_source(member: &str, args: &[&str], ret: &str) -> String {
    let params: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(i, ty)| format!("a{i} AS {ty}"))
        .collect();
    let call_args: Vec<String> = (0..args.len()).map(|i| format!("a{i}")).collect();
    let imports = if args.iter().any(|ty| ty.starts_with("astrings::")) {
        "IMPORT astrings\n\n"
    } else {
        ""
    };
    // Consume the result without copying it: returning or binding a `String`
    // that is not freshly built (`typeName`'s folded constant) copies it into
    // the arena, and that copy's ErrOutOfMemory belongs to the RETURN, not to
    // the member. `len` reads a String in place; an `Error`'s `code` is a field
    // read; every other result is a scalar returned in a register.
    let call = format!("{member}({})", call_args.join(", "));
    let (result, consume) = match ret {
        "String" => ("Integer", format!("len({call})")),
        "Error" => ("Integer", format!("{call}.code")),
        other => (other, call.clone()),
    };
    format!(
        "{imports}ENUM Color\n  Red, Green\nEND ENUM\n\n\
         FUNC probe({}) AS {result}\n  RETURN {consume}\nEND FUNC\n\n\
         FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        params.join(", "),
    )
}

/// Every error code the plan function `root` and every internal function it
/// transitively calls load.
fn transitive_codes(
    plan: &NativeCodePlan,
    root: &str,
    codes: &BTreeMap<&'static str, &'static str>,
) -> Result<BTreeSet<&'static str>, String> {
    let by_symbol: BTreeMap<&str, _> = plan
        .functions
        .iter()
        .map(|function| (function.symbol.as_str(), function))
        .collect();
    let mut raised = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([root.to_string()]);
    while let Some(symbol) = queue.pop_front() {
        if !seen.insert(symbol.clone()) {
            continue;
        }
        let Some(function) = by_symbol.get(symbol.as_str()) else {
            // An internal call target that is not a plan function is a
            // runtime-library symbol linked from outside the plan; the guard
            // cannot see into it, so it must not pass silently.
            return Err(format!(
                "`{root}` reaches `{symbol}`, which the plan does not define"
            ));
        };
        for instruction in &function.instructions {
            if let Some(name) = instruction
                .get("value")
                .and_then(|value| codes.get(value.as_str()))
            {
                // The probe's OWN entry check (`emit_stack_check`, emitted because
                // its body holds a call node) raises ErrStackOverflow to the
                // probe's caller before the member runs: an error of the probe's
                // frame, which every function with a call carries, not of the
                // member. A callee's ErrStackOverflow is still counted.
                if symbol == root && *name == "ErrStackOverflow" {
                    continue;
                }
                raised.insert(*name);
            }
        }
        for relocation in &function.relocations {
            if relocation.kind == RelocIntent::Call
                && relocation.library.is_none()
                && !RAISES_NOTHING
                    .iter()
                    .any(|(name, _)| *name == relocation.to)
            {
                queue.push_back(relocation.to.clone());
            }
        }
    }
    Ok(raised)
}

/// The codes one overload raises on every backend (unioned), or why it could
/// not be measured.
fn measure(
    member: &str,
    args: &[&str],
    ret: &str,
    codes: &BTreeMap<&'static str, &'static str>,
) -> Result<BTreeSet<&'static str>, String> {
    let source = probe_source(member, args, ret);
    let mut raised = BTreeSet::new();
    for target in CodeTarget::ALL {
        let plan = code_for_src_at(&source, target, crate::target::NativeBuildMode::Console, 0)
            .map_err(|error| format!("{}: {error}\n{source}", target.name()))?;
        let probe = plan
            .functions
            .iter()
            .find(|function| function.name == "probe")
            .ok_or_else(|| format!("{}: no `probe` function in the plan", target.name()))?;
        raised.extend(transitive_codes(&plan, &probe.symbol, codes)?);
    }
    Ok(raised)
}

#[test]
fn every_general_member_declares_exactly_the_errors_its_lowering_raises() {
    let codes = codes_by_value();
    let general = registry()
        .resolve_package("general")
        .expect("general package");
    let mut failures = Vec::new();
    let mut by_member: BTreeMap<&str, BTreeSet<&'static str>> = BTreeMap::new();
    for (member, args, ret, expected) in OVERLOADS {
        match measure(member, args, ret, &codes) {
            Ok(mut raised) => {
                raised.retain(|code| {
                    !UNREACHABLE
                        .iter()
                        .any(|(m, a, c, _)| m == member && a == args && c == code)
                });
                let expected: BTreeSet<&str> = expected.iter().copied().collect();
                if raised != expected {
                    failures.push(format!(
                        "{member}({}) raises {raised:?}; its OVERLOADS row says {expected:?}",
                        args.join(", ")
                    ));
                }
                by_member.entry(member).or_default().extend(raised);
            }
            Err(error) => failures.push(format!("{member}({}): {error}", args.join(", "))),
        }
    }
    for (member, raised) in &by_member {
        let function = general
            .functions()
            .iter()
            .find(|function| function.name == *member)
            .unwrap_or_else(|| panic!("general::{member} is registered"));
        let declared: BTreeSet<&str> = function
            .implementations()
            .iter()
            .flat_map(|implementation| implementation.errors.iter().copied())
            .collect();
        if &declared != raised {
            failures.push(format!(
                "general::{member} declares {declared:?} but its overloads raise {raised:?} \
                 (fix the descriptor `errors` in src/codegen/builtins/general/func_*.rs)"
            ));
        }
    }
    assert!(failures.is_empty(), "\n  {}", failures.join("\n  "));
}
