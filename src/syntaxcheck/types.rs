use super::helpers::*;
use super::*;

impl<'a> SyntaxChecker<'a> {
    /// Parse a collection element / `Map` value type, honoring the `RES` marker
    /// (`List OF RES File`). The marker wraps the element in [`Type::Res`]; the
    /// element validation later checks it matches the element's resource-ness.
    pub(super) fn parse_collection_element_type(&self, name: &str) -> Type {
        if let Some(inner) = name.strip_prefix("RES ") {
            return Type::Res(Box::new(self.parse_type(inner)));
        }
        self.parse_type(name)
    }

    pub(super) fn parse_type(&self, name: &str) -> Type {
        let name = builtins::thread::strip_type_group(name);
        // A package-qualified built-in type (`net.Url`, `http.Result`) resolves to
        // its bare internal id (plan-03-http.md §A.1/§B.2).
        if let Some(bare) = builtins::qualified_builtin_type(name) {
            return Type::User(bare);
        }
        if let Some(rest) = name.strip_prefix("ISOLATED FUNC(") {
            return self.parse_function_type(rest, true);
        }
        if let Some(rest) = name.strip_prefix("FUNC(") {
            return self.parse_function_type(rest, false);
        }
        if let Some(element) = name.strip_prefix("List OF ") {
            return Type::List(Box::new(self.parse_collection_element_type(element)));
        }
        if let Some(success) = name.strip_prefix("Result OF ") {
            return Type::Result(Box::new(self.parse_type(success)));
        }
        if let Some((kind, message, resource, output)) = builtins::thread::thread_parts_full(name) {
            let resource = resource.map(|resource| Box::new(self.parse_type(resource)));
            if kind == builtins::thread::THREAD_WORKER_TYPE {
                return Type::ThreadWorker(
                    Box::new(self.parse_type(message)),
                    resource,
                    Box::new(self.parse_type(output)),
                );
            }
            return Type::Thread(
                Box::new(self.parse_type(message)),
                resource,
                Box::new(self.parse_type(output)),
            );
        }
        if let Some(rest) = name.strip_prefix("Map OF ") {
            if let Some((key, value)) = rest.split_once(" TO ") {
                return Type::Map(
                    Box::new(self.parse_type(key)),
                    Box::new(self.parse_collection_element_type(value)),
                );
            }
        }

        match name {
            "Boolean" => Type::Boolean,
            "Byte" => Type::Byte,
            "Error" => Type::Error,
            "ErrorLoc" => Type::ErrorLoc,
            "Fixed" => Type::Fixed,
            "Float" => Type::Float,
            "Integer" => Type::Integer,
            "Nothing" => Type::Nothing,
            "String" => Type::String,
            "Unknown" => Type::Unknown,
            "Result" => Type::Result(Box::new(Type::Unknown)),
            other if builtins::is_builtin_type(other) => Type::User(other.to_string()),
            other if self.user_types.contains(other) => Type::User(other.to_string()),
            other => Type::User(other.to_string()),
        }
    }

    pub(super) fn parse_function_type(&self, rest: &str, isolated: bool) -> Type {
        let Some((params, return_type)) = rest.split_once(") AS ") else {
            return Type::Unknown;
        };
        let params = if params.trim().is_empty() {
            Vec::new()
        } else {
            params
                .split(", ")
                .map(|param| self.parse_type(param))
                .collect()
        };
        Type::Function {
            params,
            return_type: Box::new(self.parse_type(return_type)),
            isolated,
        }
    }

