mod arch;
mod ast;
mod builtins;
mod bytecode;
mod ir;
mod lexer;
mod man;
mod os;
mod resolver;
mod rules;
mod target;
mod typecheck;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use tinyjson::JsonValue;

const USAGE: &str = "Usage: mfb <command> <arguments>\n\nCommands:\n  help                        Show this message\n  init <location>             Create a new MFBASIC project\n  build [-ast|-ir|-bc|-bin] [location] Validate and build an MFBASIC project\n  man [package] [function]    Show built-in package and function help";

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
        Some("build") => {
            let build_options = match parse_build_options(args.collect()) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    process::exit(2);
                }
            };

            if let Err(()) = build_project(&build_options) {
                process::exit(1);
            }
        }
        Some("man") => {
            let man_args = args.collect::<Vec<_>>();
            if let Err(err) = show_man(&man_args) {
                eprintln!("error: {err}");
                process::exit(2);
            }
        }
        Some(command) => {
            eprintln!("error: unknown command '{command}'\n\n{USAGE}");
            process::exit(2);
        }
    }
}

struct BuildOptions {
    location: PathBuf,
    output: BuildOutput,
}

enum BuildOutput {
    Validate,
    Ast,
    Ir,
    Bytecode,
    Binary,
}

fn parse_build_options(args: Vec<String>) -> Result<BuildOptions, String> {
    let mut location = None;
    let mut output = BuildOutput::Validate;

    for arg in args {
        if arg == "-ast" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::Ast;
        } else if arg == "-ir" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::Ir;
        } else if arg == "-bc" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::Bytecode;
        } else if arg == "-bin" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::Binary;
        } else if arg.starts_with('-') {
            return Err(format!("unknown build option `{arg}`"));
        } else if location.replace(PathBuf::from(&arg)).is_some() {
            return Err("mfb build accepts at most one [location]".to_string());
        }
    }

    Ok(BuildOptions {
        location: location.unwrap_or_else(|| PathBuf::from(".")),
        output,
    })
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

fn build_project(options: &BuildOptions) -> Result<(), ()> {
    let target = target::BuildTarget::host();
    let project_path = options.location.join("project.json");
    let manifest = validate_project_manifest(&project_path)?;
    let project_kind = project_kind(&manifest);
    if project_kind == "package" {
        eprintln!("error: package builds are not supported yet");
        return Err(());
    }

    let project_name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name");
    let ast = ast::parse_project(project_name, &options.location, &manifest)?;
    resolver::resolve_project(&options.location, &manifest, &ast)?;
    let entry = validate_entry_point(&options.location, &manifest, &ast)?;
    typecheck::check_project(&options.location, &ast)?;

    match options.output {
        BuildOutput::Validate => {
            if project_kind == "executable" {
                if has_external_packages(&manifest) {
                    eprintln!(
                        "error: executable binary output does not support external packages yet"
                    );
                    return Err(());
                }

                let ir = ir::lower_project(&ast, entry.clone());
                let executable_path = os::write_executable(&options.location, &ir, &target)
                    .map_err(|err| {
                        eprintln!("error: {err}");
                    })?;
                println!("Wrote executable to {}", executable_path.display());
            } else {
                println!(
                    "Validated MFBASIC project at {}",
                    options.location.display()
                );
            }
        }
        BuildOutput::Ast => {
            let ast_path = ast::write_ast(&options.location, &ast).map_err(|err| {
                eprintln!("error: {err}");
            })?;
            println!("Wrote AST to {}", ast_path.display());
        }
        BuildOutput::Ir => {
            let ir = ir::lower_project(&ast, entry.clone());
            let ir_path = ir::write_ir(&options.location, &ir).map_err(|err| {
                eprintln!("error: {err}");
            })?;
            println!("Wrote IR to {}", ir_path.display());
        }
        BuildOutput::Bytecode => {
            let ir = ir::lower_project(&ast, entry.clone());
            let version = manifest
                .get("version")
                .and_then(|value| value.get::<String>())
                .expect("validated project version");
            let bytecode_path = bytecode::write_bytecode_hex(&options.location, &ir, version)
                .map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            println!("Wrote bytecode hex to {}", bytecode_path.display());
        }
        BuildOutput::Binary => {
            if has_external_packages(&manifest) {
                eprintln!("error: binary output does not support external packages yet");
                return Err(());
            }

            let ir = ir::lower_project(&ast, entry);
            let binary_path =
                arch::write_binary_dump(&options.location, &ir, &target).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            println!("Wrote binary to {}", binary_path.display());
        }
    }

    Ok(())
}

