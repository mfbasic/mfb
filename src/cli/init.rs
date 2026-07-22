use std::fs;
use std::path::Path;

use crate::json_string;

pub(crate) fn init_project(location: &Path) -> Result<(), String> {
    let src_dir = location.join("src");
    fs::create_dir_all(&src_dir).map_err(|err| {
        format!(
            "failed to create source directory '{}': {err}",
            src_dir.display()
        )
    })?;

    let project_path = location.join("project.json");
    let main_path = src_dir.join("main.mfb");

    write_new_file(&project_path, project_manifest(location) + "\n")?;
    write_new_file(&main_path, hello_world_source())?;

    println!("Created MFBASIC project at {}", location.display());
    Ok(())
}

pub(crate) fn init_package_project(location: &Path) -> Result<(), String> {
    let src_dir = location.join("src");
    fs::create_dir_all(&src_dir).map_err(|err| {
        format!(
            "failed to create source directory '{}': {err}",
            src_dir.display()
        )
    })?;

    let project_path = location.join("project.json");
    let lib_path = src_dir.join("lib.mfb");

    write_new_file(&project_path, package_project_manifest(location) + "\n")?;
    write_new_file(&lib_path, package_source())?;

    println!("Created MFBASIC package project at {}", location.display());
    Ok(())
}

/// Create `path` and write `contents`, refusing if anything is already there.
///
/// `create_new` is `O_EXCL`: it fails on an existing path without following a
/// final-component symlink, and leaves no window between the check and the write.
/// An `exists()` test followed by `fs::write` would both race and follow a
/// pre-planted (even dangling) symlink onto its target.
fn write_new_file(path: &Path, contents: String) -> Result<(), String> {
    use std::io::Write;

    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(format!("refusing to overwrite '{}'", path.display()));
        }
        Err(err) => return Err(format!("failed to write '{}': {err}", path.display())),
    };
    file.write_all(contents.as_bytes())
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))
}

pub(crate) fn project_manifest(location: &Path) -> String {
    let name = json_string(&project_name(location));

    format!(
        concat!(
            "{{\n",
            "  \"name\": {},\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"sources\": [\n",
            "    {{\n",
            "      \"root\": \"src\",\n",
            "      \"role\": \"main\",\n",
            "      \"include\": [\"**/*.mfb\"]\n",
            "    }}\n",
            "  ],\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"config\": {{\n",
            "    \"stdinLogCap\": {}\n",
            "  }}\n",
            "}}"
        ),
        name,
        crate::target::shared::code::STDIN_LOG_CAP_DEFAULT
    )
}

fn package_project_manifest(location: &Path) -> String {
    let name = json_string(&project_name(location));

    format!(
        concat!(
            "{{\n",
            "  \"name\": {},\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"package\",\n",
            // Required for `kind: "package"` since plan-61-F Phase 4
            // (2-200-0016 is an error, not a warning): a scaffold without it
            // does not build, so `init-pkg` must emit one. Deliberately a
            // placeholder the author is meant to replace -- it is shown on the
            // registry.
            "  \"description\": \"A reusable MFBASIC package.\",\n",
            "  \"sources\": [\n",
            "    {{\n",
            "      \"root\": \"src\",\n",
            "      \"role\": \"package\",\n",
            "      \"include\": [\"**/*.mfb\"]\n",
            "    }}\n",
            "  ],\n",
            "  \"config\": {{\n",
            "    \"stdinLogCap\": {}\n",
            "  }}\n",
            "}}"
        ),
        name,
        crate::target::shared::code::STDIN_LOG_CAP_DEFAULT
    )
}

fn project_name(location: &Path) -> String {
    location
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_project_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "mfb_project".to_string())
}

fn sanitize_project_name(name: &str) -> String {
    let mut sanitized = String::new();

    for (index, ch) in name.chars().enumerate() {
        let valid = ch.is_ascii_alphanumeric() || ch == '_';
        if valid && (index > 0 || ch.is_ascii_alphabetic() || ch == '_') {
            sanitized.push(ch);
        } else if index > 0 {
            sanitized.push('_');
        }
    }

    if sanitized.is_empty() {
        "mfb_project".to_string()
    } else {
        sanitized
    }
}

fn hello_world_source() -> String {
    "IMPORT io\n\nSUB main()\n  io::print(\"Hello World\")\nEND SUB\n".to_string()
}

