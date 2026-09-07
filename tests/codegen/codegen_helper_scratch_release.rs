//! bug-574: who owns the marshalling **scratch** a fixed runtime helper allocates
//! for its own use, asserted on the emitted code.
//!
//! A `fs::`/`os::`/`net::`/`udp::`/`tcp::` call copies its `String` argument into a
//! fresh NUL-terminated arena block so the host call can read it. That block is not
//! a `ValueResult` any node yielded — no `Bind`, no statement-scope drop and no
//! `TRAP` desugar can see it — so nothing freed it and every such call leaked its
//! argument, in proportion to the argument's LENGTH (a 415-byte path cost ~1 819 B
//! per call; a zero-argument helper was flat).
//!
//! The RSS cases in `tests/runtime/rt_scope_drop_leaks.rs` prove the leak is gone.
//! They cannot prove the *shape* of the fix, and the shape is what keeps it sound:
//!
//! * **No free** is the leak, and nothing goes red.
//! * **An unguarded free** frees a pointer that was never allocated on the paths
//!   that reach the helper's `done` without allocating — the `ErrOutOfMemory` tail,
//!   an empty-path rejection, `net::listen`'s bind-all host that jumps straight past
//!   its `emit_cstring`. A behavioural probe sees that only if the garbage pointer
//!   happens to corrupt something it then reads.
//! * **Freeing the RESULT** is a use-after-free the caller performs: `fs::readText`
//!   allocates the marshalled path AND the `String` it returns, and only the first
//!   is scratch.
//!
//! So the assertion is a per-symbol table of `(alloc, free, guarded-free)` counts
//! over the packages' own `codegen_cover` fixtures, and the table's key set is
//! asserted to BE the set of that package's runtime helpers. A new member, or a new
//! allocation inside an existing one, changes a triple or adds a key and reds this
//! file — the decision cannot be inherited.
//!
//! Build-only `-ncode` cross-built for `linux-x86_64`, matching the sibling
//! codegen-inspection suites: ownership is target-independent codegen.

#[path = "../common/mod.rs"]
mod common;

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const TARGET: &str = "linux-x86_64";

/// `(allocations, frees, guarded scratch releases)` for one emitted function.
type Counts = (usize, usize, usize);

/// Copy the package's `codegen_cover` byte-identity fixture into a scratch
/// directory and dump its code plan. The fixture is copied rather than built in
/// place because `-ncode` writes its dump beside `project.json`.
fn cover_plan(package: &str) -> Value {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/byte-identity")
        .join(package);
    let project = common::temp_project(&format!("b574_cover_{package}"), "");
    copy_tree(&fixture, &project);
    let plan = common::build_ncode(&project, TARGET, &format!("{package}_codegen_cover_rt"));
    let _ = std::fs::remove_dir_all(&project);
    plan
}

fn copy_tree(from: &Path, to: &PathBuf) {
    for entry in std::fs::read_dir(from).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            std::fs::create_dir_all(&target).expect("create fixture subdirectory");
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}

/// Every `_mfb_rt_<package>_*` function in `plan`, with its counts.
///
/// The prefix filter is what makes the key-set assertion meaningful: it selects
/// exactly the package's own runtime helpers and none of the shared runtime
/// (`_mfb_rt_park_error`, `_mfb_rt_int_to_string`, …) or the user function, whose
/// allocation counts move for unrelated reasons.
fn helper_counts(plan: &Value, package: &str) -> BTreeMap<String, Counts> {
    let prefix = format!("_mfb_rt_{package}_");
    let mut out = BTreeMap::new();
    for function in plan["functions"].as_array().expect("functions array") {
        let symbol = function["symbol"].as_str().expect("symbol").to_string();
        if !symbol.starts_with(&prefix) {
            continue;
        }
        let instructions = function["instructions"]
            .as_array()
            .expect("instructions array");
        let calls = |target: &str| {
            instructions
                .iter()
                .filter(|i| i["op"].as_str() == Some("bl") && i["target"].as_str() == Some(target))
                .count()
        };
        out.insert(
            symbol,
            (
                calls("_mfb_arena_alloc"),
                calls("_mfb_arena_free"),
                guarded_scratch_releases(instructions),
            ),
        );
    }
    out
}

