//! Regression test for bug-632: two types with the same bare name — a consumer's
//! own `TYPE A` and a package's exported `TYPE A`, or two packages' `TYPE A` —
//! collapsed into one when package IR was merged into the consumer.
//! `prefix_package_symbols` identity-prefixed a package's functions and globals
//! but left its types unqualified, and `merge_package` de-duplicated types by
//! bare name, so the second `A` was discarded and its code was verified (and
//! would have been laid out) against the first `A`'s fields.
//!
//! Every case builds real packages and a consumer from source.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug632_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug632_home"));
    command
}

fn write_project(root: &Path, name: &str, kind: &str, deps: &[&str], entry: bool, source: &str) {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("src")).expect("src dir");
    fs::create_dir_all(dir.join("packages")).expect("packages dir");
    let role = if entry { "main" } else { "package" };
    let packages = deps
        .iter()
        .map(|dep| {
            // bug-628: `name<requirer,…` also records the declared packages that
            // import `name` in its `requiredBy`.
            let (dep, required_by) = dep.split_once('<').unwrap_or((dep, ""));
            let required_by: Vec<String> = required_by
                .split(',')
                .filter(|requirer| !requirer.is_empty())
                .map(|requirer| format!("\"{requirer}\""))
                .collect();
            format!(
                "{{\"name\":\"{dep}\",\"version\":\"=0.1.0\",\"source\":\"file:packages/{dep}.mfp\",\
                 \"direct\":true,\"requiredBy\":[{}]}}",
                required_by.join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let entry_field = if entry {
        "\"entry\":\"main\",\"targets\":[\"native\"],"
    } else {
        ""
    };
    let manifest = format!(
        "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"{kind}\",\
         \"description\":\"bug-632 fixture\",\
         \"sources\":[{{\"root\":\"src\",\"role\":\"{role}\",\"include\":[\"**/*.mfb\"]}}],\
         {entry_field}\"packages\":[{packages}]}}\n"
    );
    fs::write(dir.join("project.json"), manifest).expect("write manifest");
    let src_name = if entry { "main.mfb" } else { "lib.mfb" };
    fs::write(dir.join("src").join(src_name), source).expect("write source");
}

fn build(root: &Path, name: &str) -> String {
    let out = mfb()
        .arg("build")
        .arg(root.join(name))
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "expected `{name}` to build, but it failed:\n{combined}"
    );
    combined
}

/// Build `name` expecting failure; return the combined output.
fn build_fails(root: &Path, name: &str) -> String {
    let out = mfb()
        .arg("build")
        .arg(root.join(name))
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "expected `{name}` to fail to build, but it succeeded:\n{combined}"
    );
    combined
}

fn install(root: &Path, dep: &str, into: &str) {
    fs::copy(
        root.join(dep).join(format!("{dep}.mfp")),
        root.join(into).join("packages").join(format!("{dep}.mfp")),
    )
    .unwrap_or_else(|e| panic!("install {dep}.mfp into {into}: {e}"));
}

/// A package: its name, the packages it depends on, and its `lib.mfb` source.
type Package<'a> = (&'a str, &'a [&'a str], &'a str);

/// Build `packages` in order (installing each one's dependencies first), then a
/// consumer that depends on `app_deps` with `app_src`, run it, and return its
/// stdout lines.
fn run_app(case: &str, packages: &[Package<'_>], app_deps: &[&str], app_src: &str) -> Vec<String> {
    let root = unique_root(case);
    for (name, deps, src) in packages {
        write_project(&root, name, "package", deps, false, src);
        for dep in *deps {
            install(&root, dep, name);
        }
        build(&root, name);
    }
    let declared: Vec<String> = app_deps
        .iter()
        .map(|dep| {
            let requirers: Vec<&str> = app_deps
                .iter()
                .copied()
                .filter(|other| {
                    packages
                        .iter()
                        .any(|(name, deps, _)| name == other && deps.contains(dep))
                })
                .collect();
            format!("{dep}<{}", requirers.join(","))
        })
        .collect();
    let declared: Vec<&str> = declared.iter().map(String::as_str).collect();
    write_project(&root, "app", "executable", &declared, true, app_src);
    for dep in app_deps {
        install(&root, dep, "app");
    }
    let out = build(&root, "app");
    let exe = out
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("app build reported no executable path")
        .trim()
        .to_string();
    let run = Command::new(&exe).output().expect("run app");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success(),
        "case `{case}`: app crashed:\n{stdout}"
    );
    stdout.lines().map(str::to_string).collect()
}

