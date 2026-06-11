use std::env;
use std::fs;
use std::path::Path;
use std::process;
use tinyjson::JsonValue;

const USAGE: &str = "Usage: mfb <command> <arguments>\n\nCommands:\n  help                 Show this message\n  init <location>      Create a new MFBASIC project";

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("help") | None => {
            println!("{USAGE}");
        }
        Some("init") => {
            let Some(location) = args.next() else {
                eprintln!("error: mfb init requires <location>\n\n{USAGE}");
                process::exit(2);
            };

            if args.next().is_some() {
                eprintln!("error: mfb init accepts exactly one <location>\n\n{USAGE}");
                process::exit(2);
            }

            if let Err(err) = init_project(Path::new(&location)) {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        Some(command) => {
            eprintln!("error: unknown command '{command}'\n\n{USAGE}");
            process::exit(2);
        }
    }
}

fn init_project(location: &Path) -> Result<(), String> {
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

fn write_new_file(path: &Path, contents: String) -> Result<(), String> {
    if path.exists() {
        return Err(format!("refusing to overwrite '{}'", path.display()));
    }

    fs::write(path, contents).map_err(|err| format!("failed to write '{}': {err}", path.display()))
}

fn project_manifest(location: &Path) -> String {
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
            "  \"targets\": [\"native\"]\n",
            "}}"
        ),
        name
    )
}

fn json_string(value: &str) -> String {
    JsonValue::String(value.to_string())
        .stringify()
        .unwrap_or_else(|_| "\"mfb_project\"".to_string())
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
    "SUB main()\n  io.print(\"Hello World\")\nEND SUB\n".to_string()
}