/// How many `..._scratch_kept_N` labels this function carries, checking each one
/// is the tail of a complete guard-and-free sequence:
///
/// ```text
///   compare  <scratch pointer>, 0
///   branch_eq  <symbol>_scratch_kept_N
///   move  ARG[0], <scratch pointer>
///   move  ARG[1], <scratch size>
///   bl  _mfb_arena_free
/// <symbol>_scratch_kept_N:
/// ```
///
/// This is the assertion that the free is a **runtime pointer guard** and not a
/// whole-program claim: a helper that reaches `done` without having allocated
/// carries a null there and frees nothing. A refactor that dropped the compare
/// would leave the label and the free in place and pass a count-only check.
fn guarded_scratch_releases(instructions: &[Value]) -> usize {
    let op = |index: usize| instructions[index]["op"].as_str().unwrap_or_default();
    let target = |index: usize| instructions[index]["target"].as_str().unwrap_or_default();
    let mut guarded = 0;
    for (index, instruction) in instructions.iter().enumerate() {
        let Some(name) = instruction["name"].as_str() else {
            continue;
        };
        if instruction["op"].as_str() != Some("label") || !name.contains("_scratch_kept_") {
            continue;
        }
        // The call immediately before the label — reload/staging instructions may
        // sit between them, but no other call and no other label may.
        let call = (0..index)
            .rev()
            .find(|candidate| op(*candidate) == "bl")
            .unwrap_or_else(|| panic!("{name}: no call precedes the scratch label"));
        assert_eq!(
            target(call),
            "_mfb_arena_free",
            "{name}: the instruction the guard jumps over must be \
             `_mfb_arena_free`, not `{}`",
            target(call),
        );
        // The branch that skips that call must be a conditional on the scratch
        // pointer being null, and must target THIS label. That is the whole
        // safety argument: a helper that reaches `done` without allocating (the
        // `ErrOutOfMemory` tail, an empty-path rejection, `net::listen`'s
        // bind-all host) carries a null and frees nothing.
        let guard = (0..call)
            .rev()
            .find(|candidate| op(*candidate).starts_with("b.") || op(*candidate) == "label")
            .unwrap_or_else(|| panic!("{name}: no guard branch precedes the free"));
        assert_eq!(
            op(guard),
            "b.eq",
            "{name}: the scratch free must be guarded by an equality branch, not \
             `{}` — an UNGUARDED free releases a pointer the OOM path never \
             allocated",
            op(guard),
        );
        assert_eq!(
            target(guard),
            name,
            "{name}: the scratch guard must branch to its OWN label"
        );
        assert_eq!(
            op(guard - 1),
            "cmp_imm",
            "{name}: the scratch guard must be preceded by a compare"
        );
        assert_eq!(
            instructions[guard - 1]["rhs"].as_str(),
            Some("0"),
            "{name}: the scratch guard must compare the pointer against 0"
        );
        guarded += 1;
    }
    guarded
}

/// Assert `package`'s runtime helpers are exactly `expected`, counts included.
fn assert_package(package: &str, expected: &[(&str, Counts)]) {
    let actual = helper_counts(&cover_plan(package), package);
    let expected: BTreeMap<String, Counts> = expected
        .iter()
        .map(|(symbol, counts)| ((*symbol).to_string(), *counts))
        .collect();
    assert_eq!(
        actual.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "the set of `{package}` runtime helpers changed. Every new one must be \
         classified here: does it copy a `String` argument into an arena block for \
         the host call, and if so does it release that block at its `done` \
         (bug-574)? Inheriting the answer is how the whole family leaked."
    );
    for (symbol, counts) in &expected {
        assert_eq!(
            actual.get(symbol),
            Some(counts),
            "{symbol}: (arena_alloc, arena_free, guarded scratch release) counts \
             changed. A NEW allocation with no matching free is bug-574 again — \
             marshalling scratch nothing on the caller side can see. A new FREE \
             with no guard, or one covering the block the helper RETURNS, is a \
             use-after-free"
        );
    }
}

