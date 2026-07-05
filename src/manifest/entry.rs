use std::collections::HashMap;
use std::path::Path;
use tinyjson::JsonValue;

use crate::ast;
use crate::ir;
use crate::rules;

use super::{entry_point, project_kind};

pub(crate) fn validate_entry_point(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    ast: &ast::AstProject,
) -> Result<Option<ir::EntryPoint>, ()> {
    let kind = project_kind(manifest);
    if kind == "package" {
        return Ok(None);
    }

    let entry = entry_point(manifest);
    let mut matches = Vec::new();

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

            matches.push((
                file.path.clone(),
                function.line,
                entry.to_string(),
                returns.to_string(),
                accepts_args,
            ));
        }
    }

    if matches.len() > 1 {
        let (path, line, _, _, _) = &matches[1];
        rules::show_diagnostic(
            "PROJECT_ENTRY_INVALID",
            &format!(
                "Executable project must declare exactly one entry point named `{entry}`; found multiple matching declarations."
            ),
            &project_dir.join(path),
            *line,
            1,
            1,
        );
        return Err(());
    }

    if let Some((_, _, name, returns, accepts_args)) = matches.pop() {
        return Ok(Some(ir::EntryPoint {
            name,
            returns,
            accepts_args,
        }));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(kind: &str, entry: Option<&str>) -> HashMap<String, JsonValue> {
        let mut map = HashMap::new();
        map.insert("kind".to_string(), JsonValue::String(kind.to_string()));
        if let Some(entry) = entry {
            map.insert("entry".to_string(), JsonValue::String(entry.to_string()));
        }
        map
    }

    fn project(src: &str) -> ast::AstProject {
        let path = std::path::Path::new("main.mfb");
        let file = ast::parse_source(path, "main.mfb", src).expect("parse source");
        ast::AstProject {
            name: "demo".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn package_kind_returns_none() {
        let ast = project("FUNC helper() AS Integer\n  RETURN 0\nEND FUNC\n");
        let result =
            validate_entry_point(std::path::Path::new("."), &manifest("package", None), &ast);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn func_entry_returning_integer_accepts_no_args() {
        let ast = project("FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n");
        let entry = validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", None),
            &ast,
        )
        .expect("ok")
        .expect("entry");
        assert_eq!(entry.name, "main");
        assert_eq!(entry.returns, "Integer");
        assert!(!entry.accepts_args);
    }

    #[test]
    fn sub_entry_accepts_args_parameter() {
        let ast = project("SUB main(args AS List OF String)\n  LET n AS Integer = 0\nEND SUB\n");
        let entry = validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", None),
            &ast,
        )
        .expect("ok")
        .expect("entry");
        assert_eq!(entry.returns, "Nothing");
        assert!(entry.accepts_args);
    }

    #[test]
    fn func_entry_not_returning_integer_is_error() {
        let ast = project("FUNC main() AS String\n  RETURN \"x\"\nEND FUNC\n");
        assert!(validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", None),
            &ast
        )
        .is_err());
    }

    #[test]
    fn wrong_arg_type_is_error() {
        let ast = project("FUNC main(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n");
        assert!(validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", None),
            &ast
        )
        .is_err());
    }

    #[test]
    fn too_many_params_is_error() {
        let ast = project(
            "FUNC main(a AS List OF String, b AS Integer) AS Integer\n  RETURN 0\nEND FUNC\n",
        );
        assert!(validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", None),
            &ast
        )
        .is_err());
    }

    #[test]
    fn args_default_value_is_error() {
        let ast =
            project("FUNC main(args AS List OF String = []) AS Integer\n  RETURN 0\nEND FUNC\n");
        assert!(validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", None),
            &ast
        )
        .is_err());
    }

    #[test]
    fn multiple_entries_is_error() {
        let ast = project(
            "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\nFUNC main() AS Integer\n  RETURN 1\nEND FUNC\n",
        );
        assert!(validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", None),
            &ast
        )
        .is_err());
    }

    #[test]
    fn missing_entry_is_error() {
        let ast = project("FUNC other() AS Integer\n  RETURN 0\nEND FUNC\n");
        assert!(validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", None),
            &ast
        )
        .is_err());
    }

    #[test]
    fn custom_entry_name_is_honored() {
        let ast = project("FUNC run() AS Integer\n  RETURN 0\nEND FUNC\n");
        let entry = validate_entry_point(
            std::path::Path::new("."),
            &manifest("executable", Some("run")),
            &ast,
        )
        .expect("ok")
        .expect("entry");
        assert_eq!(entry.name, "run");
    }
}