// `ov` exports a record `A` whose single field is an Integer, and builds one.
const OV_SRC: &str = "EXPORT TYPE A\n  x AS Integer\nEND TYPE\n\
EXPORT FUNC make() AS A\n  RETURN A[1]\nEND FUNC\n\
EXPORT FUNC one(a AS A) AS String\n  RETURN \"one\"\nEND FUNC\n";

// The consumer's own `A` has a different layout (one String field).
const LOCAL_A: &str = "TYPE A\n  z AS String\nEND TYPE\n";

/// bug-632 Case 1: a consumer type and a package type share the bare name `A`.
#[test]
fn consumer_type_and_package_type_share_a_name() {
    let app = format!(
        "IMPORT ov\nIMPORT io\n{LOCAL_A}\
         FUNC main() AS Integer\n  LET mine AS A = A[\"z\"]\n  io::print(mine.z)\n\
         \x20 io::print(ov::one(ov::make()))\n  RETURN 0\nEND FUNC\n"
    );
    let lines = run_app("case1", &[("ov", &[], OV_SRC)], &["ov"], &app);
    assert_eq!(lines, ["z", "one"]);
}

/// bug-632 Case 2: two packages each export a different `TYPE A`; the consumer
/// declares no type at all.
#[test]
fn two_packages_export_types_with_the_same_name() {
    let pa = "EXPORT TYPE A\n  x AS Integer\nEND TYPE\n\
              EXPORT FUNC describe() AS String\n  LET a AS A = A[7]\n  RETURN \"pa:\" & toString(a.x)\nEND FUNC\n";
    let pb = "EXPORT TYPE A\n  s AS String\nEND TYPE\n\
              EXPORT FUNC describe() AS String\n  LET a AS A = A[\"hi\"]\n  RETURN \"pb:\" & a.s\nEND FUNC\n";
    let app = "IMPORT pa\nIMPORT pb\nIMPORT io\n\
               FUNC main() AS Integer\n  io::print(pa::describe())\n  io::print(pb::describe())\n  RETURN 0\nEND FUNC\n";
    let lines = run_app(
        "case2",
        &[("pa", &[], pa), ("pb", &[], pb)],
        &["pa", "pb"],
        app,
    );
    assert_eq!(lines, ["pa:7", "pb:hi"]);
}

/// The consumer names the imported `ov::A` explicitly and reads its field while
/// declaring its own `A` — the consumer-side reference must resolve to `ov`'s
/// layout, not the local one.
#[test]
fn consumer_reads_an_imported_types_field_beside_a_local_type_of_the_same_name() {
    let app = format!(
        "IMPORT ov\nIMPORT io\n{LOCAL_A}\
         FUNC main() AS Integer\n  LET mine AS A = A[\"z\"]\n  io::print(mine.z)\n\
         \x20 LET theirs AS ov::A = ov::make()\n  io::print(toString(theirs.x))\n  RETURN 0\nEND FUNC\n"
    );
    let lines = run_app("qualified_ref", &[("ov", &[], OV_SRC)], &["ov"], &app);
    assert_eq!(lines, ["z", "1"]);
}

/// bug-631's collision row: an overloaded imported `show` called with a field of
/// an imported record, while the consumer declares its own `A`.
#[test]
fn overloaded_imported_call_with_imported_field_beside_a_local_type_of_the_same_name() {
    let ov = "EXPORT TYPE A\n  x AS Integer\nEND TYPE\n\
              EXPORT TYPE B\n  y AS Integer\nEND TYPE\n\
              EXPORT TYPE Holder\n  first AS A\nEND TYPE\n\
              EXPORT FUNC show(a AS A) AS String\n  RETURN \"A\"\nEND FUNC\n\
              EXPORT FUNC show(b AS B) AS String\n  RETURN \"B\"\nEND FUNC\n\
              EXPORT FUNC holder() AS Holder\n  RETURN Holder[A[3]]\nEND FUNC\n";
    let app = format!(
        "IMPORT ov\nIMPORT io\n{LOCAL_A}\
         FUNC main() AS Integer\n  LET h AS ov::Holder = ov::holder()\n  LET mine AS A = A[\"z\"]\n\
         \x20 io::print(mine.z)\n  io::print(ov::show(h.first))\n  RETURN 0\nEND FUNC\n"
    );
    let lines = run_app("bug631_row", &[("ov", &[], ov)], &["ov"], &app);
    assert_eq!(lines, ["z", "A"]);
}