/// `fs` is the family the report measured: `fs::exists` leaked ~65 B per call for
/// a 10-character path and ~1 819 B for a 415-character one.
///
/// The rows that are NOT `(n, n, n)` are the interesting ones, and each is a
/// deliberate answer:
///
/// * `openFile`/`open`/`openFileNoFollow`/`createTempFile` allocate the path
///   scratch AND the `File` record they hand back — one free.
/// * `readText`/`listDirectory`/`currentDirectory`/`tempDirectory` allocate the
///   scratch (or the `getcwd` buffer) AND the `String`/`List` result — one free.
/// * `canonicalPath` allocates the path scratch, the PATH_MAX `realpath` buffer,
///   and the result — two frees.
/// * `openWithin` allocates the root C-string, the PATH_MAX join buffer, and the
///   `File` record — two frees.
/// * `readBytes` allocates the path scratch and an INTERNAL `File` record it only
///   uses to call `readAllBytes` through — both are scratch, both freed.
/// * `readLine` has five allocations (the lazily-created per-`File` read buffer,
///   which the `File` owns; the line accumulator; up to two accumulator regrows;
///   the result `String`) and three frees: the accumulator's guarded release plus
///   the two regrow paths handing back the block they just drained.
/// * `readAll`/`readAllBytes`/`writeAll`/`writeAllBytes`/`path_join` take a `File`
///   or a `List OF String` — no `String` argument to marshal, one result block.
const FS_HELPERS: &[(&str, Counts)] = &[
    ("_mfb_rt_fs_file_drain", (0, 0, 0)),
    ("_mfb_rt_fs_fs_appendBytes", (1, 1, 1)),
    ("_mfb_rt_fs_fs_appendText", (1, 1, 1)),
    ("_mfb_rt_fs_fs_canonicalPath", (3, 2, 2)),
    ("_mfb_rt_fs_fs_close", (0, 0, 0)),
    ("_mfb_rt_fs_fs_createDirectories", (1, 1, 1)),
    ("_mfb_rt_fs_fs_createDirectory", (1, 1, 1)),
    ("_mfb_rt_fs_fs_createTempFile", (2, 1, 1)),
    ("_mfb_rt_fs_fs_currentDirectory", (2, 1, 1)),
    ("_mfb_rt_fs_fs_deleteDirectory", (1, 1, 1)),
    ("_mfb_rt_fs_fs_deleteFile", (1, 1, 1)),
    ("_mfb_rt_fs_fs_directoryExists", (1, 1, 1)),
    ("_mfb_rt_fs_fs_eof", (0, 0, 0)),
    ("_mfb_rt_fs_fs_exists", (1, 1, 1)),
    ("_mfb_rt_fs_fs_fileExists", (1, 1, 1)),
    ("_mfb_rt_fs_fs_flush", (0, 0, 0)),
    ("_mfb_rt_fs_fs_isBuffered", (0, 0, 0)),
    ("_mfb_rt_fs_fs_isWithin", (4, 4, 4)),
    ("_mfb_rt_fs_fs_listDirectory", (2, 1, 1)),
    ("_mfb_rt_fs_fs_open", (2, 1, 1)),
    ("_mfb_rt_fs_fs_openFile", (2, 1, 1)),
    ("_mfb_rt_fs_fs_openFileNoFollow", (2, 1, 1)),
    ("_mfb_rt_fs_fs_openWithin", (3, 2, 2)),
    ("_mfb_rt_fs_fs_readAll", (1, 0, 0)),
    ("_mfb_rt_fs_fs_readAllBytes", (1, 0, 0)),
    ("_mfb_rt_fs_fs_readBytes", (2, 2, 2)),
    ("_mfb_rt_fs_fs_readLine", (5, 3, 1)),
    ("_mfb_rt_fs_fs_readText", (2, 1, 1)),
    ("_mfb_rt_fs_fs_setBuffered", (0, 0, 0)),
    ("_mfb_rt_fs_fs_setCurrentDirectory", (1, 1, 1)),
    ("_mfb_rt_fs_fs_tempDirectory", (2, 1, 1)),
    ("_mfb_rt_fs_fs_writeAll", (1, 0, 0)),
    ("_mfb_rt_fs_fs_writeAllBytes", (1, 0, 0)),
    ("_mfb_rt_fs_fs_writeBytes", (1, 1, 1)),
    ("_mfb_rt_fs_fs_writeBytesAtomic", (3, 3, 3)),
    ("_mfb_rt_fs_fs_writeText", (1, 1, 1)),
    ("_mfb_rt_fs_fs_writeTextAtomic", (3, 3, 3)),
    ("_mfb_rt_fs_path_join", (1, 0, 0)),
];

