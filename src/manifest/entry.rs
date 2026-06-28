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