    pub(super) fn compatible(&self, expected: &Type, actual: &Type) -> bool {
        if matches!(expected, Type::Unknown) || matches!(actual, Type::Unknown) {
            return true;
        }
        // The `RES` element marker is an ownership-axis annotation (§15.6), not a
        // distinct value type: a `File` value fits a `RES File` slot and vice
        // versa. Strip it before comparing.
        let (expected, actual) = (strip_res(expected), strip_res(actual));
        match (expected, actual) {
            (Type::List(expected), Type::List(actual)) => self.compatible(expected, actual),
            (Type::Map(expected_key, expected_value), Type::Map(actual_key, actual_value)) => {
                self.compatible(expected_key, actual_key)
                    && self.compatible(expected_value, actual_value)
            }
            (Type::Result(expected), Type::Result(actual)) => self.compatible(expected, actual),
            (
                Type::Thread(expected_message, expected_resource, expected_output),
                Type::Thread(actual_message, actual_resource, actual_output),
            ) => {
                self.compatible(expected_message, actual_message)
                    && self.compatible_optional(expected_resource, actual_resource)
                    && self.compatible(expected_output, actual_output)
            }
            (
                Type::ThreadWorker(expected_message, expected_resource, expected_output),
                Type::ThreadWorker(actual_message, actual_resource, actual_output),
            ) => {
                self.compatible(expected_message, actual_message)
                    && self.compatible_optional(expected_resource, actual_resource)
                    && self.compatible(expected_output, actual_output)
            }
            (
                Type::Function {
                    params: expected_params,
                    return_type: expected_return,
                    isolated: expected_isolated,
                },
                Type::Function {
                    params: actual_params,
                    return_type: actual_return,
                    isolated: actual_isolated,
                },
            ) => {
                (!expected_isolated || *actual_isolated)
                    && expected_params.len() == actual_params.len()
                    && expected_params
                        .iter()
                        .zip(actual_params.iter())
                        .all(|(expected, actual)| self.compatible(expected, actual))
                    && self.compatible(expected_return, actual_return)
            }
            (Type::User(expected_name), Type::User(actual_name)) => {
                // An imported package's types are registered under their bare name
                // (`Db`), while a qualified reference written by the importer
                // resolves to `binding.Db` (plan-link-update.md §5a). Treat a
                // qualified name as equal to its bare form so an imported
                // resource/user type returned from a package function matches the
                // importer's `binding::Type` annotation.
                let expected_bare = expected_name.rsplit('.').next().unwrap_or(expected_name);
                let actual_bare = actual_name.rsplit('.').next().unwrap_or(actual_name);
                expected_name == actual_name
                    || expected_bare == actual_bare
                    || self
                        .type_infos
                        .get(expected_name)
                        .or_else(|| self.type_infos.get(expected_bare))
                        .is_some_and(|info| {
                            matches!(info.kind, TypeDeclKind::Union)
                                && info
                                    .variants
                                    .iter()
                                    .any(|variant| variant.name == *actual_bare)
                        })
            }
            _ => expected == actual,
        }
    }

    /// Compatibility for the optional resource plane of a thread type: both
    /// absent, or both present and compatible.
    pub(super) fn compatible_optional(
        &self,
        expected: &Option<Box<Type>>,
        actual: &Option<Box<Type>>,
    ) -> bool {
        match (expected, actual) {
            (None, None) => true,
            (Some(expected), Some(actual)) => self.compatible(expected, actual),
            _ => false,
        }
    }

