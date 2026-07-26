//! Regression tests for bug-377: an imported package's resource was invisible
//! to the importing project, so it was neither closed nor checked.
//!
//! `tests/native_resource_scope_drop.rs` is the same guard for a resource the
//! project declares itself (bug-374). That is precisely why this hole survived:
//! bug-374's fix reached `RESOURCE T CLOSE BY op` in the current project, and
//! its tests only ever exercised that shape. Everything keyed on a resource
//! being *known* stayed inert across a package boundary, because a decoded
//! package carries no `native_resources` (`ir/binary.rs` drops them by
//! contract) and nothing repopulated them in the consumer.
//!
//! Three independent failures rode on that one gap:
//!
//!   1. **The handle leaked.** `code::validation` registered the close op under
//!      `<package>.<close>`, but `merge_packages` identity-prefixes every
//!      imported symbol, so `resource_cleanup_symbol`'s lookup missed and no
//!      cleanup was ever registered. Scope exit emitted nothing at all.
//!   2. **The close thunk never set `RESOURCE_CLOSED_BIT`**, because the
//!      thunk's close-op set also came from the empty `native_resources`. Once
//!      (1) was fixed this turned into a double free into the C library.
//!   3. **`ir::verify` could not see the type was a resource**, so the RES
//!      ownership axis and move tracking silently skipped it.
//!
//! These read the emitted code and the build's diagnostics rather than running
//! anything: a leak is invisible to a stdout golden, which is how the runtime
//! fixture `rt-behavior/native/native-link-import-sqlite-rt` passed throughout.
//!
//! The package under test is the committed `sqlite3.mfp` that fixture already
//! ships, so these are hermetic and need no package build, install, or
//! signature. No `libsqlite3` is loaded — nothing is executed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET: &str = "linux-x86_64";

/// The committed `.mfp` the sqlite runtime fixture consumes. It exports
/// `RESOURCE Db CLOSE BY sqliteLink::close` plus a `close` re-export alias,
/// which is the shape plan-link-update.md §5a describes.
fn fixture_package() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/rt-behavior/native/native-link-import-sqlite-rt/packages/sqlite3.mfp")
}

/// A consumer project with `sqlite3.mfp` installed. The dependency entry names
/// no `source`, which `verify_and_report_packages` treats as local and so
/// permits unsigned.
fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::create_dir_all(root.join("packages")).expect("create packages dir");
    fs::copy(fixture_package(), root.join("packages/sqlite3.mfp")).expect("install sqlite3.mfp");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
             \"kind\":\"executable\",\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"packages\":[{{\"name\":\"sqlite3\",\"version\":\"=0.1.0\"}}],\
             \"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

struct Build {
    stdout: String,
    success: bool,
    project: PathBuf,
    name: String,
}

impl Build {
    fn plan(&self) -> String {
        fs::read_to_string(self.project.join(format!("{}.ncode", self.name)))
            .expect("read emitted native code plan")
    }
}

impl Drop for Build {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.project);
    }
}

/// Build `source` with `--ncode`, returning the build's outcome rather than
/// asserting success — one of these tests wants the build to FAIL.
fn build(name: &str, source: &str) -> Build {
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
    Build {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr),
        success: output.status.success(),
        project,
        name: name.to_string(),
    }
}

/// A function-level `"symbol"` key in the emitted plan. The leading newline and
/// four-space indent are load-bearing — individual instructions carry a
/// `"symbol"` field too, inline on their own line (see the sibling test's note).
const FUNCTION_SYMBOL_KEY: &str = "\n    \"symbol\": \"";

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

/// How many times `body` branches to the imported package's `close` thunk.
///
/// The symbol carries the package's content-addressed identity prefix
/// (`_mfb_linker_<id>_2Esqlite3_2EsqliteLink_close`), which changes whenever
/// the `.mfp` is rebuilt, so match the stable tail rather than pinning a hash.
fn close_thunk_calls(body: &str) -> usize {
    const CALL: &str = "\"op\": \"bl\", \"target\": \"";
    body.match_indices(CALL)
        .filter(|(index, _)| {
            let rest = &body[index + CALL.len()..];
            let Some(end) = rest.find('"') else {
                return false;
            };
            rest[..end].ends_with("sqliteLink_close")
        })
        .count()
}

