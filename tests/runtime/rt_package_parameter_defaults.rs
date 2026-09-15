//! An importer's call that omits a defaulted argument of an imported package
//! function passes the default, evaluated on that call in the PACKAGE's scope —
//! exactly what the same code does in an executable (plan-136-B).
//!
//! Before: `ir::lower::lower_facts` built an imported function's call parameters
//! from `ExternalFunctionParam { has_default: bool }`, a flag with no value, so an
//! omitted argument was simply not passed (a literal default read garbage or
//! crashed), and a computed default did not build at all (`only constant IR
//! values can be stored in CONST_POOL`).

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A source package: its name, its `lib.mfb`, the packages it imports, and any
/// extra manifest members (a `libraries` table) spliced in verbatim.
struct Package<'a> {
    name: &'a str,
    source: &'a str,
    depends_on: &'a [&'a str],
    manifest_extra: &'a str,
}

/// The C library entry a `LINK "c"` block needs, per host OS.
const LIBC_LIBRARIES: &str = "\"libraries\":{\"c\":[\
     {\"os\":\"macos\",\"type\":\"system\",\"source\":\"libSystem.dylib\"},\
     {\"os\":\"linux\",\"type\":\"system\",\"source\":\"libc.so.6\"}]},";

/// The P6 package: one literal Integer default and one literal String default.
const DEFLIB: Package<'static> = Package {
    name: "deflib",
    source: "EXPORT FUNC f(x AS Integer = 5) AS Integer\n\
            \x20 RETURN x\n\
             END FUNC\n\
             \n\
             EXPORT FUNC g(a AS Integer, s AS String = \"dflt\") AS String\n\
            \x20 RETURN toString(a) & s\n\
             END FUNC\n",
    depends_on: &[],
    manifest_extra: "",
};

