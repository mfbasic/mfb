//! Every package member with a `Body::Intrinsic` implementation outside
//! `general` declares exactly the error codes its lowering can raise.
//!
//! An intrinsic's registry entry carries no lowering: codegen is the builder's
//! own dispatch (`builder_search/`, `builder_strings/replace.rs`) or, for
//! `encoding::utf8Encode`/`utf8Decode`, a package helper the monomorphizer
//! picks by type. So `declared_errors_all.rs` (which lowers `AbiFunction`/
//! `AbiInline` bodies) and `mfb_body_declared_errors/` (which scans `Body::Mfb`
//! and `Body::Rewrite` source) never saw them, and `strings::mid`/`replace`,
//! `collections::mid` and both `utf8` codecs raised `ErrOutOfMemory` while
//! declaring none.
//!
//! This guard measures them the way `general_declared_errors.rs` measures the
//! bare-name globals: one `probe` function per overload, whose parameters have
//! that overload's argument types and whose body is the single call, compiled
//! on every backend at `-O0`; every error code `probe` and the internal
//! functions it calls load is collected. Each implementation's `errors` must
//! equal the union of its rows, and every row must equal what was measured.

use crate::codegen::registry::{registry, Body};
use crate::testutil::{code_for_src_at, CodeTarget};
use std::collections::{BTreeMap, BTreeSet};

use super::declared_errors_sweep::codes_by_value;
use super::general_declared_errors::transitive_codes;

/// `(package, member, implementation index, argument types, return type, codes)`
/// — one row per overload a program can spell. A generic `T` is probed with a
/// scalar and a `String` element, since a `String` element is compared and
/// copied through a different path.
const OVERLOADS: &[(&str, &str, usize, &[&str], &str, &[&str])] = &[
    (
        "strings",
        "find",
        0,
        &["String", "String", "Integer"],
        "Integer",
        &["ErrIndexOutOfRange", "ErrNotFound"],
    ),
    (
        "strings",
        "mid",
        0,
        &["String", "Integer", "Integer"],
        "String",
        &["ErrIndexOutOfRange", "ErrOutOfMemory"],
    ),
    (
        "strings",
        "replace",
        0,
        &["String", "String", "String"],
        "String",
        &["ErrInvalidArgument", "ErrOutOfMemory"],
    ),
    (
        "collections",
        "find",
        0,
        &["List OF Integer", "Integer", "Integer"],
        "Integer",
        &["ErrIndexOutOfRange", "ErrNotFound"],
    ),
    (
        "collections",
        "find",
        0,
        &["List OF String", "String", "Integer"],
        "Integer",
        &["ErrIndexOutOfRange", "ErrNotFound"],
    ),
    (
        "collections",
        "find",
        1,
        &["List OF Integer", "List OF Integer", "Integer"],
        "Integer",
        &["ErrIndexOutOfRange", "ErrNotFound"],
    ),
    (
        "collections",
        "find",
        1,
        &["List OF String", "List OF String", "Integer"],
        "Integer",
        &["ErrIndexOutOfRange", "ErrNotFound"],
    ),
    (
        "collections",
        "mid",
        0,
        &["List OF Integer", "Integer", "Integer"],
        "List OF Integer",
        &["ErrIndexOutOfRange", "ErrOutOfMemory"],
    ),
    (
        "collections",
        "mid",
        0,
        &["List OF String", "Integer", "Integer"],
        "List OF String",
        &["ErrIndexOutOfRange", "ErrOutOfMemory"],
    ),
    (
        "collections",
        "replace",
        0,
        &["List OF Integer", "Integer", "Integer"],
        "List OF Integer",
        &["ErrOutOfMemory"],
    ),
    (
        "collections",
        "replace",
        0,
        &["List OF String", "String", "String"],
        "List OF String",
        &["ErrOutOfMemory"],
    ),
    (
        "encoding",
        "utf8Encode",
        0,
        &["String"],
        "List OF Byte",
        &["ErrOutOfMemory"],
    ),
    (
        "encoding",
        "utf8Encode",
        1,
        &["String"],
        "List OF Integer",
        &["ErrOutOfMemory"],
    ),
    (
        "encoding",
        "utf8Decode",
        0,
        &["List OF Byte"],
        "String",
        &["ErrInvalidFormat", "ErrOutOfMemory"],
    ),
    (
        "encoding",
        "utf8Decode",
        1,
        &["List OF Integer"],
        "String",
        &["ErrInvalidFormat", "ErrOutOfMemory"],
    ),
];

