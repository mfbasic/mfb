//! A parameter default resolves its names where the function is DECLARED and is
//! evaluated on every call that omits the argument (plan-136-A).
//!
//! bug-614: `ir::lower::lower_local_call_arguments` filled an omitted argument by
//! lowering the callee's default with the CALLER's locals, so a caller local (or
//! local lambda) named like the default's global or function won. The build
//! succeeded and the program printed the caller's value. A `LINK` function's
//! omitted defaulted argument was never passed at all.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The C library entry a `LINK "c"` block needs, per host OS.
const LIBC_LIBRARIES: &str = "\"libraries\":{\"c\":[\
     {\"os\":\"macos\",\"type\":\"system\",\"source\":\"libSystem.dylib\"},\
     {\"os\":\"linux\",\"type\":\"system\",\"source\":\"libc.so.6\"}]},";

/// Write an executable whose `src/main.mfb` is `main` (with `manifest_extra`
/// spliced into its manifest), build it, and return the build's success and
/// combined output together with the project root.
fn build_program(
    test: &str,
    manifest_extra: &str,
    main: &str,
) -> (Option<PathBuf>, String, PathBuf) {
    let root = std::env::temp_dir().join(format!("mfb_{test}_{}", common::unique_nonce()));
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).expect("create app dir");
    fs::write(
        app.join("project.json"),
        format!(
            "{{\"name\":\"defaultapp\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
             \"description\":\"parameter defaults\",{manifest_extra}\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write app manifest");
    fs::write(app.join("src/main.mfb"), main).expect("write app source");
    let (executable, output) = build(&app);
    (executable, output, root)
}

/// Build and run `main`, asserting both succeed, and return the program output.
fn run_program(test: &str, manifest_extra: &str, main: &str) -> String {
    let (executable, build_output, root) = build_program(test, manifest_extra, main);
    let executable = executable.unwrap_or_else(|| panic!("build failed:\n{build_output}"));
    let output = Command::new(&executable).output().expect("run the app");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "the app exited {}:\n{combined}",
        common::exit_description(&output.status)
    );
    let _ = fs::remove_dir_all(&root);
    combined
}

fn build(project: &Path) -> (Option<PathBuf>, String) {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        return (None, combined);
    }
    let path = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names an executable");
    (Some(PathBuf::from(path)), combined)
}

fn lines(output: &str) -> Vec<&str> {
    output.lines().collect()
}

/// P11: a default that CALLS a function. A caller's local lambda of the same
/// name must not be what the default calls. plan-136-C does not ban this local
/// (functions are not top-level bindings), so this case pins bug-614's property
/// for good.
#[test]
fn a_default_calling_a_function_ignores_a_callers_same_named_local() {
    let output = run_program(
        "default_call_scope",
        "",
        "IMPORT io\n\
         \n\
         FUNC helper() AS Integer\n\
        \x20 RETURN 5\n\
         END FUNC\n\
         \n\
         FUNC f(x AS Integer = helper()) AS Integer\n\
        \x20 RETURN x\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET helper = LAMBDA() -> 99\n\
        \x20 io::print(toString(f()))\n\
        \x20 io::print(toString(helper()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["5", "99"],
        "the default must call the declaration's `helper`, not the caller's local:\n{output}"
    );
}

/// P1: bug-614's own program — a default reading a global.
#[test]
fn a_default_reading_a_global_ignores_a_callers_same_named_local() {
    let output = run_program(
        "default_global_scope",
        "",
        "IMPORT io\n\
         \n\
         LET limit AS Integer = 5\n\
         \n\
         FUNC f(x AS Integer = limit) AS Integer\n\
        \x20 RETURN x\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET limit AS Integer = 99\n\
        \x20 io::print(toString(f()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["5"],
        "the default must read the global `limit`:\n{output}"
    );
}

/// A named-argument call leaves an earlier defaulted parameter to its default.
#[test]
fn a_named_call_fills_an_omitted_default_in_declaration_scope() {
    let output = run_program(
        "default_named_scope",
        "",
        "IMPORT io\n\
         \n\
         FUNC helper() AS Integer\n\
        \x20 RETURN 5\n\
         END FUNC\n\
         \n\
         FUNC f(x AS Integer = helper(), y AS Integer = 0) AS Integer\n\
        \x20 RETURN x * 10 + y\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET helper = LAMBDA() -> 99\n\
        \x20 io::print(toString(f(y := 1)))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["51"],
        "the named call must fill `x` from the declaration's `helper`:\n{output}"
    );
}