fn package_entries(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| {
            format!("{{\"name\":\"{name}\",\"version\":\"=0.1.0\",\"source\":\"file:../{name}\"}}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Write every package plus an executable importing `app_packages`, build it, and
/// return the build's executable (or `None`), its combined output, and the root.
fn build_app(
    test: &str,
    packages: &[Package<'_>],
    app_packages: &[&str],
    main: &str,
) -> (Option<PathBuf>, String, PathBuf) {
    let root = std::env::temp_dir().join(format!("mfb_{test}_{}", common::unique_nonce()));
    for package in packages {
        let dir = root.join(package.name);
        fs::create_dir_all(dir.join("src")).expect("create package dir");
        fs::write(
            dir.join("project.json"),
            format!(
                "{{\"name\":\"{}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
                 \"description\":\"a package\",{}\
                 \"sources\":[{{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}}],\
                 \"packages\":[{}]}}\n",
                package.name,
                package.manifest_extra,
                package_entries(package.depends_on)
            ),
        )
        .expect("write package manifest");
        fs::write(dir.join("src/lib.mfb"), package.source).expect("write package source");
    }
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).expect("create app dir");
    fs::write(
        app.join("project.json"),
        format!(
            "{{\"name\":\"defaultsapp\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
             \"description\":\"an importer\",\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"packages\":[{}],\
             \"entry\":\"main\",\"targets\":[\"native\"]}}\n",
            package_entries(app_packages)
        ),
    )
    .expect("write app manifest");
    fs::write(app.join("src/main.mfb"), main).expect("write app source");
    let (executable, output) = build(&app);
    (executable, output, root)
}

/// Build and run the app, asserting both succeed, and return the program output.
fn run_app(test: &str, packages: &[Package<'_>], app_packages: &[&str], main: &str) -> String {
    let (executable, build_output, root) = build_app(test, packages, app_packages, main);
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

/// P6: `deflib::f()` receives the literal `5`.
#[test]
fn an_omitted_literal_integer_package_default_is_passed() {
    let output = run_app(
        "pkgdefault_literal_int",
        &[DEFLIB],
        &["deflib"],
        "IMPORT deflib\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(deflib::f(7)))\n\
        \x20 io::print(toString(deflib::f()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["7", "5"], "{output}");
}

/// P6: `deflib::g(1)` receives the literal `"dflt"`.
#[test]
fn an_omitted_literal_string_package_default_is_passed() {
    let output = run_app(
        "pkgdefault_literal_string",
        &[DEFLIB],
        &["deflib"],
        "IMPORT deflib\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(deflib::g(1))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["1dflt"], "{output}");
}

/// A computed package default reads a package-PRIVATE `MUT` global on each call,
/// never the importer's same-named local.
#[test]
fn a_computed_package_default_reads_a_private_global_on_each_call() {
    let output = run_app(
        "pkgdefault_private_global",
        &[Package {
            name: "limits",
            source: "PRIVATE MUT Limit AS Integer = 5\n\
                     \n\
                     EXPORT SUB setLimit(n AS Integer)\n\
                    \x20 Limit = n\n\
                     END SUB\n\
                     \n\
                     EXPORT FUNC f(x AS Integer = Limit) AS Integer\n\
                    \x20 RETURN x\n\
                     END FUNC\n",
            depends_on: &[],
            manifest_extra: "",
        }],
        &["limits"],
        "IMPORT limits\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET Limit AS Integer = 99\n\
        \x20 io::print(toString(limits::f()))\n\
        \x20 limits::setLimit(6)\n\
        \x20 io::print(toString(limits::f()))\n\
        \x20 io::print(toString(Limit))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["5", "6", "99"], "{output}");
}

/// A named-argument call that skips a defaulted MIDDLE parameter fills it.
#[test]
fn a_named_package_call_fills_a_skipped_middle_default() {
    let output = run_app(
        "pkgdefault_named_middle",
        &[Package {
            name: "triple",
            source: "EXPORT FUNC f(a AS Integer, b AS Integer = 2, c AS Integer = 3) AS String\n\
                    \x20 RETURN toString(a) & \" \" & toString(b) & \" \" & toString(c)\n\
                     END FUNC\n",
            depends_on: &[],
            manifest_extra: "",
        }],
        &["triple"],
        "IMPORT triple\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(triple::f(1, c := 9))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["1 2 9"], "{output}");
}

/// Package B's default calls package A's export. The app lists both packages, as
/// `rt_top_level_initializer_globals`'s chain test does: a dependency an importer
/// does not list is not merged, whether B reaches A from a default or from an
/// ordinary function body (plan-136-B Corrections).
#[test]
fn a_package_default_calling_another_packages_export_is_filled() {
    let output = run_app(
        "pkgdefault_cross_package",
        &[
            Package {
                name: "basepkg",
                source: "EXPORT FUNC base() AS Integer\n\
                        \x20 RETURN 5\n\
                         END FUNC\n",
                depends_on: &[],
                manifest_extra: "",
            },
            Package {
                name: "userpkg",
                source: "IMPORT basepkg\n\
                         \n\
                         EXPORT FUNC f(x AS Integer = basepkg::base()) AS Integer\n\
                        \x20 RETURN x\n\
                         END FUNC\n",
                depends_on: &["basepkg"],
                manifest_extra: "",
            },
        ],
        &["userpkg", "basepkg"],
        "IMPORT userpkg\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(userpkg::f()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["5"], "{output}");
}

/// A package exposes a LINK function only through an exported MFB wrapper (owner
/// ruling, plan-136-B Corrections). The wrapper's default crosses the package
/// boundary; the LINK function's own default is filled inside the package.
#[test]
fn a_link_wrapper_package_fills_both_defaults() {
    let output = run_app(
        "pkgdefault_link",
        &[Package {
            name: "cmath",
            source: "LINK \"c\" AS libc\n\
                    \x20 FUNC absval(n AS Integer = -5) AS Integer\n\
                    \x20   SYMBOL \"abs\"\n\
                    \x20   ABI (n CInt32) AS r CInt32\n\
                    \x20   RETURN r\n\
                    \x20 END FUNC\n\
                     END LINK\n\
                     \n\
                     EXPORT FUNC absval(n AS Integer = -7) AS Integer\n\
                    \x20 RETURN libc::absval(n)\n\
                     END FUNC\n\
                     \n\
                     EXPORT FUNC absDefault() AS Integer\n\
                    \x20 RETURN libc::absval()\n\
                     END FUNC\n",
            depends_on: &[],
            manifest_extra: LIBC_LIBRARIES,
        }],
        &["cmath"],
        "IMPORT cmath\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(cmath::absval(-3)))\n\
        \x20 io::print(toString(cmath::absval()))\n\
        \x20 io::print(toString(cmath::absDefault()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["3", "7", "5"], "{output}");
}

/// Pin (plan-136-A's rule holds for a package): a package default naming a
/// parameter is a located error when the package is built.
#[test]
fn a_package_default_naming_a_parameter_is_refused_at_package_build() {
    let (executable, output, root) = build_app(
        "pkgdefault_names_parameter",
        &[Package {
            name: "badpkg",
            source: "EXPORT FUNC f(a AS Integer, b AS Integer = a) AS Integer\n\
                    \x20 RETURN a + b\n\
                     END FUNC\n",
            depends_on: &[],
            manifest_extra: "",
        }],
        &["badpkg"],
        "IMPORT badpkg\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(badpkg::f(1)))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        executable.is_none(),
        "a package default naming a parameter must not build:\n{output}"
    );
    assert!(
        output.contains("SYMBOL_DEFAULT_NAMES_PARAMETER") && output.contains("src/lib.mfb:1"),
        "the refusal must be the located plan-136-A rule:\n{output}"
    );
    let _ = fs::remove_dir_all(&root);
}