fn package_source() -> String {
    "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinyjson::JsonValue;

    #[test]
    fn sanitize_project_name_replaces_invalid_characters() {
        assert_eq!(sanitize_project_name("my-app"), "my_app");
        assert_eq!(sanitize_project_name("hello world"), "hello_world");
        // A leading digit is dropped (first char must be a letter/underscore).
        assert_eq!(sanitize_project_name("1abc"), "abc");
        assert_eq!(sanitize_project_name("valid_name9"), "valid_name9");
        // Trailing invalid chars become underscores; a leading invalid char at
        // index 0 is dropped entirely.
        assert_eq!(sanitize_project_name("---"), "__");
        // Only-a-single-invalid-leading-char produces an empty string -> default.
        assert_eq!(sanitize_project_name("1"), "mfb_project");
        // Empty input falls back to the default.
        assert_eq!(sanitize_project_name(""), "mfb_project");
    }

    #[test]
    fn project_name_uses_directory_basename() {
        assert_eq!(project_name(Path::new("/tmp/cool-app")), "cool_app");
        // A path with no usable file name falls back.
        assert_eq!(project_name(Path::new("/")), "mfb_project");
    }

    #[test]
    fn project_manifest_is_valid_executable_json() {
        let manifest = project_manifest(Path::new("/tmp/demo"));
        let parsed: JsonValue = manifest.parse().expect("valid JSON");
        let object = parsed
            .get::<std::collections::HashMap<String, JsonValue>>()
            .expect("object");
        assert_eq!(
            object
                .get("kind")
                .and_then(|v| v.get::<String>())
                .map(String::as_str),
            Some("executable")
        );
        assert_eq!(
            object
                .get("name")
                .and_then(|v| v.get::<String>())
                .map(String::as_str),
            Some("demo")
        );
        // plan-15 D3: the scaffold seeds config.stdinLogCap at the runtime default,
        // and it round-trips through the manifest reader.
        assert_eq!(
            crate::manifest::stdin_log_cap(object),
            Some(crate::target::shared::code::STDIN_LOG_CAP_DEFAULT)
        );
    }

    #[test]
    fn package_project_manifest_is_valid_package_json() {
        let manifest = package_project_manifest(Path::new("/tmp/lib"));
        let parsed: JsonValue = manifest.parse().expect("valid JSON");
        let object = parsed
            .get::<std::collections::HashMap<String, JsonValue>>()
            .expect("object");
        assert_eq!(
            object
                .get("kind")
                .and_then(|v| v.get::<String>())
                .map(String::as_str),
            Some("package")
        );
        assert_eq!(
            crate::manifest::stdin_log_cap(object),
            Some(crate::target::shared::code::STDIN_LOG_CAP_DEFAULT)
        );
    }

    #[test]
    fn init_project_scaffolds_an_executable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let location = dir.path().join("app");
        std::fs::create_dir_all(&location).expect("location");
        init_project(&location).expect("init");
        assert!(location.join("project.json").is_file());
        assert!(location.join("src").join("main.mfb").is_file());
        let source = std::fs::read_to_string(location.join("src").join("main.mfb")).unwrap();
        assert!(source.contains("io::print"));
    }

    #[test]
    fn init_package_project_scaffolds_a_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        let location = dir.path().join("lib");
        std::fs::create_dir_all(&location).expect("location");
        init_package_project(&location).expect("init");
        assert!(location.join("project.json").is_file());
        assert!(location.join("src").join("lib.mfb").is_file());
        let source = std::fs::read_to_string(location.join("src").join("lib.mfb")).unwrap();
        assert!(source.contains("EXPORT FUNC answer"));
        // `description` is REQUIRED for `kind: "package"` (2-200-0016 became an
        // error in plan-61-F Phase 4), so a scaffold that omits it produces a
        // project that cannot build. Nothing pinned that here, and the scaffold
        // silently regressed; this is the assertion that would have caught it.
        let manifest = std::fs::read_to_string(location.join("project.json")).unwrap();
        assert!(
            manifest.contains("\"description\""),
            "init-pkg must scaffold a `description`; without it the new package \
             fails validation with 2-200-0016. Manifest was:\n{manifest}"
        );
    }

    #[test]
    fn init_project_refuses_to_overwrite_existing_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let location = dir.path().join("app");
        std::fs::create_dir_all(&location).expect("location");
        init_project(&location).expect("first init");
        // Re-running refuses to clobber the existing project.json.
        let err = init_project(&location).unwrap_err();
        assert!(err.contains("refusing to overwrite"));
    }

    #[test]
    fn write_new_file_never_follows_a_symlink_at_the_target() {
        let dir = tempfile::tempdir().expect("temp dir");
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"original").expect("victim");
        let target = dir.path().join("project.json");
        std::os::unix::fs::symlink(&victim, &target).expect("symlink");
        let err = write_new_file(&target, "clobbered".to_string()).unwrap_err();
        assert!(err.contains("refusing to overwrite"), "{err}");
        assert_eq!(std::fs::read(&victim).expect("victim"), b"original");

        // A *dangling* symlink passed the old `exists()` check and was followed
        // onto its target; `create_new` refuses it too.
        let dangling = dir.path().join("dangling.mfb");
        std::os::unix::fs::symlink(dir.path().join("absent"), &dangling).expect("symlink");
        let err = write_new_file(&dangling, "clobbered".to_string()).unwrap_err();
        assert!(err.contains("refusing to overwrite"), "{err}");
        assert!(!dir.path().join("absent").exists());

        // A fresh path still writes.
        let fresh = dir.path().join("fresh.mfb");
        write_new_file(&fresh, "hello".to_string()).expect("fresh write");
        assert_eq!(std::fs::read_to_string(&fresh).expect("fresh"), "hello");
    }

    #[test]
    fn init_reports_create_dir_failure() {
        // `src` already exists as a *file*, so `create_dir_all(location/src)`
        // fails and the error closure runs.
        let dir = tempfile::tempdir().expect("temp dir");
        let location = dir.path().join("app");
        std::fs::create_dir_all(&location).expect("location");
        std::fs::write(location.join("src"), "not a directory").expect("blocker file");
        assert!(init_project(&location)
            .unwrap_err()
            .contains("failed to create source directory"));

        let package_location = dir.path().join("lib");
        std::fs::create_dir_all(&package_location).expect("location");
        std::fs::write(package_location.join("src"), "not a directory").expect("blocker file");
        assert!(init_package_project(&package_location)
            .unwrap_err()
            .contains("failed to create source directory"));
    }

    #[test]
    fn write_new_file_refuses_existing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("file.txt");
        write_new_file(&path, "one".to_string()).expect("first write");
        assert!(write_new_file(&path, "two".to_string())
            .unwrap_err()
            .contains("refusing to overwrite"));
        // The original contents survive.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
    }
}