/// Spec §13: "A bare imported type is refused with `SYMBOL_UNKNOWN_TYPE`." A user
/// package's type is imported, so `AS A` outside `ov` names nothing.
#[test]
fn a_bare_imported_user_package_type_is_refused() {
    let root = unique_root("bare_refused");
    write_project(&root, "ov", "package", &[], false, OV_SRC);
    build(&root, "ov");
    let app = "IMPORT ov\nIMPORT io\n\
               FUNC main() AS Integer\n  LET a AS A = ov::make()\n  io::print(ov::one(a))\n  RETURN 0\nEND FUNC\n";
    write_project(&root, "app", "executable", &["ov"], true, app);
    install(&root, "ov", "app");
    let out = build_fails(&root, "app");
    assert!(
        out.contains("SYMBOL_UNKNOWN_TYPE"),
        "a bare imported type must be refused with SYMBOL_UNKNOWN_TYPE:\n{out}"
    );
}

/// Spec §13: "The prefix is the import **binding**, not the package name." An
/// aliased import names the same type, and still does not collide with a local
/// type of the same bare name.
#[test]
fn an_aliased_import_names_the_same_type() {
    let app = format!(
        "IMPORT ov AS o\nIMPORT io\n{LOCAL_A}\
         FUNC main() AS Integer\n  LET mine AS A = A[\"z\"]\n  io::print(mine.z)\n\
         \x20 LET theirs AS o::A = o::make()\n  io::print(toString(theirs.x))\n\
         \x20 io::print(o::one(theirs))\n  RETURN 0\nEND FUNC\n"
    );
    let lines = run_app("alias", &[("ov", &[], OV_SRC)], &["ov"], &app);
    assert_eq!(lines, ["z", "1", "one"]);
}

/// A union, its variants and an enum follow the same rule, beside consumer types
/// that reuse every one of those bare names.
#[test]
fn imported_union_variants_and_enum_members_keep_their_package() {
    let shapes = "EXPORT TYPE Circle\n  r AS Integer\nEND TYPE\n\
                  EXPORT TYPE Square\n  s AS Integer\nEND TYPE\n\
                  EXPORT UNION Shape\n  Circle\n  Square\nEND UNION\n\
                  EXPORT ENUM Kind\n  Round\n  Boxy\nEND ENUM\n\
                  EXPORT FUNC unit() AS Shape\n  LET s AS Shape = Circle[2]\n  RETURN s\nEND FUNC\n\
                  EXPORT FUNC kindOf(s AS Shape) AS Kind\n  MATCH s\n    CASE Circle(c)\n      RETURN Kind.Round\n    CASE Square(q)\n      RETURN Kind.Boxy\n  END MATCH\nEND FUNC\n";
    let app = "IMPORT shapes\nIMPORT io\n\
               TYPE Circle\n  label AS String\nEND TYPE\n\
               ENUM Kind\n  Local\nEND ENUM\n\
               FUNC main() AS Integer\n  LET mine AS Circle = Circle[\"mine\"]\n  io::print(mine.label)\n\
               \x20 LET k AS Kind = Kind.Local\n\
               \x20 LET s AS shapes::Shape = shapes::unit()\n\
               \x20 MATCH s\n    CASE shapes::Circle(c)\n      io::print(\"r=\" & toString(c.r))\n    CASE shapes::Square(q)\n      io::print(\"s=\" & toString(q.s))\n  END MATCH\n\
               \x20 IF shapes::kindOf(s) = shapes::Kind.Round THEN\n    io::print(\"round\")\n  END IF\n\
               \x20 RETURN 0\nEND FUNC\n";
    let lines = run_app("union_enum", &[("shapes", &[], shapes)], &["shapes"], app);
    assert_eq!(lines, ["mine", "r=2", "round"]);
}