fn show_man(args: &[String]) -> Result<(), String> {
    match args {
        [] => {
            print_man_index();
            Ok(())
        }
        [package_name] => {
            let package =
                man::package(package_name).ok_or_else(|| unknown_package_error(package_name))?;
            print_package_man(package);
            Ok(())
        }
        [package_name, function_name] => {
            let package =
                man::package(package_name).ok_or_else(|| unknown_package_error(package_name))?;
            let function = man::function(package, function_name).ok_or_else(|| {
                format!(
                    "unknown function `{function_name}` in package `{package_name}`\n\nRun `mfb man {package_name}` to list available functions."
                )
            })?;
            if let Some(page) = man::function_page(package, function_name) {
                print_man_page(page);
            } else {
                print_function_man(package, function);
            }
            Ok(())
        }
        _ => Err(format!("mfb man accepts at most two arguments\n\n{USAGE}")),
    }
}

fn print_man_index() {
    println!("Usage: mfb man [package] [function]");
    println!();
    println!("Show help for built-in packages and functions.");
    println!();
    println!("Examples:");
    println!("  mfb man");
    println!("  mfb man general");
    println!("  mfb man io print");
    println!();
    println!("Packages:");
    for package in man::packages() {
        println!("  {:<8} {}", package.name, package.summary);
    }
}

fn print_package_man(package: &man::PackageDoc) {
    println!("Package: {}", package.name);
    println!();
    println!("{}", package.summary);
    println!();
    println!("Usage:");
    println!("  {}", package.usage);
    println!();
    println!("Functions:");
    for function in package.functions {
        println!("  {:<18} {}", function.name, function.summary);
    }
    println!();
    println!(
        "Run `mfb man {} <function>` for function signatures and examples.",
        package.name
    );
}

fn print_man_page(page: &str) {
    println!("{}", page.trim_end_matches('\n'));
}

fn print_function_man(package: &man::PackageDoc, function: &man::FunctionDoc) {
    println!("{} {}", package.name, function.name);
    println!();
    println!("{}", function.summary);
    println!();
    println!("Signature:");
    println!("  {}", function.signature);
    println!();
    println!("Example:");
    for line in function.example.lines() {
        println!("  {line}");
    }
}

