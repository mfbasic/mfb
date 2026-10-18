//! bug-914: import bindings are per file, so one alias may name different
//! packages in different files. An imported overload set is resolved against the
//! package the CALLING file's own binding names — never a project-wide
//! `binding -> package` map, where the file that sorted last won and the other
//! file's overloaded call was left unresolved (`TYPE_UNKNOWN_VALUE`).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn package(root: &Path, name: &str, source: &str) {
    let dir = root.join("packages").join(name);
    write(
        &dir.join("project.json"),
        &format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\"description\":\"bug-914 fixture package\",\"sources\":[{{\"root\":\"src\",\"role\":\"lib\",\"include\":[\"**/*.mfb\"]}}],\"targets\":[\"native\"]}}\n"
        ),
    );
    write(&dir.join("src/lib.mfb"), source);
}

/// Build and run a project whose `lib` alias names `pA` in one file and `pH` in
/// the other; `alias_file` is the name of the file that binds `pH`, so the two
/// sort orders are both exercised.
fn run_with(alias_file: &str) -> String {
    let root = std::env::temp_dir().join(format!(
        "mfb_alias_per_file_{alias_file}_{}",
        common::unique_nonce()
    ));
    let entry = |name: &str| {
        format!(
            "{{ \"name\": \"{name}\", \"version\": \"=0.1.0\", \"source\": \"file:packages/{name}\", \"direct\": true, \"requiredBy\": [] }}"
        )
    };
    write(
        &root.join("project.json"),
        &format!(
            "{{\"name\":\"alias_per_file\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"packages\":[{}, {}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n",
            entry("pA"),
            entry("pH")
        ),
    );
    package(
        &root,
        "pA",
        "EXPORT FUNC show(n AS Integer) AS String
  RETURN \"int:\" & toString(n)
END FUNC
EXPORT FUNC show(s AS String) AS String
  RETURN \"str:\" & s
END FUNC
",
    );
    package(
        &root,
        "pH",
        "EXPORT FUNC hello() AS String
  RETURN \"pH\"
END FUNC
EXPORT FUNC show(n AS Integer, m AS Integer) AS String
  RETURN \"pH-two\"
END FUNC
EXPORT FUNC show(b AS Boolean) AS String
  RETURN \"pH-bool\"
END FUNC
",
    );
    write(
        &root.join("src/a.mfb"),
        "IMPORT pA AS lib
LET fromGlobal AS String = lib::show(\"g\")
FUNC fromA() AS String
  RETURN lib::show(1) & \" \" & lib::show(\"s\") & \" \" & fromGlobal
END FUNC
",
    );
    write(
        &root.join(format!("src/{alias_file}.mfb")),
        "IMPORT pH AS lib
FUNC fromH() AS String
  RETURN lib::hello() & \" \" & lib::show(1, 2) & \" \" & lib::show(TRUE)
END FUNC
",
    );
    write(
        &root.join("src/main.mfb"),
        "IMPORT io
FUNC main AS Integer
  io::print(fromA() & \" | \" & fromH())
  RETURN 0
END FUNC
",
    );
    let exe = common::build_project(&root);
    let out = Command::new(&exe).output().expect("run the program");
    let _ = fs::remove_dir_all(&root);
    assert!(out.status.success(), "program failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn an_alias_reused_across_files_resolves_overloads_per_file() {
    // `z` sorts after `a`, `0z` before it: the result must not depend on order.
    for alias_file in ["z", "0z"] {
        assert_eq!(
            run_with(alias_file),
            "int:1 str:s str:g | pH pH-two pH-bool\n",
            "file order `{alias_file}`"
        );
    }
}