/// A code an overload's code loads that a reader of the source has proven can
/// never fire: `(package, member, argument types, code, proof)`. Both `utf8Decode`
/// forms run `__encoding_utf8Valid` (`helper_utf8_valid.rs`), whose checked
/// operations are all bounded; the `mfb_body_declared_errors` exception table
/// makes the same arguments for the other helpers that call it.
const UNREACHABLE: &[(&str, &str, &[&str], &str, &str)] = &[
    (
        "encoding",
        "utf8Decode",
        &["List OF Byte"],
        "ErrIndexOutOfRange",
        "__encoding_utf8Valid reads collections::get(data, i) only while i < n, and \
         collections::get(data, i + 1 + j) only after `i + extra >= n` returned FALSE \
         and while j < extra, so every index is in 0 .. n - 1.",
    ),
    (
        "encoding",
        "utf8Decode",
        &["List OF Byte"],
        "ErrOverflow",
        "in __encoding_utf8Valid i, j and extra stay below n + 4 and codePoint below \
         2^21 (at most 7 * 64^3 plus three continuations); toInt of a Byte cannot fail.",
    ),
    (
        "encoding",
        "utf8Decode",
        &["List OF Integer"],
        "ErrIndexOutOfRange",
        "the same __encoding_utf8Valid bounds as the List OF Byte form; FOR EACH reads \
         no index.",
    ),
    (
        "encoding",
        "utf8Decode",
        &["List OF Integer"],
        "ErrOverflow",
        "the same __encoding_utf8Valid bounds, and toByte(unit) runs only after \
         `unit < 0 OR unit > 255` failed with ErrInvalidFormat.",
    ),
    (
        "encoding",
        "utf8Decode",
        &["List OF Byte"],
        "ErrEncoding",
        "__encoding_utf8Decode fails with ErrInvalidFormat unless __encoding_utf8Valid \
         accepts the bytes, so the toString(List OF Byte) that follows never sees \
         ill-formed UTF-8.",
    ),
    (
        "encoding",
        "utf8Decode",
        &["List OF Integer"],
        "ErrEncoding",
        "the same __encoding_utf8Valid check precedes the List OF Integer form's \
         toString(data).",
    ),
];

/// An error raised by a frame's own entry check, not by any member: every
/// MFBASIC helper an intrinsic calls (`__encoding_utf8Encode`, …) checks the
/// stack on entry, as every `Body::Mfb` member's body does, and no member
/// declares it (`mfb_body_declared_errors` never counts it either).
const FRAME_ENTRY_CHECK: &str = "ErrStackOverflow";

/// Packages whose `Body::Intrinsic` members never reach codegen: `testing`'s
/// assertions are rewritten to MFBASIC statements at AST-to-IR lowering
/// (`testing/mod.rs`), and `general` has its own guard
/// (`general_declared_errors.rs`).
const NOT_LOWERED: &[&str] = &["general", "testing"];

/// Intrinsic members no probe can measure, with the reason.
const NOT_PROBED: &[(&str, &str, &str)] = &[(
    "os",
    "getConfigOr",
    "both arguments must be literals and the call is folded to a constant when the \
     program is built, so a probe with parameters does not compile; \
     inplace_self_update_census's LITERAL_FIRST_ARGUMENT pins the page sentence.",
)];

/// The probe program for one overload. The result is bound with its declared
/// type, which also picks a return-type overload (`utf8Encode`), and only its
/// length leaves `probe`, so no copy of it is counted against the member.
fn probe_source(package: &str, member: &str, args: &[&str], ret: &str) -> String {
    let params: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(i, ty)| format!("a{i} AS {ty}"))
        .collect();
    let call_args: Vec<String> = (0..args.len()).map(|i| format!("a{i}")).collect();
    let call = format!("{package}::{member}({})", call_args.join(", "));
    let body = if ret == "Integer" {
        format!("  RETURN {call}\n")
    } else {
        format!("  LET r AS {ret} = {call}\n  RETURN len(r)\n")
    };
    format!(
        "IMPORT {package}\n\n\
         FUNC probe({}) AS Integer\n{body}END FUNC\n\n\
         FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        params.join(", "),
    )
}

fn measure(
    package: &str,
    member: &str,
    args: &[&str],
    ret: &str,
    codes: &BTreeMap<&'static str, &'static str>,
) -> Result<BTreeSet<&'static str>, String> {
    let source = probe_source(package, member, args, ret);
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
fn every_intrinsic_member_declares_exactly_the_errors_its_lowering_raises() {
    let codes = codes_by_value();
    let mut failures = Vec::new();
    let mut by_implementation: BTreeMap<(&str, &str, usize), BTreeSet<&'static str>> =
        BTreeMap::new();
    for (package, member, index, args, ret, expected) in OVERLOADS {
        let label = format!("{package}::{member}({}) AS {ret}", args.join(", "));
        match measure(package, member, args, ret, &codes) {
            Ok(mut raised) => {
                raised.remove(FRAME_ENTRY_CHECK);
                raised.retain(|code| {
                    !UNREACHABLE.iter().any(|(p, m, a, c, _)| {
                        p == package && m == member && a == args && c == code
                    })
                });
                let expected: BTreeSet<&str> = expected.iter().copied().collect();
                if raised != expected {
                    failures.push(format!(
                        "{label} raises {raised:?}; its OVERLOADS row says {expected:?}"
                    ));
                }
                by_implementation
                    .entry((package, member, *index))
                    .or_default()
                    .extend(expected);
            }
            Err(error) => failures.push(format!("{label}: {error}")),
        }
    }

    let reg = registry();
    for package in reg.packages() {
        let pkg = package.import_name();
        if NOT_LOWERED.contains(&pkg) {
            continue;
        }
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                if !matches!(implementation.body, Body::Intrinsic) {
                    continue;
                }
                let name = function.name;
                if NOT_PROBED.iter().any(|(p, m, _)| *p == pkg && *m == name) {
                    continue;
                }
                let Some(rows) = by_implementation.get(&(pkg, name, index)) else {
                    failures.push(format!(
                        "{pkg}::{name} implementation {index} is Body::Intrinsic but has no \
                         OVERLOADS row here"
                    ));
                    continue;
                };
                let declared: BTreeSet<&str> = implementation.errors.iter().copied().collect();
                if &declared != rows {
                    failures.push(format!(
                        "{pkg}::{name} implementation {index} declares {declared:?} but its \
                         overloads raise {rows:?} (fix the descriptor `errors` in \
                         src/codegen/builtins/{pkg}/func_*.rs)"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n  {}", failures.join("\n  "));
}