fn unknown_package_error(package_name: &str) -> String {
    let packages = man::packages()
        .iter()
        .map(|package| package.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!("unknown package `{package_name}`\n\nAvailable packages: {packages}")
}

fn validate_entry_point(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    ast: &ast::AstProject,
) -> Result<Option<ir::EntryPoint>, ()> {
    let kind = project_kind(manifest);
    if kind == "library" || kind == "package" {
        return Ok(None);
    }

    let entry = entry_point(manifest);

    for file in &ast.files {
        for item in &file.items {
            let ast::Item::Function(function) = item else {
                continue;
            };
            if function.name != entry {
                continue;
            }

            let returns = match function.kind {
                ast::FunctionKind::Sub => "Nothing",
                ast::FunctionKind::Func => function.return_type.as_deref().unwrap_or(""),
            };

            if matches!(function.kind, ast::FunctionKind::Func) && returns != "Integer" {
                rules::show_diagnostic(
                    "PROJECT_ENTRY_INVALID",
                    &format!("Executable FUNC entry `{entry}` must return Integer."),
                    &project_dir.join(&file.path),
                    function.line,
                    1,
                    1,
                );
                return Err(());
            }

            let accepts_args = match function.params.as_slice() {
                [] => false,
                [param] if param.type_name.as_deref() == Some("List OF String") => true,
                [param] => {
                    rules::show_diagnostic(
                        "PROJECT_ENTRY_INVALID",
                        &format!(
                            "Executable entry `{entry}` parameter `{}` must have type List OF String.",
                            param.name
                        ),
                        &project_dir.join(&file.path),
                        param.line,
                        1,
                        1,
                    );
                    return Err(());
                }
                _ => {
                    rules::show_diagnostic(
                        "PROJECT_ENTRY_INVALID",
                        &format!(
                            "Executable entry `{entry}` must declare zero parameters or one `args AS List OF String` parameter."
                        ),
                        &project_dir.join(&file.path),
                        function.line,
                        1,
                        1,
                    );
                    return Err(());
                }
            };

            if function.params.len() == 1 && function.params[0].default.is_some() {
                rules::show_diagnostic(
                    "PROJECT_ENTRY_INVALID",
                    &format!("Executable entry `{entry}` args parameter must not declare a default value."),
                    &project_dir.join(&file.path),
                    function.params[0].line,
                    1,
                    1,
                );
                return Err(());
            }

            return Ok(Some(ir::EntryPoint {
                name: entry.to_string(),
                returns: returns.to_string(),
                accepts_args,
            }));
        }
    }

    rules::show_diagnostic(
        "PROJECT_ENTRY_INVALID",
        &format!("Executable project must declare an entry point named `{entry}`."),
        &project_dir.join("project.json"),
        1,
        1,
        1,
    );
    Err(())
}

fn validate_project_manifest(project_path: &Path) -> Result<HashMap<String, JsonValue>, ()> {
    if !project_path.exists() {
        rules::show_diagnostic(
            "PROJECT_JSON_MISSING",
            "Run `mfb init <location>` first or build from a directory that contains project.json.",
            project_path,
            1,
            1,
            1,
        );
        return Err(());
    }

    let contents = fs::read_to_string(project_path).map_err(|err| {
        rules::show_diagnostic(
            "PROJECT_JSON_READ_FAILED",
            &err.to_string(),
            project_path,
            1,
            1,
            1,
        );
    })?;

    let manifest: JsonValue = contents.parse().map_err(|err: tinyjson::JsonParseError| {
        let column = err.column().max(1);
        rules::show_diagnostic(
            "PROJECT_JSON_PARSE_FAILED",
            &err.to_string(),
            project_path,
            err.line(),
            column,
            column + 1,
        );
    })?;

    let Some(manifest) = manifest.get::<HashMap<String, JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_ROOT_TYPE",
            "The top-level JSON value must be an object with project fields.",
            project_path,
            1,
            1,
            1,
        );
        return Err(());
    };

    let mut valid = true;

    for field in ["name", "version", "mfb"] {
        if !validate_required_string(manifest, project_path, &contents, field) {
            valid = false;
        }
    }

    if !validate_sources(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_optional_string(manifest, project_path, &contents, "entry") {
        valid = false;
    }

    if !validate_kind(manifest, project_path, &contents) {
        valid = false;
    }

    if valid {
        Ok(manifest.clone())
    } else {
        Err(())
    }
}

fn validate_required_string(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
    field: &str,
) -> bool {
    let Some(value) = manifest.get(field) else {
        let (line, column) = fallback_field_position(contents);
        rules::show_diagnostic(
            "PROJECT_JSON_REQUIRED_FIELD",
            &format!("Required field `{field}` is missing."),
            project_path,
            line,
            column,
            column + 1,
        );
        return false;
    };

    let (line, column) = field_position(contents, field);
    let Some(value) = value.get::<String>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            &format!("Field `{field}` must be a string."),
            project_path,
            line,
            column,
            column + field.len() + 2,
        );
        return false;
    };

    if value.trim().is_empty() {
        rules::show_diagnostic(
            "PROJECT_JSON_EMPTY_FIELD",
            &format!("Field `{field}` must contain a non-empty string."),
            project_path,
            line,
            column,
            column + field.len() + 2,
        );
        return false;
    }

    true
}