    pub(super) fn expression_compatible(
        &self,
        expected: &Type,
        actual: &Type,
        expression: Option<&Expression>,
    ) -> bool {
        if self.compatible(expected, actual) {
            return true;
        }
        match (expected, actual, expression) {
            (Type::Byte, Type::Integer, Some(Expression::Number(value))) => value
                .parse::<u16>()
                .is_ok_and(|number| number <= u8::MAX as u16),
            (Type::Fixed, Type::Integer | Type::Float, Some(Expression::Number(_))) => true,
            (
                Type::Fixed,
                Type::Integer | Type::Float,
                Some(Expression::Unary {
                    operator, operand, ..
                }),
            ) if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) => true,
            (
                Type::List(expected_element),
                Type::List(_),
                Some(Expression::ListLiteral(values)),
            ) => values.iter().all(|value| {
                let Some(actual_element) = numeric_literal_type(value) else {
                    return false;
                };
                self.expression_compatible(expected_element, &actual_element, Some(value))
            }),
            _ => false,
        }
    }

    pub(super) fn is_numeric(&self, type_: &Type) -> bool {
        matches!(
            type_,
            Type::Byte | Type::Fixed | Type::Float | Type::Integer | Type::Unknown
        )
    }

    pub(super) fn is_comparable(&self, type_: &Type) -> bool {
        self.is_comparable_with_seen(type_, &mut HashSet::new())
    }

    /// An operand acceptable on either side of a `String` ordering comparison
    /// (`<`, `>`, `<=`, `>=`). `Unknown` is permitted so a prior error does not
    /// cascade. Numeric operands are handled separately by `is_numeric`.
    pub(super) fn is_orderable_string(&self, type_: &Type) -> bool {
        matches!(type_, Type::String | Type::Unknown)
    }

    pub(super) fn is_comparable_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::ErrorLoc
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            Type::List(_)
            | Type::Map(_, _)
            | Type::Function { .. }
            | Type::Result(_)
            | Type::Res(_)
            | Type::Thread(..)
            | Type::ThreadWorker(..) => false,
            Type::User(name) => {
                if self.resource_registry.is_resource(name) || !seen.insert(name.clone()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return true;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => true,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .all(|field| self.is_comparable_with_seen(&field.type_, seen)),
                    TypeDeclKind::Union => false,
                };
                seen.remove(name);
                result
            }
        }
    }

    pub(super) fn require_comparable_type(
        &mut self,
        _file: &AstFile,
        _line: usize,
        _context: &str,
        type_: &Type,
    ) {
        if self.is_comparable(type_) {
            return;
        }
    }

    /// The argument mode for argument `index` of a call to `callee`. A call to a
    /// resource's *registered close op* consumes its single resource argument
    /// (overhaul invalidation event #1) — for native LINK resources this is the
    /// `LINK` CLOSE wrapper (plan-link-update.md §6). All other resource arguments
    /// borrow by default.
    pub(super) fn call_argument_mode(
        &self,
        callee: &str,
        index: usize,
        sig: &FunctionSig,
    ) -> ExprMode {
        let param_type = sig.params.get(index).map(|param| &param.type_);
        if index == 0 {
            if let Some(Type::User(name)) = param_type {
                let base = builtins::resource::base_resource_name(name);
                let is_close_op = self.resource_registry.close_function(base) == Some(callee)
                    || self.resource_registry.close_function(name.as_str()) == Some(callee)
                    // A re-export alias of the close op consumes too (§5a).
                    || self
                        .close_op_aliases
                        .get(callee)
                        .is_some_and(|type_name| type_name == base || type_name == name);
                if is_close_op {
                    return ExprMode::Transfer;
                }
            }
        }
        self.argument_mode_for_type(&param_type)
    }

    pub(super) fn argument_mode_for_type(&self, expected: &Option<&Type>) -> ExprMode {
        match expected {
            // Resources borrow by default: an ordinary call uses the handle for
            // the duration of the call but does not take ownership. Only the
            // fixed invalidation events (a registered close op, `thread::transfer`,
            // `RETURN`, and scope-drop) end a resource's life.
            Some(type_) if self.is_resource_type(type_) => ExprMode::Borrow,
            Some(type_) if !self.is_copyable_type(type_) => ExprMode::Transfer,
            _ => ExprMode::Read,
        }
    }

    pub(super) fn thread_argument_mode(&self, callee: &str, index: usize) -> ExprMode {
        match (callee, index) {
            // `thread.transfer` is resource-plane invalidation event #2: the
            // resource moves to the worker, so the sender binding is consumed.
            ("thread.start", 1) | ("thread.send", 1) | ("thread.transfer", 1) => ExprMode::Transfer,
            ("thread.start", _) | ("thread.send", _) | ("thread.transfer", _) => ExprMode::Borrow,
            _ => ExprMode::Borrow,
        }
    }

    /// Argument evaluation mode for a builtin collection op, keyed on the BARE op
    /// name. Callers pass the dequalified member (`append`, not
    /// `collections.append`); this is only ever reached for recognised builtin
    /// calls, so a freed bare name from user code never gets here
    /// (plan-01-functions.md §5).
    pub(super) fn general_argument_mode(&self, callee: &str, index: usize) -> ExprMode {
        if matches!(
            callee,
            "len"
                | "get"
                | "getOr"
                | "find"
                | "keys"
                | "values"
                | "hasKey"
                | "contains"
                | "forEach"
                | "transform"
                | "filter"
                | "reduce"
                | "sum"
        ) {
            return ExprMode::Read;
        }
        if matches!(
            callee,
            "removeAt" | "removeKey" | "replace" | "set" | "append" | "prepend" | "insert"
        ) {
            return if index == 0 {
                ExprMode::Transfer
            } else {
                ExprMode::Read
            };
        }
        ExprMode::Read
    }
}

