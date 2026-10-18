//! bug-886: `os::executablePath` / `os::appResourcePath` are the absolute,
//! symlink-resolved path with no `.`/`..` components (`mfb spec stdlib os`), however
//! the program was launched. macOS used to return `_NSGetExecutablePath`'s bytes
//! as-is: the symlink's own path, or `<cwd>/../x/prog`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::os::unix::fs::symlink;
use std::process::Command;

use common::{build_project, temp_project};

const SOURCE: &str = "IMPORT io\nIMPORT os\n\nFUNC main AS Integer\n  io::print(\"exe=\" & os::executablePath())\n  io::print(\"res=\" & os::appResourcePath(\"data.txt\"))\n  RETURN 0\nEND FUNC\n";

fn run(command: &mut Command) -> (String, String) {
    let out = command.output().expect("run program");
    assert!(out.status.success(), "program failed: {:?}", out.status);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .unwrap_or_else(|| panic!("no {key} line in {text}"))
            .to_string()
    };
    (field("exe="), field("res="))
}

#[test]
fn executable_path_is_resolved_for_symlinked_and_dotdot_launches() {
    let project = temp_project("exe_path_resolved", SOURCE);
    let exe = std::fs::canonicalize(build_project(&project)).unwrap();
    let real_dir = exe.parent().unwrap().to_path_buf();
    let want_exe = exe.to_string_lossy().into_owned();
    let want_res = real_dir.join("data.txt").to_string_lossy().into_owned();

    // Direct launch is the baseline.
    let (e, r) = run(&mut Command::new(&exe));
    assert_eq!(
        (e.as_str(), r.as_str()),
        (want_exe.as_str(), want_res.as_str())
    );

    // Launched through a symlink in another directory.
    let bin = project.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let link = bin.join("tool");
    symlink(&exe, &link).unwrap();
    let (e, r) = run(&mut Command::new(&link));
    assert_eq!(e, want_exe, "symlinked launch must resolve to the image");
    assert_eq!(
        r, want_res,
        "appResourcePath must follow the resolved image"
    );

    // Launched by a relative path full of `.` and `..`.
    let rel = format!(
        "../{}/./{}",
        real_dir.file_name().unwrap().to_string_lossy(),
        exe.file_name().unwrap().to_string_lossy()
    );
    let (e, r) = run(Command::new(&rel).current_dir(&real_dir));
    assert_eq!(e, want_exe, "`..`/`.` launch must be canonical");
    assert_eq!(r, want_res);
}
