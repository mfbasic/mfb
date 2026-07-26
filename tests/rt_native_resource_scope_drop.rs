//! Regression test for bug-374: a user-declared `RESOURCE T CLOSE BY nativeOp`
//! emitted no close and no reclaim when its binding left scope.
//!
//! `resource_cleanup_symbol` resolved the close op only through
//! `builtins::resource_close_function`, an 8-entry table of the language's own
//! resources (`File`/`Socket`/...). A `RESOURCE Db CLOSE BY sql::close` missed
//! it, so the bind site in `builder_control` never pushed an
//! `ActiveCleanup::Resource` and scope exit had nothing to emit. The result was
//! silent: exit 0, correct output, and a leaked native handle per drop —
//! against the §15 guarantee whose own worked example is a native resource.
//!
//! `tests/rt-behavior/native/native-resource-scope-drop-rt` covers this at
//! runtime, but a leak is invisible to a stdout golden: that fixture peaked at
//! 2.34 GB before the fix and 8.5 MB after, and passed its golden either way.
//! These tests read the emitted code instead, so the guard does not depend on
//! anyone measuring RSS by hand.
//!
//! Build-only and target-independent (the cleanup registration is shared
//! codegen), so these run on any host via a cross-target build. No `libsqlite3`
//! is loaded — nothing is executed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET: &str = "linux-x86_64";

/// A `LINK` block declaring a `Db` resource whose registered close op is
/// `sql.close`, plus whatever function bodies the caller appends.
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

/// A function-level `"symbol"` key in the emitted plan.
///
/// The leading newline + four-space indent is load-bearing: individual
/// instructions carry a `"symbol"` field too (`{ "op": "adrp", "dst": "r10",
/// "symbol": "_mfb_str_0" }`), inline on their own line. Matching the bare key
/// truncated every slice at the function's first `adrp`, which is well before
/// the scope-drop cleanup at the end — so these tests reported zero closes
/// against a plan that in fact contained them.
const FUNCTION_SYMBOL_KEY: &str = "\n    \"symbol\": \"";

/// The body of one function in the code plan, sliced between its function-level
/// `"symbol"` line and the next one. The plan is JSON, but the assertions here
/// are call-site counts over a single function's text, so a slice beats pulling
/// in a parser — and it panics if the function is absent.
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

/// The `sql.close` thunk symbol, i.e. what a scope-drop close must branch to.
/// `link_thunk_symbol` escapes every non-alphanumeric byte, and neither `sql`
/// nor `close` has one, so the name is stable.
const CLOSE_THUNK: &str = "_mfb_linker_sql_close";
const OPEN_THUNK: &str = "_mfb_linker_sql_open";

fn assert_drops(name: &str, body: &str, function: &str, expected_closes: usize) {
    let plan = ncode(name, &program(body));
    let emitted = function_body(&plan, function);
    let closes = call_sites(emitted, CLOSE_THUNK);
    assert_eq!(
        closes, expected_closes,
        "{function} should call {CLOSE_THUNK} on {expected_closes} exit path(s), \
         found {closes}.\nbug-374: a user resource registered no scope cleanup at \
         all, so this count was 0.\n{emitted}"
    );
    // The close alone is not the fix: the 80-byte record itself must also be
    // reclaimed, which is the second half of what `ActiveCleanup::Resource`
    // emits and what distinguishes a registered cleanup from an incidental
    // explicit close.
    assert!(
        emitted.contains("resource_cleanup_reclaim"),
        "{function} should emit the record reclaim after the close.\n{emitted}"
    );
}

/// §15's "normal scope exit" — the bug report's own reproduction.
#[test]
fn native_resource_closes_on_normal_scope_exit() {
    assert_drops(
        "b374_normal",
        "FUNC dropIt() AS Nothing\n\
        \x20 RES db AS Db = sql::open(\":memory:\")\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 dropIt()\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        "_mfb_fn_dropIt",
        1,
    );
}

/// §15's "RETURN": an early return must run the same cleanup the fall-through
/// path does, so both exit paths carry a close.
#[test]
fn native_resource_closes_on_early_return() {
    assert_drops(
        "b374_return",
        "FUNC dropIt(n AS Integer) AS Integer\n\
        \x20 RES db AS Db = sql::open(\":memory:\")\n\
        \x20 IF n > 0 THEN\n\
        \x20   RETURN 1\n\
        \x20 END IF\n\
        \x20 RETURN 0\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 RETURN dropIt(1)\n\
         END FUNC\n",
        "_mfb_fn_dropIt",
        2,
    );
}