fn validate_optional_string(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
    field: &str,
) -> bool {
    let Some(value) = manifest.get(field) else {
        return true;
    };

    if value.get::<String>().is_some() {
        return true;
    }

    let (line, column) = field_position(contents, field);
    rules::show_diagnostic(
        "PROJECT_JSON_FIELD_TYPE",
        &format!("Field `{field}` must be a string when present."),
        project_path,
        line,
        column,
        column + field.len() + 2,
    );
    false
}

fn validate_sources(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(value) = manifest.get("sources") else {
        let (line, column) = fallback_field_position(contents);
        rules::show_diagnostic(
            "PROJECT_JSON_REQUIRED_FIELD",
            "Required field `sources` is missing.",
            project_path,
            line,
            column,
            column + 1,
        );
        return false;
    };

    let (line, column) = field_position(contents, "sources");
    let Some(sources) = value.get::<Vec<JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `sources` must be an array.",
            project_path,
            line,
            column,
            column + "\"sources\"".len(),
        );
        return false;
    };

    if sources.is_empty() {
        rules::show_diagnostic(
            "PROJECT_JSON_EMPTY_SOURCES",
            "Add at least one source entry, for example `{ \"root\": \"src\" }`.",
            project_path,
            line,
            column,
            column + "\"sources\"".len(),
        );
        return false;
    }

    let mut valid = true;
    for (index, source) in sources.iter().enumerate() {
        let Some(source) = source.get::<HashMap<String, JsonValue>>() else {
            rules::show_diagnostic(
                "PROJECT_JSON_FIELD_TYPE",
                &format!("Source entry #{index} must be an object."),
                project_path,
                line,
                column,
                column + "\"sources\"".len(),
            );
            valid = false;
            continue;
        };

        if !validate_required_string(source, project_path, contents, "root") {
            valid = false;
        }
    }

    valid
}

fn validate_kind(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(value) = manifest.get("kind") else {
        return true;
    };

    let (line, column) = field_position(contents, "kind");
    let Some(kind) = value.get::<String>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `kind` must be a string when present.",
            project_path,
            line,
            column,
            column + "\"kind\"".len(),
        );
        return false;
    };

    if !matches!(kind.as_str(), "library" | "executable" | "package") {
        rules::show_diagnostic(
            "PROJECT_JSON_UNKNOWN_KIND",
            "Expected `library`, `executable`, or `package`; continuing validation.",
            project_path,
            line,
            column,
            column + "\"kind\"".len(),
        );
    }

    true
}

fn project_kind(manifest: &HashMap<String, JsonValue>) -> &str {
    manifest
        .get("kind")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        .unwrap_or("executable")
}

fn entry_point(manifest: &HashMap<String, JsonValue>) -> &str {
    manifest
        .get("entry")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        .unwrap_or("main")
}

fn has_external_packages(manifest: &HashMap<String, JsonValue>) -> bool {
    manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .is_some_and(|packages| !packages.is_empty())
}

fn field_position(contents: &str, field: &str) -> (usize, usize) {
    let needle = format!("\"{field}\"");
    for (index, line) in contents.lines().enumerate() {
        if let Some(column) = line.find(&needle) {
            return (index + 1, column + 1);
        }
    }

    fallback_field_position(contents)
}

fn fallback_field_position(contents: &str) -> (usize, usize) {
    if contents.is_empty() {
        (1, 1)
    } else {
        (contents.lines().count().max(1), 1)
    }
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

pub(crate) fn json_string(value: &str) -> String {
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
    "IMPORT io\n\nSUB main()\n  io.print(\"Hello World\")\nEND SUB\n".to_string()
}