/// The P11 shape on a `SUB`.
#[test]
fn a_sub_default_ignores_a_callers_same_named_local() {
    let output = run_program(
        "default_sub_scope",
        "",
        "IMPORT io\n\
         \n\
         FUNC helper() AS Integer\n\
        \x20 RETURN 5\n\
         END FUNC\n\
         \n\
         SUB show(x AS Integer = helper())\n\
        \x20 io::print(toString(x))\n\
         END SUB\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET helper = LAMBDA() -> 99\n\
        \x20 show()\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["5"],
        "the SUB's default must call the declaration's `helper`:\n{output}"
    );
}

/// Each instantiation of a generic function fills the default from the
/// declaration scope.
#[test]
fn a_generic_function_default_is_filled_per_instantiation() {
    let output = run_program(
        "default_generic_scope",
        "",
        "IMPORT io\n\
         \n\
         FUNC helper() AS Integer\n\
        \x20 RETURN 5\n\
         END FUNC\n\
         \n\
         FUNC firstPlus OF T(items AS List OF T, x AS Integer = helper()) AS Integer\n\
        \x20 RETURN len(items) - len(items) + x\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET helper = LAMBDA() -> 99\n\
        \x20 io::print(toString(firstPlus([1, 2])))\n\
        \x20 io::print(toString(firstPlus([\"a\"])))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["5", "5"],
        "every instantiation must fill the default from the declaration:\n{output}"
    );
}

/// P10: an omitted defaulted argument of a `LINK` function is passed.
#[test]
fn a_link_function_fills_an_omitted_default() {
    let output = run_program(
        "default_link",
        LIBC_LIBRARIES,
        "IMPORT io\n\
         \n\
         LINK \"c\" AS libc\n\
        \x20 FUNC absval(n AS Integer = -5) AS Integer\n\
        \x20   SYMBOL \"abs\"\n\
        \x20   ABI (n CInt32) AS r CInt32\n\
        \x20   RETURN r\n\
        \x20 END FUNC\n\
         END LINK\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(libc::absval(-3)))\n\
        \x20 io::print(toString(libc::absval()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["3", "5"],
        "an omitted LINK default must be passed:\n{output}"
    );
}

/// P3 (pin): a default reading a `MUT` global sees its value at each call.
#[test]
fn a_default_reading_a_mut_global_sees_its_value_on_each_call() {
    let output = run_program(
        "default_mut_global",
        "",
        "IMPORT io\n\
         \n\
         MUT current AS Integer = 1\n\
         \n\
         FUNC f(x AS Integer = current) AS Integer\n\
        \x20 RETURN x\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(f()))\n\
        \x20 current = 2\n\
        \x20 io::print(toString(f()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["1", "2"], "{output}");
}

/// P4 (pin): a default calling a function runs on every call that omits it.
#[test]
fn a_default_calling_a_function_runs_on_every_call() {
    let output = run_program(
        "default_per_call",
        "",
        "IMPORT io\n\
         \n\
         MUT counter AS Integer = 0\n\
         \n\
         FUNC next1() AS Integer\n\
        \x20 counter = counter + 1\n\
        \x20 RETURN counter\n\
         END FUNC\n\
         \n\
         FUNC f(x AS Integer = next1()) AS Integer\n\
        \x20 RETURN x\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(f()) & \" \" & toString(f()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["1 2"], "{output}");
}

/// Pin: a literal default is passed as before.
#[test]
fn a_literal_default_is_unchanged() {
    let output = run_program(
        "default_literal",
        "",
        "IMPORT io\n\
         \n\
         SUB greet(name AS String, greeting AS String = \"Hello\")\n\
        \x20 io::print(greeting & \" \" & name)\n\
         END SUB\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 greet(\"Ada\")\n\
        \x20 greet(\"Bob\", \"Hi\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["Hello Ada", "Hi Bob"], "{output}");
}