/// §15's "EXIT": the binding lives in the loop BODY, so it is a fresh resource
/// per iteration and `EXIT WHILE` leaves the scope holding a live one.
#[test]
fn native_resource_closes_on_loop_exit() {
    assert_drops(
        "b374_exit",
        "FUNC dropIt() AS Nothing\n\
        \x20 MUT i AS Integer = 0\n\
        \x20 WHILE i < 3\n\
        \x20   RES db AS Db = sql::open(\":memory:\")\n\
        \x20   IF i = 1 THEN\n\
        \x20     EXIT WHILE\n\
        \x20   END IF\n\
        \x20   i = i + 1\n\
        \x20 END WHILE\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 dropIt()\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        "_mfb_fn_dropIt",
        2,
    );
}

/// §15's "FAIL": the failure unwinds out of a scope holding a live resource,
/// and the close happens on the way out rather than being skipped because the
/// scope is leaving abnormally.
#[test]
fn native_resource_closes_on_fail() {
    assert_drops(
        "b374_fail",
        "FUNC dropIt() AS Nothing\n\
        \x20 RES db AS Db = sql::open(\":memory:\")\n\
        \x20 FAIL error(77050002, \"boom\")\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 dropIt()\n\
        \x20 RETURN 0\n\
        \x20 TRAP(err)\n\
        \x20   RETURN 0\n\
        \x20 END TRAP\n\
         END FUNC\n",
        "_mfb_fn_dropIt",
        1,
    );
}

/// The non-goal guard: a resource closed EXPLICITLY must still be RECLAIMED at
/// scope exit, and the resulting second close must stay harmless.
///
/// bug-374's fix adds a drop-close to every native program in the tree, all of
/// which already close explicitly — so each of them now closes twice. That is
/// safe and, more than that, the second one is load-bearing:
///
///   - Safe, because plan-59-B's `closed` flag makes the second close a defined
///     `ERR_RESOURCE_CLOSED` no-op, which `emit_resource_cleanup_call` already
///     treats as benign on the drop path rather than a cleanup failure.
///   - Load-bearing, because a `LINK` close thunk only sets `RESOURCE_CLOSED_BIT`
///     — unlike the built-in `fs.close` runtime helper, it never frees the
///     80-byte record. The scope-exit cleanup's `emit_resource_block_reclaim` is
///     the ONLY thing that reclaims it, on the explicit-close path too.
///
/// That is why `deactivate_moved_resource_arguments` is deliberately NOT extended
/// to user resources alongside `resource_cleanup_symbol`: retiring the cleanup at
/// the explicit close would drop the reclaim with it and leak a record per close.
/// It also explains why a native function emits one more close site here than the
/// built-in `File` equivalent, which can retire its cleanup because its helper
/// reclaims internally.
#[test]
fn explicitly_closed_native_resource_is_still_reclaimed() {
    let plan = ncode(
        "b374_explicit",
        &program(
            "FUNC closeIt() AS Nothing\n\
            \x20 RES db AS Db = sql::open(\":memory:\")\n\
            \x20 sql::close(db)\n\
             END FUNC\n\n\
             FUNC main() AS Integer\n\
            \x20 closeIt()\n\
            \x20 RETURN 0\n\
             END FUNC\n",
        ),
    );
    let emitted = function_body(&plan, "_mfb_fn_closeIt");
    let opens = call_sites(emitted, OPEN_THUNK);
    let closes = call_sites(emitted, CLOSE_THUNK);
    assert_eq!(opens, 1, "expected a single open call site.\n{emitted}");
    assert!(
        closes > opens,
        "an explicitly-closed resource should still carry its scope-exit cleanup \
         (the only thing that reclaims the record), so closes ({closes}) must \
         exceed opens ({opens}).\n{emitted}"
    );
    assert!(
        emitted.contains("resource_cleanup_reclaim"),
        "the record must still be reclaimed after an explicit close.\n{emitted}"
    );
}

/// The fixture that exercises all of the above at runtime exists and still
/// relies on drop rather than an explicit close. If someone "tidies" it by
/// adding closes, it stops covering the bug and this says so.
#[test]
fn runtime_fixture_still_relies_on_scope_drop() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/rt-behavior/native/native-resource-scope-drop-rt/src/main.mfb");
    let source = fs::read_to_string(&fixture).expect("read native-resource-scope-drop-rt");
    // `closeExplicitly` is the one function that is *supposed* to close, and it
    // is the only permitted `sql::close` call in the fixture.
    let explicit = source.matches("sql::close(").count();
    assert_eq!(
        explicit, 1,
        "native-resource-scope-drop-rt must keep exactly one explicit close \
         (in closeExplicitly); every other path must rely on scope drop, which \
         is the behavior bug-374 restored."
    );
}
