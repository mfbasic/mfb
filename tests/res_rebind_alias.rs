//! Regression test for bug-375: rebinding a live resource to a new `RES` name
//! registered a SECOND close obligation, so the callee closed the caller's
//! still-live handle on the way out.
//!
//! `builder_control` decided a `Bind`'s close obligation from the declared type
//! alone. The three non-owning escape hatches were `UnionExtract`,
//! `Capture { by_ref }`, and a collection-floated owner; an initializer that is
//! a plain reference to an existing resource matched none of them and fell
//! through to `ActiveCleanup::Resource`. So `RES g AS Db = d` inside a callee
//! manufactured an owner out of an alias, against §15.6: "a `RES` binding, a
//! `RES` parameter, and a collection slot all hold a copy of the one handle
//! pointer ... none of these close the resource; the owning scope closes it
//! exactly once on exit."
//!
//! `tests/rt-behavior/resources/res-rebind-alias-runtime` is the runtime proof
//! (its post-call use exits 255 with `7-703-0004` before the fix). These tests
//! count emitted close sites instead, because the two failure modes here pull in
//! OPPOSITE directions and only one of them is visible at runtime:
//!
//! * too narrow -> the premature close survives (the runtime fixture catches it)
//! * too wide   -> producers stop closing and bug-374's leak returns, which is
//!   invisible to an exit code and to a stdout golden (bug-374 measured it as
//!   5.84 GB -> 18.9 MB while passing either way)
//!
//! `alias_*` pins the first, `producing_*` the second. Build-only and
//! target-independent (cleanup registration is shared codegen), so these run on
//! any host via a cross-target build. No `libsqlite3` is loaded — nothing is
//! executed.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET: &str = "linux-x86_64";

/// A `LINK` block declaring a `Db` resource whose registered close op is
/// `sql.close`, plus whatever function bodies the caller appends. Mirrors
/// `native_resource_scope_drop.rs`, deliberately: these two files assert
/// opposite halves of the same branch and are read together.
fn program(body: &str) -> String {
    format!(
        "IMPORT io\n\n\
         RESOURCE Db CLOSE BY sql::close\n\n\
         LINK \"sqlite3\" AS sql\n\
        \x20 FUNC open(path AS String) AS RES Db\n\
        \x20   SYMBOL \"sqlite3_open\"\n\
        \x20   ABI (path CString, db OUT CPtr) AS status CInt32\n\
        \x20   RETURN db\n\
        \x20   SUCCESS_ON status = 0\n\
        \x20 END FUNC\n\
        \x20 FUNC close(RES db AS Db) AS Nothing\n\
        \x20   SYMBOL \"sqlite3_close\"\n\
        \x20   ABI (db CPtr) AS status CInt32\n\
        \x20   SUCCESS_ON status = 0\n\
        \x20 END FUNC\n\
         END LINK\n\n\
         {body}"
    )
}

fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
             \"kind\":\"executable\",\
             \"libraries\":{{\"sqlite3\":[\
             {{\"os\":\"macos\",\"type\":\"system\",\"source\":\"libsqlite3.dylib\"}},\
             {{\"os\":\"linux\",\"type\":\"system\",\"source\":\"libsqlite3.so.0\"}}]}},\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

/// Build `source` with `--ncode` and return the emitted code plan as text.
fn ncode(name: &str, source: &str) -> String {
    let project = temp_project(name, source);
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .args(["-target", TARGET])
        .arg("--ncode")
        .arg(&project)
        // `--ncode` writes `<name>.ncode` relative to the process cwd, so give
        // each test its own directory rather than racing in the repo root.
        .current_dir(&project)
        .output()
        .expect("run mfb build --ncode");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name} should build for {TARGET}:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let plan = fs::read_to_string(project.join(format!("{name}.ncode")))
        .expect("read emitted native code plan");
    let _ = fs::remove_dir_all(&project);
    plan
}

/// A function-level `"symbol"` key in the emitted plan. The leading newline +
/// four-space indent is load-bearing: individual instructions carry a `"symbol"`
/// field too, inline on their own line, so matching the bare key truncates every
/// slice well before the scope-drop cleanup at the end.
const FUNCTION_SYMBOL_KEY: &str = "\n    \"symbol\": \"";

