//! Fold `os::getConfigOr` calls into constants from the build's selected config
//! (plan-167-B).
//!
//! `os::getConfigOr(key, default)` is a [`crate::codegen::registry::Body::Intrinsic`]
//! with no lowering of its own: every call is replaced here, before NIR exists.
//! A call whose key the config declares becomes a `Const` holding the declared
//! value; a call whose key it does not declare becomes its `default`, which
//! `ir::shape` has already required to be a literal, so nothing with a side
//! effect is dropped. A call that survives (a position this walk does not
//! reach) is not lowered as anything: the backend's capability check rejects
//! the unknown runtime call `os.getConfigOr`, so the build fails rather than
//! running a wrong value.
//!
//! The config is the CONSUMING executable's ([`IrProject::selected_config`]).
//! `nir::merge_packages` applies it to the executable's own IR and to every
//! imported package's IR as it is decoded, before package symbols are renamed,
//! so a package's calls read the app's values and a diagnostic can name the
//! package the call came from.

use crate::ir::{IrProject, IrValue};
use crate::manifest::config::{ConfigType, ConfigValue, SelectedConfig};
use crate::operators::UnaryOp;
use crate::types::ParameterType;
use std::path::{Path, PathBuf};

/// The IR call target of `os::getConfigOr`.
const GET_CONFIG_OR: &str = "os.getConfigOr";

/// Why one call could not fold.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConfigFoldProblem {
    /// The config declares `key` with a type other than the default's.
    TypeMismatch {
        key: String,
        declared: ConfigType,
        requested: ConfigType,
    },
    /// An argument is not a literal. `ir::shape` rejects this in source, so
    /// only a `.mfp` that was not built by this compiler can carry it.
    NotLiteral { param: &'static str },
}

/// One call that could not fold, located for a diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConfigFoldError {
    pub(crate) path: PathBuf,
    pub(crate) line: u32,
    pub(crate) column: u32,
    /// The imported package the call is in, or `None` for the executable's
    /// own source.
    pub(crate) package: Option<String>,
    pub(crate) problem: ConfigFoldProblem,
}

impl ConfigFoldError {
    pub(crate) fn rule(&self) -> &'static str {
        match self.problem {
            ConfigFoldProblem::TypeMismatch { .. } => "OS_CONFIG_TYPE_MISMATCH",
            ConfigFoldProblem::NotLiteral { .. } => "OS_CONFIG_ARGUMENT_NOT_LITERAL",
        }
    }

    pub(crate) fn detail(&self, config: &str) -> String {
        let mut detail = match &self.problem {
            ConfigFoldProblem::TypeMismatch {
                key,
                declared,
                requested,
            } => format!(
                "Config `{config}` declares `{key}` as {}, but this call's default is {}.",
                declared.name(),
                requested.name()
            ),
            ConfigFoldProblem::NotLiteral { param } => format!(
                "`os::getConfigOr` needs a literal `{param}`; the value is read when the program is built."
            ),
        };
        if let Some(package) = &self.package {
            detail.push_str(&format!(" The call is in package `{package}`."));
        }
        detail
    }
}

/// Replace every `os::getConfigOr` call in `project` — function bodies at any
/// depth and global initializers — with its folded value. `package` names the
/// imported package `project` was decoded from, or is `None` for the
/// executable's own IR; it decides how a diagnostic names the call's file.
pub(crate) fn fold_config_calls(
    project: &mut IrProject,
    config: &SelectedConfig,
    package: Option<&str>,
) -> Vec<ConfigFoldError> {
    let path_of = |file: &str| match package {
        // A package's source paths are relative to the package's own project,
        // which an imported `.mfp` does not record; name the package instead.
        Some(package) => PathBuf::from(package).join(file),
        None => config.project_dir.join(file),
    };
    let mut errors = Vec::new();
    for function in &mut project.functions {
        let path = path_of(&function.file);
        for op in &mut function.body {
            op.visit_values_mut(&mut |value| {
                fold_value(value, config, &path, package, &mut errors)
            });
        }
    }
    for binding in &mut project.bindings {
        let path = path_of(&binding.file);
        if let Some(value) = &mut binding.value {
            crate::ir::visit_value_mut(value, &mut |value| {
                fold_value(value, config, &path, package, &mut errors)
            });
        }
    }
    errors
}