/// Distinct identities must also be distinct TYPES to the checker: a value of
/// `pa`'s `A` is not a `pb` `A`, even when the two records have the same fields.
/// Guard: with both names qualified this is refused by the shape pass's
/// declaration-identity check (bug-41), and it must stay refused.
#[test]
fn one_packages_type_is_refused_where_another_packages_same_named_type_is_expected() {
    let root = unique_root("cross_package_value");
    let pa = "EXPORT TYPE A\n  x AS Integer\nEND TYPE\n\
              EXPORT FUNC make() AS A\n  RETURN A[1]\nEND FUNC\n";
    let pb = "EXPORT TYPE A\n  x AS Integer\nEND TYPE\n\
              EXPORT FUNC use(a AS A) AS String\n  RETURN \"pb:\" & toString(a.x)\nEND FUNC\n";
    for (name, src) in [("pa", pa), ("pb", pb)] {
        write_project(&root, name, "package", &[], false, src);
        build(&root, name);
    }
    let app = "IMPORT pa\nIMPORT pb\nIMPORT io\n\
               FUNC main() AS Integer\n  io::print(pb::use(pa::make()))\n  RETURN 0\nEND FUNC\n";
    write_project(&root, "app", "executable", &["pa", "pb"], true, app);
    install(&root, "pa", "app");
    install(&root, "pb", "app");
    let out = build_fails(&root, "app");
    assert!(
        out.contains("TYPE_CALL_ARGUMENT_MISMATCH"),
        "pa::A passed where pb::A is expected must be a call argument mismatch:\n{out}"
    );
}

/// The same between a consumer's own type and a package's: an imported `ov::A`
/// value does not bind to a local `A`, however alike their fields are.
#[test]
fn an_imported_value_is_refused_by_a_local_type_of_the_same_name() {
    let root = unique_root("local_binding_value");
    write_project(&root, "ov", "package", &[], false, OV_SRC);
    build(&root, "ov");
    let app = "IMPORT ov\nIMPORT io\n\
               TYPE A\n  x AS Integer\nEND TYPE\n\
               FUNC main() AS Integer\n  LET mine AS A = ov::make()\n  io::print(toString(mine.x))\n  RETURN 0\nEND FUNC\n";
    write_project(&root, "app", "executable", &["ov"], true, app);
    install(&root, "ov", "app");
    let out = build_fails(&root, "app");
    assert!(
        out.contains("TYPE_BINDING_MISMATCH"),
        "an ov::A value bound to a local A must be a binding mismatch:\n{out}"
    );
}

/// Guard: a diamond — `left` and `right` both depend on `base`, which exports a
/// type — must still merge `base`'s type into ONE copy that both use.
#[test]
fn diamond_import_shares_one_copy_of_a_type() {
    let base = "EXPORT TYPE P\n  n AS Integer\nEND TYPE\n\
                EXPORT FUNC mk(n AS Integer) AS P\n  RETURN P[n]\nEND FUNC\n\
                EXPORT FUNC get(p AS P) AS Integer\n  RETURN p.n\nEND FUNC\n";
    let left = "IMPORT base\n\
                EXPORT FUNC value() AS Integer\n  RETURN base::get(base::mk(10))\nEND FUNC\n";
    let right = "IMPORT base\n\
                 EXPORT FUNC value() AS Integer\n  RETURN base::get(base::mk(20))\nEND FUNC\n";
    let app = "IMPORT left\nIMPORT right\nIMPORT base\nIMPORT io\n\
               FUNC main() AS Integer\n  io::print(toString(left::value()))\n\
               \x20 io::print(toString(right::value()))\n\
               \x20 io::print(toString(base::get(base::mk(30))))\n  RETURN 0\nEND FUNC\n";
    let lines = run_app(
        "diamond",
        &[
            ("base", &[], base),
            ("left", &["base"], left),
            ("right", &["base"], right),
        ],
        &["left", "right", "base"],
        app,
    );
    assert_eq!(lines, ["10", "20", "30"]);
}