/// `os`'s environment family marshals through the shared `marshal_cstring`.
/// `setEnv` marshals TWO arguments and frees both; `getEnvOr` allocates the
/// scratch, the result `String` built from `getenv`'s answer, and the fallback
/// copy — one scratch. The rest of the package takes no `String` argument and
/// allocates only its result: `args`, `environ`, `hostName`, `userName`,
/// `executablePath`, `name`, `arch`, `resourcePath`. `os::arch()` being flat is
/// the report's own contrast row.
const OS_HELPERS: &[(&str, Counts)] = &[
    ("_mfb_rt_os_os_arch", (1, 0, 0)),
    ("_mfb_rt_os_os_args", (1, 0, 0)),
    ("_mfb_rt_os_os_cpuCount", (0, 0, 0)),
    ("_mfb_rt_os_os_environ", (1, 0, 0)),
    ("_mfb_rt_os_os_executablePath", (1, 0, 0)),
    ("_mfb_rt_os_os_getEnv", (2, 1, 1)),
    ("_mfb_rt_os_os_getEnvOr", (3, 1, 1)),
    ("_mfb_rt_os_os_hasEnv", (1, 1, 1)),
    ("_mfb_rt_os_os_hostName", (1, 0, 0)),
    ("_mfb_rt_os_os_name", (1, 0, 0)),
    ("_mfb_rt_os_os_pid", (0, 0, 0)),
    ("_mfb_rt_os_os_resourcePath", (1, 0, 0)),
    ("_mfb_rt_os_os_setEnv", (2, 2, 2)),
    ("_mfb_rt_os_os_unsetEnv", (1, 1, 1)),
    ("_mfb_rt_os_os_userName", (1, 0, 0)),
];

/// `net`'s three resolvers each marshal the host name for `getaddrinfo`; the rest
/// of their allocations build the `List OF net::Address` / `net::PingResult` they
/// return.
const NET_HELPERS: &[(&str, Counts)] = &[
    ("_mfb_rt_net_net_lookup", (5, 1, 1)),
    ("_mfb_rt_net_net_ping", (7, 1, 1)),
    ("_mfb_rt_net_net_pingAddr", (7, 1, 1)),
];

/// `tcp::connect`/`connectAddr`/`listen` all route through the shared endpoint
/// helper and marshal their host; `accept`, `read`, `localAddress`,
/// `remoteAddress` and `pollList` take no `String` argument. `pollList`'s three
/// frees are its own poll-array bookkeeping, which predates this change.
const TCP_HELPERS: &[(&str, Counts)] = &[
    ("_mfb_rt_tcp_tcp_accept", (1, 0, 0)),
    ("_mfb_rt_tcp_tcp_close", (0, 0, 0)),
    ("_mfb_rt_tcp_tcp_connect", (2, 1, 1)),
    ("_mfb_rt_tcp_tcp_connectAddr", (2, 1, 1)),
    ("_mfb_rt_tcp_tcp_listen", (2, 1, 1)),
    ("_mfb_rt_tcp_tcp_localAddress", (3, 0, 0)),
    ("_mfb_rt_tcp_tcp_poll", (0, 0, 0)),
    ("_mfb_rt_tcp_tcp_pollList", (1, 3, 0)),
    ("_mfb_rt_tcp_tcp_read", (2, 0, 0)),
    ("_mfb_rt_tcp_tcp_remoteAddress", (3, 0, 0)),
    ("_mfb_rt_tcp_tcp_setReadTimeout", (0, 0, 0)),
    ("_mfb_rt_tcp_tcp_setWriteTimeout", (0, 0, 0)),
    ("_mfb_rt_tcp_tcp_write", (0, 0, 0)),
    ("_mfb_rt_tcp_tcp_writeText", (0, 0, 0)),
];

