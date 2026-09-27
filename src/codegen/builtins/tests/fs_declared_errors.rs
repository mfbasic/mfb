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

use super::declared_errors_sweep::{
    assert_package_declares_exactly_the_errors_its_lowering_raises, lower_body, platforms,
    Propagating,
};
use crate::codegen::engine::types::PlatformFamily;
use crate::codegen::registry::registry;
use std::collections::{BTreeMap, BTreeSet};

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
    (
        crate::codegen::builtins::fs::gen_classify::FS_PATH_CLASS_SYMBOL,
        "returns a path class in x0; the caller's host-error mapping raises (bug-701)",
    ),
];

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
const UNREACHABLE_VIA_CALLEE: &[(&str, &str, &str)] = &[
    (
        "readBytes",
        "ErrResourceClosed",
        "readBytes hands readAllBytes the File record it just opened, with \
         FILE_OFFSET_CLOSED stored as 0, so the closed-handle check cannot fire",
    ),
    (
        "readBytes",
        "ErrResourceMoved",
        "the same fresh record: its flag word is 0, so the moved-handle check \
         (bug-702) cannot fire either",
    ),
];

/// Every `fs` member's descriptor `errors` equals the set of codes its lowering
/// can raise: its own raise sites, the shared errno mappers it emits, and the
/// raises of any helper whose Result it returns.
#[test]
fn every_fs_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "fs",
        RAISES_NOTHING,
        PROPAGATING,
        UNREACHABLE_VIA_CALLEE,
        42,
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
                lower_body("fs", function, implementation, &windows).expect("lowers on Windows");
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
    // bug-701: one mapping for every member (`emit_fs_host_error_mapping`). The
    // `classify_*` labels ask `_mfb_rt_fs_path_class` which condition the host
    // meant where its code names two.
    let codes: &'static [&'static str] = match (family, reason) {
        // ERROR_FILE_NOT_FOUND; ERROR_PATH_NOT_FOUND when there is no path to ask
        (PlatformFamily::Windows, "path_not_found") => &["2", "3"],
        // ERROR_PATH_NOT_FOUND: a missing parent, a file used as a directory, or
        // an over-long name
        (PlatformFamily::Windows, "classify_missing") => &["3"],
        // ERROR_ACCESS_DENIED: a refusal, or a directory where a file is required
        (PlatformFamily::Windows, "access_denied" | "classify_directory") => &["5"],
        // ERROR_FILE_EXISTS, ERROR_ALREADY_EXISTS
        (PlatformFamily::Windows, "already_exists") => &["80", "183"],
        // ERROR_DIR_NOT_EMPTY
        (PlatformFamily::Windows, "not_empty") => &["145"],
        // ERROR_INVALID_NAME, ERROR_FILENAME_EXCED_RANGE, ERROR_DIRECTORY,
        // ERROR_CANT_RESOLVE_FILENAME
        (PlatformFamily::Windows, "invalid_path") => &["123", "206", "267", "1921"],
        // ENOENT
        (_, "path_not_found") => &["2"],
        // EACCES, EPERM, and ELOOP from a no-follow open
        (PlatformFamily::Linux, "access_denied") => &["13", "1", "40"],
        (_, "access_denied") => &["13", "1", "62"],
        // EPERM from `unlink`: macOS's answer for a directory
        (_, "classify_directory") => &["1"],
        // EEXIST
        (_, "already_exists") => &["17"],
        // ENOTEMPTY, and the EEXIST POSIX allows `rmdir` to give for it
        (PlatformFamily::Linux, "not_empty") => &["39", "17"],
        (_, "not_empty") => &["66", "17"],
        // ENOTDIR, EISDIR, ENAMETOOLONG, EILSEQ, ELOOP
        (PlatformFamily::Linux, "invalid_path") => &["20", "21", "36", "84", "40"],
        (_, "invalid_path") => &["20", "21", "63", "92", "62"],
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
                let Ok((instructions, _)) =
                    lower_body("fs", function, implementation, platform.as_ref())
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