/// A literal as IR spells it: a constant, or a negated numeric constant.
fn is_literal(value: &IrValue) -> bool {
    match value {
        IrValue::Const { .. } => true,
        IrValue::Unary {
            op: UnaryOp::Negate,
            operand,
            ..
        } => matches!(operand.as_ref(), IrValue::Const { .. }),
        _ => false,
    }
}

fn config_type(type_: &ParameterType) -> Option<ConfigType> {
    match type_ {
        ParameterType::String => Some(ConfigType::String),
        ParameterType::Integer => Some(ConfigType::Integer),
        ParameterType::Float => Some(ConfigType::Float),
        _ => None,
    }
}

/// The `Const.value` text for a config value: the decoded String (as a String
/// literal's `Const` holds it), the decimal Integer, or the Float exactly as
/// written in the config.
fn const_text(value: &ConfigValue) -> String {
    match value {
        ConfigValue::String(text) => text.clone(),
        ConfigValue::Integer(number) => number.to_string(),
        ConfigValue::Float(_, lexeme) => lexeme.clone(),
    }
}

fn fold_value(
    value: &mut IrValue,
    config: &SelectedConfig,
    path: &Path,
    package: Option<&str>,
    errors: &mut Vec<ConfigFoldError>,
) {
    let IrValue::Call {
        target,
        args,
        type_,
        loc,
    } = value
    else {
        return;
    };
    if target != GET_CONFIG_OR || args.len() != 2 {
        return;
    }
    let Some(requested) = config_type(type_) else {
        return;
    };
    let mut error = |problem| {
        errors.push(ConfigFoldError {
            path: path.to_path_buf(),
            line: loc.line,
            column: loc.column,
            package: package.map(str::to_string),
            problem,
        })
    };
    let IrValue::Const {
        type_: ParameterType::String,
        value: key,
    } = &args[0]
    else {
        error(ConfigFoldProblem::NotLiteral { param: "key" });
        return;
    };
    if !is_literal(&args[1]) {
        error(ConfigFoldProblem::NotLiteral { param: "default" });
        return;
    }
    let folded = match config.values.get(key) {
        None => args.pop().expect("two arguments"),
        Some(declared) if declared.config_type() == requested => IrValue::Const {
            type_: type_.clone(),
            value: const_text(declared),
        },
        Some(declared) => {
            let key = key.clone();
            error(ConfigFoldProblem::TypeMismatch {
                key,
                declared: declared.config_type(),
                requested,
            });
            return;
        }
    };
    *value = folded;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{test_support, IrBinding, IrSourceLoc};

    fn selected(values: &[(&str, ConfigValue)]) -> SelectedConfig {
        SelectedConfig {
            values: values
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone()))
                .collect(),
            project_dir: PathBuf::from("proj"),
            ..SelectedConfig::default()
        }
    }

    fn loc(line: u32, column: u32) -> IrSourceLoc {
        IrSourceLoc { line, column }
    }

    fn string(text: &str) -> IrValue {
        IrValue::Const {
            type_: ParameterType::String,
            value: text.to_string(),
        }
    }

    fn constant(type_: ParameterType, text: &str) -> IrValue {
        IrValue::Const {
            type_,
            value: text.to_string(),
        }
    }

    fn call(key: &str, default: IrValue, type_: ParameterType) -> IrValue {
        IrValue::Call {
            target: GET_CONFIG_OR.to_string(),
            args: vec![string(key), default],
            type_,
            loc: loc(3, 7),
        }
    }

    fn binding(value: IrValue) -> IrBinding {
        IrBinding {
            name: "g".to_string(),
            visibility: "public".to_string(),
            mutable: false,
            type_: ParameterType::String,
            value: Some(value),
            loc: loc(3, 1),
            file: "src/main.mfb".to_string(),
            explicit_type: false,
        }
    }

    /// Fold one value in an otherwise empty project's global initializer.
    fn fold_one(value: IrValue, config: &SelectedConfig) -> (IrValue, Vec<ConfigFoldError>) {
        let mut project = test_support::project_fixture("demo", vec![], vec![]);
        project.bindings.push(binding(value));
        let errors = fold_config_calls(&mut project, config, None);
        (project.bindings[0].value.clone().expect("value"), errors)
    }

    fn const_of(value: &IrValue) -> (ParameterType, String) {
        match value {
            IrValue::Const { type_, value } => (type_.clone(), value.clone()),
            _ => panic!("not folded to a constant"),
        }
    }

    #[test]
    fn found_missing_and_mismatched_for_each_type() {
        let config = selected(&[
            ("s", ConfigValue::String("text".to_string())),
            ("i", ConfigValue::Integer(42)),
            ("f", ConfigValue::Float(2.5, "2.5".to_string())),
        ]);
        let cases = [
            (ParameterType::String, "s", string("d"), "text", "i"),
            (
                ParameterType::Integer,
                "i",
                constant(ParameterType::Integer, "7"),
                "42",
                "f",
            ),
            (
                ParameterType::Float,
                "f",
                constant(ParameterType::Float, "1.0"),
                "2.5",
                "s",
            ),
        ];
        for (type_, found_key, default, found_text, wrong_key) in cases {
            let (found, errors) =
                fold_one(call(found_key, default.clone(), type_.clone()), &config);
            assert!(errors.is_empty());
            assert_eq!(const_of(&found), (type_.clone(), found_text.to_string()));

            let (missing, errors) =
                fold_one(call("absent", default.clone(), type_.clone()), &config);
            assert!(errors.is_empty());
            assert_eq!(const_of(&missing), const_of(&default));

            let (_, errors) = fold_one(call(wrong_key, default, type_.clone()), &config);
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].rule(), "OS_CONFIG_TYPE_MISMATCH");
            assert_eq!((errors[0].line, errors[0].column), (3, 7));
            assert_eq!(errors[0].path, PathBuf::from("proj/src/main.mfb"));
            match &errors[0].problem {
                ConfigFoldProblem::TypeMismatch { requested, .. } => {
                    assert_eq!(*requested, config_type(&type_).unwrap())
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn mismatch_detail_names_config_key_and_both_types() {
        let config = selected(&[("port", ConfigValue::Integer(80))]);
        let (_, errors) = fold_one(call("port", string("80"), ParameterType::String), &config);
        assert_eq!(
            errors[0].detail("release"),
            "Config `release` declares `port` as Integer, but this call's default is String."
        );
    }

    #[test]
    fn a_negative_default_is_kept_when_the_key_is_missing() {
        let negated = IrValue::Unary {
            op: UnaryOp::Negate,
            operand: Box::new(constant(ParameterType::Integer, "5")),
            type_: ParameterType::Integer,
            loc: loc(3, 30),
        };
        let (folded, errors) = fold_one(
            call("absent", negated, ParameterType::Integer),
            &selected(&[]),
        );
        assert!(errors.is_empty());
        assert!(matches!(
            folded,
            IrValue::Unary {
                op: UnaryOp::Negate,
                ..
            }
        ));
    }

    #[test]
    fn edge_values_keep_their_exact_text() {
        let config = selected(&[
            ("min", ConfigValue::Integer(i64::MIN)),
            ("tenth", ConfigValue::Float(0.1, "0.1".to_string())),
            ("big", ConfigValue::Float(1e300, "1e300".to_string())),
            ("tiny", ConfigValue::Float(5e-324, "5e-324".to_string())),
            ("negzero", ConfigValue::Float(-0.0, "-0.0".to_string())),
            ("text", ConfigValue::String("q\" b\\ nl\n é ✓".to_string())),
        ]);
        let expect = [
            ("min", ParameterType::Integer, "-9223372036854775808"),
            ("tenth", ParameterType::Float, "0.1"),
            ("big", ParameterType::Float, "1e300"),
            ("tiny", ParameterType::Float, "5e-324"),
            ("negzero", ParameterType::Float, "-0.0"),
            ("text", ParameterType::String, "q\" b\\ nl\n é ✓"),
        ];
        for (key, type_, text) in expect {
            let default = match type_ {
                ParameterType::String => string(""),
                ParameterType::Integer => constant(type_.clone(), "0"),
                _ => constant(type_.clone(), "0.0"),
            };
            let (folded, errors) = fold_one(call(key, default, type_.clone()), &config);
            assert!(errors.is_empty(), "{key}");
            assert_eq!(const_of(&folded), (type_, text.to_string()), "{key}");
        }
    }

    #[test]
    fn non_literal_arguments_from_a_package_are_reported() {
        let mut project = test_support::project_fixture("pkg", vec![], vec![]);
        project.bindings.push(binding(IrValue::Call {
            target: GET_CONFIG_OR.to_string(),
            args: vec![IrValue::Local("k".to_string()), string("d")],
            type_: ParameterType::String,
            loc: loc(4, 2),
        }));
        let errors = fold_config_calls(&mut project, &selected(&[]), Some("tiny"));
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].rule(), "OS_CONFIG_ARGUMENT_NOT_LITERAL");
        assert_eq!(errors[0].path, PathBuf::from("tiny/src/main.mfb"));
        assert!(errors[0]
            .detail("release")
            .ends_with("The call is in package `tiny`."));
    }

    /// Every IR position a call can sit in folds: a statement expression, a
    /// nested argument, a record field, a list literal, a lambda body (lifted to
    /// its own function), a global initializer, and every nested statement
    /// kind. A position the walk misses leaves the call behind, and the
    /// assertion below finds it.
    #[test]
    fn every_position_folds() {
        let source = r#"
IMPORT os
IMPORT io

TYPE Box
  label AS String
END TYPE

LET g AS Integer = os::getConfigOr("n", 1)

FUNC main AS Integer
  io::print(os::getConfigOr("s", "a"))
  io::print(toString(os::getConfigOr("n", 2) + 1))
  LET b AS Box = Box[label := os::getConfigOr("s", "b")]
  LET xs AS List OF String = [os::getConfigOr("s", "c")]
  LET f = LAMBDA() -> os::getConfigOr("s", "d")
  IF os::getConfigOr("n", 3) > 0 THEN
    io::print(os::getConfigOr("s", "e"))
  ELSE
    io::print(os::getConfigOr("s", "f"))
  END IF
  FOR i = os::getConfigOr("n", 0) TO os::getConfigOr("n", 4)
    io::print(os::getConfigOr("s", "g"))
  NEXT
  FOR EACH x IN xs
    io::print(x & os::getConfigOr("s", "k"))
  NEXT
  WHILE os::getConfigOr("n", 5) < 0
    io::print(os::getConfigOr("s", "h"))
  END WHILE
  MATCH os::getConfigOr("n", 6)
    CASE 1
      io::print(os::getConfigOr("s", "i"))
    CASE ELSE
      io::print(os::getConfigOr("s", "j"))
  END MATCH
  io::print(b.label & f())
  RETURN os::getConfigOr("n", 0)
END FUNC
"#;
        let mut project = crate::ir::tests::helpers::lower_src(source);
        let count = |project: &mut IrProject| {
            let mut calls = 0;
            let mut count_value = |value: &mut IrValue| {
                if matches!(value, IrValue::Call { target, .. } if target == GET_CONFIG_OR) {
                    calls += 1;
                }
            };
            for function in &mut project.functions {
                for op in &mut function.body {
                    op.visit_values_mut(&mut count_value);
                }
            }
            for binding in &mut project.bindings {
                if let Some(value) = &mut binding.value {
                    crate::ir::visit_value_mut(value, &mut count_value);
                }
            }
            calls
        };
        assert!(
            count(&mut project) >= 18,
            "the source must reach the IR as calls"
        );
        let config = selected(&[
            ("n", ConfigValue::Integer(9)),
            ("s", ConfigValue::String("x".to_string())),
        ]);
        let errors = fold_config_calls(&mut project, &config, None);
        assert!(errors.is_empty());
        assert_eq!(count(&mut project), 0, "a call survived the fold");
    }
}