/// `udp::bind` marshals its bind host; `send`/`sendText` marshal the destination
/// host out of the `net::Address` they are handed. `receive` and `localAddress`
/// build only results.
const UDP_HELPERS: &[(&str, Counts)] = &[
    ("_mfb_rt_udp_udp_bind", (2, 1, 1)),
    ("_mfb_rt_udp_udp_close", (0, 0, 0)),
    ("_mfb_rt_udp_udp_localAddress", (3, 0, 0)),
    ("_mfb_rt_udp_udp_poll", (0, 0, 0)),
    ("_mfb_rt_udp_udp_pollList", (1, 3, 0)),
    ("_mfb_rt_udp_udp_receive", (6, 0, 0)),
    ("_mfb_rt_udp_udp_send", (1, 1, 1)),
    ("_mfb_rt_udp_udp_sendText", (1, 1, 1)),
    ("_mfb_rt_udp_udp_setReadTimeout", (0, 0, 0)),
    ("_mfb_rt_udp_udp_setWriteTimeout", (0, 0, 0)),
];

#[test]
fn every_fs_helper_releases_the_path_it_marshalled() {
    assert_package("fs", FS_HELPERS);
}

#[test]
fn every_os_helper_releases_the_name_it_marshalled() {
    assert_package("os", OS_HELPERS);
}

#[test]
fn every_net_helper_releases_the_host_it_marshalled() {
    assert_package("net", NET_HELPERS);
}

#[test]
fn every_tcp_helper_releases_the_host_it_marshalled() {
    assert_package("tcp", TCP_HELPERS);
}

#[test]
fn every_udp_helper_releases_the_host_it_marshalled() {
    assert_package("udp", UDP_HELPERS);
}

/// The residue, pinned so it cannot be mistaken for done.
///
/// `tls::` marshals its host name and certificate paths through the OTHER
/// `emit_cstring` (`builtins/tls/gen_shared.rs`), across twelve call sites in the
/// OpenSSL, Secure Transport and Schannel backends, two of which sit in shared
/// sub-emitters (`emit_read_whole_file`, `socket_connect`) whose `done` belongs to
/// a caller. None of them was converted here: the three backends are mutually
/// exclusive per platform, two of the three cannot be exercised on this host, and a
/// scratch release placed after a branch that reaches `done` frees an undefined
/// vreg. It is filed as bug-575 with this enumeration.
///
/// The assertion is that the count has not silently CHANGED — a new `tls`
/// marshalling site is a new leak, and a converted one should be moved out of this
/// count and into a table above.
#[test]
fn the_tls_marshalling_sites_are_the_known_residue() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/codegen/builtins/tls/gen_shared.rs"),
    )
    .expect("read tls/gen_shared.rs");
    assert!(
        source.contains("pub(crate) fn emit_cstring("),
        "the `tls` marshaller was renamed; re-derive the residue"
    );
    let mut sites = 0;
    for entry in walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/codegen/builtins/tls")) {
        let text = std::fs::read_to_string(&entry).expect("read tls source");
        sites += text
            .lines()
            .filter(|line| line.contains("emit_cstring(") && !line.contains("fn emit_cstring("))
            .count();
    }
    assert_eq!(
        sites, 12,
        "the number of `tls` C-string marshalling sites changed. Each one leaks its \
         argument (bug-575); converting one means releasing its block at the \
         enclosing helper's `done` and moving the row into this file's per-package \
         tables"
    );
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).expect("read source directory") {
        let entry = entry.expect("source entry");
        if entry.file_type().expect("file type").is_dir() {
            out.extend(walk(&entry.path()));
        } else if entry.path().extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(entry.path());
        }
    }
    out
}