/// bug-377 (1): the scope-exit drop must close the imported handle.
///
/// Before the fix this count was 0 — `resource_cleanup_symbol` looked the close
/// op up under a name the merged module does not use, found nothing, and
/// registered no cleanup. The program leaked one sqlite connection per call and
/// exited 0 with correct output, so only reading the code catches it.
#[test]
fn imported_resource_closes_on_scope_exit() {
    let built = build(
        "b377_drop",
        "IMPORT sqlite3\n\n\
         FUNC dropIt() AS Nothing\n\
        \x20 RES db AS Db = sqlite3::create(\":memory:\")\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 dropIt()\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(built.success, "build should succeed:\n{}", built.stdout);
    let plan = built.plan();
    let body = function_body(&plan, "_mfb_fn_dropIt");
    assert_eq!(
        close_thunk_calls(body),
        1,
        "dropIt should close the imported resource once at scope exit; \
         bug-377 registered no cleanup at all, so this was 0.\n{body}"
    );
    // The close alone is not the fix: the 80-byte record must also be
    // reclaimed, which is what distinguishes a registered cleanup from an
    // incidental explicit close.
    assert!(
        body.contains("resource_cleanup_reclaim"),
        "dropIt should reclaim the record after the close.\n{body}"
    );
}

/// bug-377 (2): an explicit close does NOT retire the drop-path close, so a
/// correctly-written program closes twice — and the second one must be a
/// defined no-op.
///
/// A `LINK` close thunk never reclaims the record, so the drop-path close is
/// load-bearing and must stay (memory: `link-thunk-never-reclaims-the-record`).
/// What makes the second call safe is the thunk setting `RESOURCE_CLOSED_BIT`
/// on the first. The importing project's thunk did not, so this shape was a
/// double free into `libsqlite3` the moment the drop-path close started firing.
#[test]
fn explicitly_closed_imported_resource_still_drops_and_is_guarded() {
    let built = build(
        "b377_explicit",
        "IMPORT sqlite3\n\n\
         FUNC dropIt() AS Nothing\n\
        \x20 RES db AS Db = sqlite3::create(\":memory:\")\n\
        \x20 sqlite3::close(db)\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n\
        \x20 dropIt()\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(built.success, "build should succeed:\n{}", built.stdout);
    let plan = built.plan();
    let body = function_body(&plan, "_mfb_fn_dropIt");
    // Three call sites, not two: the explicit close, plus one drop-path close on
    // each of the two exits the explicit close creates. `sqlite3::close` is
    // itself fallible (`SUCCESS_ON`), so the function leaves either on its
    // failure branch or by falling through, and §15 requires the cleanup on both
    // — the same per-exit-path accounting the bug-374 early-return test pins.
    assert_eq!(
        close_thunk_calls(body),
        3,
        "an explicit close does not retire the drop-path close: expected the \
         explicit call plus one scope-exit close per exit path.\n{body}"
    );
    // The guard that makes the second close safe lives in the thunk, not at the
    // call site: being a registered close op is what makes it set the closed
    // bit. `RESOURCE_CLOSED_BIT` is bit 0, so the thunk ORs the flag word with
    // an immediate 1 and stores it back — a sequence a non-close thunk has no
    // reason to contain.
    let thunk_symbol = plan
        .match_indices(FUNCTION_SYMBOL_KEY)
        .map(|(index, _)| {
            let rest = &plan[index + FUNCTION_SYMBOL_KEY.len()..];
            rest[..rest.find('"').expect("terminated symbol")].to_string()
        })
        .find(|symbol| symbol.ends_with("sqliteLink_close"))
        .expect("plan should contain the imported close thunk");
    let thunk = function_body(&plan, &thunk_symbol);
    assert!(
        thunk.contains("\"op\": \"orr\""),
        "the imported close thunk should set RESOURCE_CLOSED_BIT; without it the \
         drop-path close is a double free.\n{thunk}"
    );
}

/// bug-377 (3): `ir::verify` must recognize the imported type as a resource, so
/// the RES ownership axis applies to it exactly as to a built-in.
///
/// `LET`-binding a resource compiled clean before the fix: `close_op_for`
/// returned `None` for an imported type, so `is_resource_or_resource_union` was
/// false and the rule's precondition was never met.
#[test]
fn imported_resource_let_binding_is_rejected() {
    let built = build(
        "b377_let",
        "IMPORT sqlite3\n\n\
         FUNC main() AS Integer\n\
        \x20 LET db = sqlite3::create(\":memory:\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        !built.success,
        "LET-binding an imported resource should be rejected:\n{}",
        built.stdout
    );
    assert!(
        built.stdout.contains("TYPE_RESOURCE_REQUIRES_RES"),
        "expected 2-203-0082 naming the RES axis, got:\n{}",
        built.stdout
    );
}

/// bug-377 (3), the second axis: move tracking must also apply, so a double
/// close of an imported handle is rejected at compile time rather than
/// double-freeing at runtime.
#[test]
fn imported_resource_double_close_is_rejected() {
    let built = build(
        "b377_double",
        "IMPORT sqlite3\n\n\
         FUNC main() AS Integer\n\
        \x20 RES db AS Db = sqlite3::create(\":memory:\")\n\
        \x20 sqlite3::close(db)\n\
        \x20 sqlite3::close(db)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        !built.success,
        "a double close of an imported resource should be rejected:\n{}",
        built.stdout
    );
    assert!(
        built.stdout.contains("TYPE_USE_AFTER_MOVE"),
        "expected the use-after-move diagnostic, got:\n{}",
        built.stdout
    );
}
