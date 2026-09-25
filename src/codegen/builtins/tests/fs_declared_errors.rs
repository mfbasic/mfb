//! Every `fs` member declares exactly the error codes its lowering can raise
//! (bug-694).
//!
//! `mfb man fs <member>` derives its Errors table from the descriptor's `errors`,
//! and nothing tied that list to the code: every `fs` member was migrated with
//! `errors: vec![]`, so no `fs` page had an Errors section. The textual guard
//! `every_raise_error_site_is_declared_in_its_descriptor` cannot see these raises
//! — they go through `raise_error_into(symbol, "Err…", …)` in an `abi_function`
//! helper, and many are emitted by the shared errno mappers in
//! `fs::gen_shared`, so the error name is never at the member's own call site.
//!
//! So this test does not read the source: it LOWERS each member's real body on
//! all five backends and collects every error code the emitted instructions load.
//! Every raise path (`raise_error_into`, `CodeBuilder::raise_error_bare`) moves
//! the error's numeric `errorCode` value into a register, so an 8-digit code
//! immediate is the one signal shared by all of them. The POSIX and Windows
//! errno mappers dispatch to different code sets, so one backend is not enough.
//!
//! A body can also return an error raised by a helper it calls. Every internal
//! call target is either scanned too ([`PROPAGATING`]) or listed in
//! [`RAISES_NOTHING`], with the reason. An unclassified callee fails the test, so
//! a new call cannot hide a new error.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::codegen::engine::types::{
    CodeInstruction, CodeRelocation, CodegenPlatform, PlatformFamily,
};
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, AbiCtx, Body, Implementation, RegistryFunction};
use crate::os::linux::flavor::LinuxFlavor;
use std::collections::{BTreeMap, BTreeSet};

/// The five real backends. The stub `TestPlatform` leaves the OS hooks
/// `unimplemented!()`, and the errno mapping differs between POSIX and Windows.
fn platforms() -> Vec<(&'static str, Box<dyn CodegenPlatform>)> {
    vec![
        (
            "macos-aarch64",
            Box::new(crate::target::macos_aarch64::code::Platform),
        ),
        (
            "windows-x86_64",
            Box::new(crate::target::win_x86_64::code::Platform),
        ),
        (
            "linux-aarch64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_aarch64::code::Aarch64,
                LinuxFlavor::Glibc,
            )),
        ),
        (
            "linux-x86_64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_x86_64::code::X86_64,
                LinuxFlavor::Glibc,
            )),
        ),
        (
            "linux-riscv64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_riscv64::code::Riscv64,
                LinuxFlavor::Glibc,
            )),
        ),
    ]
}

/// Internal callees that return a status the CALLER turns into an error (so the
/// code is already in the caller's own instructions), or that only carry an
/// error the caller already raised.
const RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_rt_validate_utf8",
        "returns 0/1 in x0; the caller raises ErrEncoding",
    ),
    (
        "_mfb_rt_fs_file_drain",
        "returns 0/1 in x0; the caller raises ErrWriteFailed",
    ),
    (
        "_mfb_rt_sort_string_list",
        "sorts in place and makes no calls; it cannot fail",
    ),
    (
        "_mfb_make_error_result",
        "packages an error code the caller already loaded",
    ),
    (
        "_mfb_rt_park_error",
        "parks an error the caller already raised",
    ),
];