#[cfg(test)]
mod types_tests {
    use crate::syntaxcheck::testutil::*;

    // ---- parse_type arms exercised through type annotations ----------------

    #[test]
    fn scalar_type_annotations_accept() {
        // Boolean, Byte, Fixed, Float, Integer, String, Nothing.
        assert!(accepts(
            "FUNC main AS Integer\n  LET a AS Boolean = TRUE\n  LET b AS Byte = toByte(1)\n  LET c AS Fixed = toFixed(\"1.5\")\n  LET d AS Float = 1.0\n  LET e AS Integer = 1\n  LET f AS String = \"x\"\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn list_map_result_annotations_accept() {
        assert!(accepts(
            "FUNC main AS Integer\n  LET xs AS List OF Integer = [1]\n  LET m AS Map OF String TO Integer = Map OF String TO Integer {}\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn function_type_annotation_accepts() {
        // FUNC(...) AS ... and ISOLATED FUNC(...) AS ... parse arms.
        assert!(accepts(
            "FUNC apply(f AS FUNC(Integer) AS Integer, x AS Integer) AS Integer\n  RETURN f(x)\nEND FUNC\nFUNC dbl(n AS Integer) AS Integer\n  RETURN n * 2\nEND FUNC\nFUNC main AS Integer\n  RETURN apply(dbl, 3)\nEND FUNC\n"
        ));
    }

    #[test]
    fn nested_function_type_empty_params() {
        // FUNC() AS Integer — empty parameter list arm.
        assert!(accepts(
            "FUNC run(f AS FUNC() AS Integer) AS Integer\n  RETURN f()\nEND FUNC\nFUNC zero AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN run(zero)\nEND FUNC\n"
        ));
    }

    #[test]
    fn thread_type_annotation_accepts() {
        // Thread OF ... TO ... parse arm (message/output).
        let src = "IMPORT thread\nFUNC main AS Integer\n  LET t AS Thread OF Integer TO Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn user_type_annotation_accepts() {
        assert!(accepts(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET p AS Point = Point[1, 2]\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- compatible / expression_compatible --------------------------------

    #[test]
    fn byte_literal_fits_byte() {
        // (Byte, Integer, Number) special case in expression_compatible.
        assert!(accepts(
            "FUNC main AS Integer\n  LET b AS Byte = 200\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn byte_literal_overflow_walks_false_branch() {
        // The `<= u8::MAX` guard's false arm runs here even though the actual
        // rejection for an out-of-range Byte is relocated to ir::verify.
        let _ = check_src("FUNC main AS Integer\n  LET b AS Byte = 300\n  RETURN 0\nEND FUNC\n");
    }

    #[test]
    fn fixed_from_integer_literal() {
        // (Fixed, Integer|Float, Number) arm.
        assert!(accepts(
            "FUNC main AS Integer\n  LET f AS Fixed = 5\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn fixed_from_negative_literal() {
        // (Fixed, Integer|Float, Unary '-') arm.
        assert!(accepts(
            "FUNC main AS Integer\n  LET f AS Fixed = -5\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn list_literal_element_compat() {
        // (List, List, ListLiteral) numeric-widening arm.
        assert!(accepts(
            "FUNC main AS Integer\n  LET xs AS List OF Fixed = [1, 2, 3]\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- comparability (list element in contains/find) ---------------------

    #[test]
    fn contains_on_list_of_record_is_walked() {
        // Exercises is_comparable_with_seen over a user Type record.
        let src = "IMPORT collections\nTYPE P\n  x AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET xs AS List OF P = [P[1]]\n  LET b = collections::contains(xs, P[1])\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn enum_comparable() {
        let src = "IMPORT collections\nENUM Color\n  Red\n  Green\nEND ENUM\nFUNC main AS Integer\n  LET xs AS List OF Color = [Color.Red]\n  LET b = collections::contains(xs, Color.Green)\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- string ordering comparison (is_orderable_string) ------------------

    #[test]
    fn string_ordering_comparison_accepts() {
        assert!(accepts(
            "FUNC main AS Boolean\n  RETURN \"a\" < \"b\"\nEND FUNC\n"
        ));
    }

    // ---- RES-marked collection element (parse_collection_element_type) ------

    #[test]
    fn res_marked_list_element_parses() {
        let src = "IMPORT fs\nFUNC take(xs AS List OF RES File) AS Integer\n  RETURN len(xs)\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn res_marked_map_value_parses() {
        let src = "IMPORT fs\nFUNC take(m AS Map OF String TO RES File) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- qualified builtin type reference (net.Url) ------------------------

    #[test]
    fn qualified_builtin_type_annotation() {
        let src = "IMPORT net\nFUNC main AS Integer\n  LET u AS net::Url = net::toUrl(\"http://x/\")\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- isolated function type annotation ---------------------------------

    #[test]
    fn isolated_function_type_annotation() {
        let src = "FUNC run(f AS ISOLATED FUNC(Integer) AS Integer) AS Integer\n  RETURN f(1)\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- compatible over Result / Thread / nested Map ----------------------

    #[test]
    fn result_and_thread_compatibility_walk() {
        // A worker whose message type is a nested Map exercises the Map arm of
        // compatible, and returning a Result-typed value walks Result compat.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF (Map OF String TO Integer) TO Integer, seed AS Map OF String TO Integer) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- union-variant compatibility (a variant fits its union) ------------

    #[test]
    fn union_variant_fits_union() {
        // Assigning a variant value to a union-typed binding walks the
        // User/User union-variant arm of compatible.
        let src = "TYPE A\n  x AS Integer\nEND TYPE\nTYPE B\n  y AS Integer\nEND TYPE\nUNION AB\n  A\n  B\nEND UNION\nFUNC pick AS AB\n  RETURN A[1]\nEND FUNC\nFUNC main AS Integer\n  LET v AS AB = pick()\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- non-comparable list element (union) rejected in contains ----------

    #[test]
    fn contains_on_union_list_walks_noncomparable() {
        let src = "IMPORT collections\nTYPE A\n  x AS Integer\nEND TYPE\nTYPE B\n  y AS Integer\nEND TYPE\nUNION AB\n  A\n  B\nEND UNION\nFUNC main AS Integer\n  LET xs AS List OF AB = [A[1]]\n  LET b = collections::contains(xs, A[1])\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- close-op argument mode (call_argument_mode Transfer arm) ----------

    #[test]
    fn close_op_consumes_resource() {
        // fs::close is the registered close op for File; calling it consumes the
        // handle (call_argument_mode Transfer arm).
        assert!(accepts(
            "IMPORT fs\nFUNC main AS Integer\n  RES f AS File = fs::openFile(\"x\")\n  fs::close(f)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- expression_compatible via default parameter values ----------------

    #[test]
    fn default_byte_from_int_literal() {
        // Byte param with an in-range Integer-literal default (Byte/Integer/Number).
        assert!(accepts(
            "FUNC g(a AS Byte = 200) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g()\nEND FUNC\n"
        ));
    }

    #[test]
    fn default_fixed_from_int_literal() {
        assert!(accepts(
            "FUNC g(a AS Fixed = 5) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g()\nEND FUNC\n"
        ));
    }

    #[test]
    fn default_fixed_from_negative_literal() {
        // Fixed param with a negated numeric literal default (Unary '-' arm).
        assert!(accepts(
            "FUNC g(a AS Fixed = -5) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g()\nEND FUNC\n"
        ));
    }

    #[test]
    fn default_list_of_fixed_literal() {
        // List-literal default numeric-widening arm.
        assert!(accepts(
            "FUNC g(a AS List OF Fixed = [1, 2]) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g()\nEND FUNC\n"
        ));
    }

    // ---- compatible_optional (thread resource plane both present) ----------

    #[test]
    fn thread_resource_plane_optional_compat() {
        // A worker whose declared and inferred thread types both carry a resource
        // plane exercises compatible_optional Some/Some.
        assert!(check_project_dir(std::path::Path::new(&format!(
            "{}/tests/rt-behavior/threads/func_thread_transfer_valid",
            env!("CARGO_MANIFEST_DIR")
        )))
        .is_empty());
    }
}