/// The body of one function in the code plan, sliced between its function-level
/// `"symbol"` line and the next one.
fn function_body<'a>(plan: &'a str, symbol: &str) -> &'a str {
    let needle = format!("{FUNCTION_SYMBOL_KEY}{symbol}\",");
    let start = plan
        .find(&needle)
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}':\n{plan}"));
    let rest = &plan[start + needle.len()..];
    match rest.find(FUNCTION_SYMBOL_KEY) {
        Some(end) => &rest[..end],
        None => rest,
    }
}

/// How many times `body` *calls* `symbol`. Counts the branch instruction only:
/// each call also carries a relocation naming the same symbol, so a raw
/// substring count double-counts every site.
fn call_sites(body: &str, symbol: &str) -> usize {
    body.matches(&format!("\"op\": \"bl\", \"target\": \"{symbol}\""))
        .count()
}

const CLOSE_THUNK: &str = "_mfb_linker_sql_close";

fn assert_closes(name: &str, body: &str, function: &str, expected: usize, why: &str) {
    let plan = ncode(name, &program(body));
    let emitted = function_body(&plan, function);
    let closes = call_sites(emitted, CLOSE_THUNK);
    assert_eq!(
        closes, expected,
        "{function} should call {CLOSE_THUNK} {expected} time(s), found {closes}.\n{why}\n{emitted}"
    );
}

/// The bug report's own reproduction, as codegen: the callee binds the caller's
/// `RES` parameter to a new name and must emit NO close for it.
#[test]
fn alias_of_res_parameter_emits_no_close() {
    assert_closes(
        "b375_param_alias",
        "FUNC passThrough(RES d AS Db) AS Nothing\n\
        \x20 RES g AS Db = d\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 RES db AS Db = sql::open(\":memory:\")\n\
        \x20 passThrough(db)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        "_mfb_fn_passThrough",
        0,
        "bug-375: `RES g AS Db = d` registered an ActiveCleanup::Resource, so the \
         callee closed the CALLER's live handle and the caller's next use failed \
         with 7-703-0004.",
    );
}

/// The same alias in the owner's OWN scope: `main` produces the resource, so it
/// still closes exactly once — the alias adds nothing, it does not subtract.
/// This is the case that hides the bug at runtime (the premature close lands at
/// the owner's own exit anyway), which is why it is pinned by count here.
#[test]
fn alias_beside_its_owner_closes_exactly_once() {
    assert_closes(
        "b375_same_scope",
        "FUNC main() AS Integer\n\
        \x20 RES db AS Db = sql::open(\":memory:\")\n\
        \x20 RES g AS Db = db\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        "_mfb_fn_main",
        1,
        "The owner closes once; the alias must not add a second close (before the \
         fix this emitted 2).",
    );
}

/// NON-GOAL guard. A call returning `AS RES Db` TRANSFERS ownership to the
/// binding, so it must keep its cleanup. Too wide an alias rule silently
/// reintroduces bug-374's leak here.
#[test]
fn producing_bind_still_closes() {
    assert_closes(
        "b375_producer",
        "FUNC dropIt() AS Nothing\n\
        \x20 RES db AS Db = sql::open(\":memory:\")\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 dropIt()\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        "_mfb_fn_dropIt",
        1,
        "bug-374's leak: a producing bind must keep its close obligation.",
    );
}

/// NON-GOAL guard, the sharp edge. `RES f = <fallible> TRAP` lowers to
/// `bind f = local $trap_valN`, so at the NIR level a genuine producer is
/// indistinguishable-by-shape from an alias. A rule that treated every
/// local-initialized bind as an alias would stop closing every TRAP-bound
/// resource — a leak no exit code or stdout golden would ever show.
#[test]
fn trap_bound_producer_still_closes() {
    assert_closes(
        "b375_trap_producer",
        "FUNC dropIt() AS Integer\n\
        \x20 RES db AS Db = sql::open(\":memory:\") TRAP\n\
        \x20   RETURN 1\n\
        \x20 END TRAP\n\
        \x20 RETURN 0\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 RETURN dropIt()\n\
         END FUNC\n",
        "_mfb_fn_dropIt",
        3,
        "A TRAP-bound producer routes through a compiler temp that owns the \
         resource; every exit path must still close it. Three, not one: this is \
         one close per exit path (the TRAP handler's RETURN, the success RETURN, \
         and the error path), the same per-path counting \
         `native_resource_scope_drop.rs` pins at 2 for a two-exit function. \
         Before the fix this was 4 — the temp's obligation plus the duplicate one \
         the alias manufactured. Dropping to 0 here is the bug-374 leak.",
    );
}