/// Internal callees whose own raises reach the caller's Result, and how to lower
/// them for scanning.
enum Propagating {
    /// Another `fs` member's runtime helper, `_mfb_rt_fs_fs_<member>`.
    Member(&'static str),
    /// A standalone helper that is not a registry member.
    Helper(fn() -> (Vec<CodeInstruction>, Vec<CodeRelocation>)),
}

const PROPAGATING: &[(&str, Propagating)] = &[
    (
        "_mfb_rt_fs_fs_readAllBytes",
        Propagating::Member("readAllBytes"),
    ),
    (
        crate::codegen::builtins::fs::gen_path_builder::FS_PATH_JOIN_SYMBOL,
        Propagating::Helper(|| {
            let function =
                crate::codegen::builtins::fs::gen_path_builder::lower_fs_path_join_helper();
            (function.instructions, function.relocations)
        }),
    ),
];

/// Codes a callee can raise that the named caller provably never reaches, and why.
const UNREACHABLE_VIA_CALLEE: &[(&str, &str, &str)] = &[(
    "readBytes",
    "ErrResourceClosed",
    "readBytes hands readAllBytes the File record it just opened, with \
     FILE_OFFSET_CLOSED stored as 0, so the closed-handle check cannot fire",
)];

/// Error codes by numeric value (`"77030001"` -> `"ErrPathNotFound"`).
fn codes_by_value() -> BTreeMap<&'static str, &'static str> {
    registry()
        .resolve_package("errorCode")
        .expect("errorCode package")
        .constants()
        .iter()
        .filter_map(|constant| Some((constant.value?, constant.name)))
        .collect()
}

/// The error codes an instruction stream loads, and the internal symbols it calls.
fn scan(
    instructions: &[CodeInstruction],
    relocations: &[CodeRelocation],
    codes: &BTreeMap<&'static str, &'static str>,
    raised: &mut BTreeSet<&'static str>,
    callees: &mut BTreeSet<String>,
) {
    for instruction in instructions {
        if let Some(name) = instruction
            .get("value")
            .and_then(|value| codes.get(value.as_str()))
        {
            raised.insert(name);
        }
    }
    for relocation in relocations {
        // External (libc/kernel32) calls report failure through errno, which the
        // body maps itself; `_mfb_str*` are the error-message data objects.
        if relocation.library.is_none() && !relocation.to.starts_with("_mfb_str") {
            callees.insert(relocation.to.clone());
        }
    }
}

/// Lower one implementation of an `fs` member on one backend and return the
/// emitted instructions and relocations.
///
/// A body refuses when an import or string literal it needs is not declared.
/// The plan derives both from the program, so supply each one it names and lower
/// again until it succeeds.
fn lower_body(
    function: &RegistryFunction,
    implementation: &Implementation,
    platform: &dyn CodegenPlatform,
) -> Result<(Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    let call = format!("fs.{}", function.name);
    let mut vregs = Vregs::new();
    let args: Vec<ValueResult> = implementation
        .params
        .iter()
        .map(|param| ValueResult {
            type_: param.ty.clone(),
            location: Operand::from(vregs.next().as_str()),
            text: param.name.to_string(),
            origin: None,
        })
        .collect();
    let mut harness = BuilderHarness::default();
    for _ in 0..64 {
        let mut builder = harness.builder("_mfb_rt_probe", platform);
        let ctx = AbiCtx {
            call: &call,
            ..harness.abi_ctx(platform)
        };
        let result = match implementation.body {
            Body::AbiFunction { lower, .. } | Body::AbiInline(lower) => {
                lower(&mut builder, &args, &ctx)
            }
            _ => Err(format!("{call}: not a native body")),
        };
        let error = match result {
            Ok(_) => return Ok((builder.instructions, builder.relocations)),
            Err(error) => error,
        };
        drop(builder);
        if let Some(import) = error
            .split_once("requires ")
            .and_then(|(_, rest)| rest.strip_suffix(" import"))
        {
            harness
                .platform_imports
                .insert(import.to_string(), "libc".to_string());
        } else if let Some(literal) = error
            .strip_prefix("native code string literal '")
            .and_then(|rest| rest.strip_suffix("' has no data object"))
        {
            harness
                .string_symbols
                .insert(literal.to_string(), "_mfb_str_probe".to_string());
        } else {
            return Err(error);
        }
    }
    Err(format!("{call}: still refusing after supplying 64 imports"))
}

/// Lower one member's every implementation on every backend and return the codes
/// its own instructions load plus the internal symbols it calls.
fn lower_member(
    function: &RegistryFunction,
    codes: &BTreeMap<&'static str, &'static str>,
    failures: &mut Vec<String>,
) -> (BTreeSet<&'static str>, BTreeSet<String>) {
    let mut raised = BTreeSet::new();
    let mut callees = BTreeSet::new();
    for (target, platform) in platforms() {
        for implementation in function.implementations() {
            match lower_body(function, implementation, platform.as_ref()) {
                Ok((instructions, relocations)) => scan(
                    &instructions,
                    &relocations,
                    codes,
                    &mut raised,
                    &mut callees,
                ),
                Err(error) => {
                    failures.push(format!("{target} fs::{} refused: {error}", function.name))
                }
            }
        }
    }
    (raised, callees)
}

/// Every `fs` member's descriptor `errors` equals the set of codes its lowering
/// can raise: its own raise sites, the shared errno mappers it emits, and the
/// raises of any helper whose Result it returns.
#[test]
fn every_fs_member_declares_exactly_the_errors_its_lowering_raises() {
    let codes = codes_by_value();
    let package = registry().resolve_package("fs").expect("fs package");
    let mut failures = Vec::new();

    let mut own = BTreeMap::new();
    for function in package.functions() {
        own.insert(function.name, lower_member(function, &codes, &mut failures));
    }

    let mut mismatched = Vec::new();
    for function in package.functions() {
        let (raised, callees) = &own[function.name];
        let mut expected: BTreeSet<&'static str> = raised.clone();
        for callee in callees {
            if RAISES_NOTHING.iter().any(|(symbol, _)| symbol == callee) {
                continue;
            }
            let Some((_, how)) = PROPAGATING.iter().find(|(symbol, _)| symbol == callee) else {
                failures.push(format!(
                    "fs::{} calls `{callee}`, which is in neither RAISES_NOTHING nor \
                     PROPAGATING — classify it",
                    function.name
                ));
                continue;
            };
            let inherited = match how {
                Propagating::Member(member) => own[member].0.clone(),
                Propagating::Helper(lower) => {
                    let (instructions, relocations) = lower();
                    let mut raised = BTreeSet::new();
                    scan(
                        &instructions,
                        &relocations,
                        &codes,
                        &mut raised,
                        &mut BTreeSet::new(),
                    );
                    raised
                }
            };
            for code in inherited {
                let unreachable = UNREACHABLE_VIA_CALLEE
                    .iter()
                    .any(|(caller, error, _)| *caller == function.name && *error == code);
                if !unreachable {
                    expected.insert(code);
                }
            }
        }

        let declared: BTreeSet<&'static str> = function
            .implementations()
            .iter()
            .flat_map(|implementation| implementation.errors.iter().copied())
            .collect();
        if declared != expected {
            let missing: Vec<_> = expected.difference(&declared).collect();
            let extra: Vec<_> = declared.difference(&expected).collect();
            mismatched.push(format!(
                "fs::{}: undeclared {missing:?}, declared but never raised {extra:?}",
                function.name
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the sweep could not lower or classify every fs member:\n  {}",
        failures.join("\n  ")
    );
    assert!(
        own.len() >= 42,
        "the sweep found only {} fs members; there were 42 when this was written",
        own.len()
    );
    assert!(
        mismatched.is_empty(),
        "{} fs member(s) declare errors that differ from what their lowering \
         raises (fix the descriptor `errors` in src/codegen/builtins/fs/func_*.rs):\n  {}",
        mismatched.len(),
        mismatched.join("\n  ")
    );
}

/// On Windows, `fs::canonicalPath` and `fs::isWithin` resolve a path by opening
/// it and asking the host for the handle's final path, as POSIX `realpath` does.
///
/// bug-694 C6: they called `GetFullPathNameW`, which only rewrites the string.
/// A path that did not exist canonicalized successfully, contradicting both pages
/// ("every component ... must exist"), and a symbolic link was never followed, so
/// `isWithin` compared link text rather than where the link leads.
#[test]
fn windows_canonicalization_resolves_through_an_open_handle() {
    let package = registry().resolve_package("fs").expect("fs package");
    let windows = crate::target::win_x86_64::code::Platform;
    let mut wrong = Vec::new();
    for name in ["canonicalPath", "isWithin"] {
        let function = package
            .functions()
            .iter()
            .find(|function| function.name == name)
            .expect("fs member");
        for implementation in function.implementations() {
            let (_, relocations) =
                lower_body(function, implementation, &windows).expect("lowers on Windows");
            let calls: BTreeSet<&str> = relocations
                .iter()
                .filter(|relocation| relocation.library.is_some())
                .map(|relocation| relocation.to.as_str())
                .collect();
            for required in ["CreateFileW", "GetFinalPathNameByHandleW", "CloseHandle"] {
                if !calls.contains(required) {
                    wrong.push(format!("fs::{name} never calls {required}"));
                }
            }
            if calls.contains("GetFullPathNameW") {
                wrong.push(format!(
                    "fs::{name} still canonicalizes lexically with GetFullPathNameW"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The host error codes each errno-mapping label may be reached from, per
/// platform family. POSIX errnos differ between Linux and macOS; Windows reports
/// `GetLastError` codes, a different numbering altogether.
fn host_codes(family: PlatformFamily, reason: &str) -> Option<&'static [&'static str]> {
    let codes: &'static [&'static str] = match (family, reason) {
        // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND
        (PlatformFamily::Windows, "not_found" | "path_not_found") => &["2", "3"],
        // ERROR_ACCESS_DENIED
        (PlatformFamily::Windows, "access_denied") => &["5"],
        // ERROR_FILE_EXISTS, ERROR_ALREADY_EXISTS
        (PlatformFamily::Windows, "already_exists") => &["80", "183"],
        // ERROR_DIR_NOT_EMPTY
        (PlatformFamily::Windows, "not_empty") => &["145"],
        // ERROR_INVALID_NAME, ERROR_FILENAME_EXCED_RANGE
        (PlatformFamily::Windows, "invalid_path") => &["123", "206"],
        // ENOENT
        (_, "not_found" | "path_not_found") => &["2"],
        // EACCES, and ELOOP from a no-follow open
        (PlatformFamily::Linux, "access_denied") => &["13", "40"],
        (_, "access_denied") => &["13", "62"],
        // EEXIST
        (_, "already_exists") => &["17"],
        // ENOTEMPTY
        (PlatformFamily::Linux, "not_empty") => &["39"],
        (_, "not_empty") => &["66"],
        // ENOTDIR, ENAMETOOLONG, EILSEQ, ELOOP
        (PlatformFamily::Linux, "invalid_path") => &["20", "36", "84", "40"],
        (_, "invalid_path") => &["20", "63", "92", "62"],
        _ => return None,
    };
    Some(codes)
}

/// Every host error code an `fs` body compares before branching to one of the
/// shared errno-mapping labels (`<symbol>_errno_<reason>`) is a code that
/// platform reports for that reason.
///
/// bug-694 C1: `emit_errno_error_mapping` compared the POSIX numbers 2/13/17 on
/// every backend, so on Windows — where the host reports `GetLastError` codes — a
/// missing directory (ERROR_PATH_NOT_FOUND, 3) or a refused write
/// (ERROR_ACCESS_DENIED, 5) fell through to `ErrWriteFailed`, while the unrelated
/// ERROR_INVALID_DATA (13) became `ErrAccessDenied`.
#[test]
fn every_fs_errno_mapping_compares_the_platforms_own_host_codes() {
    let package = registry().resolve_package("fs").expect("fs package");
    let mut wrong = Vec::new();
    let mut mapped = 0usize;
    for (target, platform) in platforms() {
        let family = platform.family();
        for function in package.functions() {
            for implementation in function.implementations() {
                let Ok((instructions, _)) = lower_body(function, implementation, platform.as_ref())
                else {
                    // A refusal is the declared-errors test's finding, not this one's.
                    continue;
                };
                // Each reason label, and the host codes that branch to it.
                let mut reached: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                for pair in instructions.windows(2) {
                    let (compare, branch) = (&pair[0], &pair[1]);
                    if compare.op.mnemonic() != "cmp_imm" || branch.op.mnemonic() != "b.eq" {
                        continue;
                    }
                    let (Some(value), Some(label)) = (compare.get("rhs"), branch.get("target"))
                    else {
                        continue;
                    };
                    let Some((_, reason)) = label.split_once("_errno_") else {
                        continue;
                    };
                    reached.entry(reason.to_string()).or_default().insert(value);
                }
                for (reason, values) in reached {
                    let Some(allowed) = host_codes(family, &reason) else {
                        continue;
                    };
                    mapped += 1;
                    let foreign: Vec<&String> = values
                        .iter()
                        .filter(|value| !allowed.contains(&value.as_str()))
                        .collect();
                    if !foreign.is_empty() {
                        wrong.push(format!(
                            "{target} fs::{}: `{reason}` is reached from host code(s) \
                             {foreign:?}, which this platform does not report for it \
                             (expected {allowed:?})",
                            function.name
                        ));
                    }
                }
            }
        }
    }
    assert!(
        mapped >= 100,
        "only {mapped} errno-mapping branches found; the instruction scan has broken"
    );
    assert!(
        wrong.is_empty(),
        "{} errno mapping(s) compare a host code the platform does not report:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}
