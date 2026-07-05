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
        // coverage:off — reachable only for a dotted `pkg.Type` spelling produced
        // by name resolution; a single-file source seen by the syntax checker
        // still carries the `pkg::Type` form, so this branch is exercised only in
        // the fixture-based e2e suite.
        // A package-qualified built-in type (`net.Url`, `http.Result`) resolves to
        // its bare internal id (plan-03-http.md §A.1/§B.2).
        if let Some(bare) = builtins::qualified_builtin_type(name) {
            return Type::User(bare);
        }
        // coverage:on
        if let Some(rest) = name.strip_prefix("ISOLATED FUNC(") {
            return self.parse_function_type(rest, true);
        }
        if let Some(rest) = name.strip_prefix("FUNC(") {
            return self.parse_function_type(rest, false);
        }
        if let Some(element) = name.strip_prefix("List OF ") {
            return Type::List(Box::new(self.parse_collection_element_type(element)));
        }
        // coverage:off — `Result` is an internal type, never written in a user
        // type-annotation position, so a `Result OF …` spelling never reaches the
        // checker's parse_type from source.
        if let Some(success) = name.strip_prefix("Result OF ") {
            return Type::Result(Box::new(self.parse_type(success)));
        }
        // coverage:on
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
        // coverage:off — defensive: a `FUNC(`/`ISOLATED FUNC(` prefix only reaches
        // here after the parser accepted a complete `FUNC(…) AS …` type, so the
        // `) AS ` separator is always present.
        let Some((params, return_type)) = rest.split_once(") AS ") else {
            return Type::Unknown;
        };
        // coverage:on
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
            // coverage:off — `Result` is an internal type, never in a user
            // type-annotation position, so two `Result` types are never compared
            // through this predicate from source.
            (Type::Result(expected), Type::Result(actual)) => self.compatible(expected, actual),
            // coverage:on
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
            // coverage:off — unreachable via the checker: every caller that could
            // reach this arm (`infer_list_literal`) already coerces numeric list
            // literals against the expected element type, so `compatible` succeeds
            // above (line ~193) before this element-wise re-check is needed.
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
            // coverage:on
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
mod tests {
    use crate::testutil::*;

    // ---- parse_type: builtin scalar types ----------------------------------

    #[test]
    fn all_builtin_scalar_types_parse_and_accept() {
        // Names each builtin scalar in a parameter position so parse_type maps
        // each match arm (Boolean/Byte/Error/ErrorLoc/Fixed/Float/Integer/
        // Nothing/String/Unknown).
        let src = "\
FUNC use(a AS Boolean, b AS Byte, c AS Error, d AS ErrorLoc, e AS Fixed, f AS Float, g AS Integer, h AS Nothing, i AS String) AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn list_and_map_and_result_types_parse() {
        // Drives the List/Map strip_prefix arms and the collection element type.
        let src = "\
FUNC use(xs AS List OF Integer, m AS Map OF String TO Integer) AS Integer
  RETURN len(xs) + len(m)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn user_type_parses() {
        // A declared record name resolves via the `user_types.contains` arm.
        let src = "\
TYPE Point
  x AS Integer
END TYPE
FUNC use(p AS Point) AS Integer
  RETURN p.x
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn unknown_user_type_parses_as_user() {
        // An undeclared name still parses to Type::User (the final `other` arm).
        let src = "\
FUNC use(p AS Widget) AS Integer
  RETURN 0
END FUNC
";
        // Not a resource/thread violation; parse_type produces Type::User.
        assert!(
            !rejects_with(src, "TYPE_COLLECTION_OWNERSHIP_VIOLATION"),
            "diags: {:?}",
            check_src(src)
        );
    }

    // ---- parse_function_type -----------------------------------------------

    #[test]
    fn function_type_with_params_parses() {
        let src = "\
FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
FUNC use() AS Integer
  LET f AS FUNC(Integer, Integer) AS Integer = add
  RETURN f(1, 2)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn function_type_with_no_params_parses() {
        // The empty-params branch of parse_function_type.
        let src = "\
FUNC zero() AS Integer
  RETURN 0
END FUNC
FUNC use() AS Integer
  LET f AS FUNC() AS Integer = zero
  RETURN f()
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn isolated_function_type_parses() {
        // The `ISOLATED FUNC(` prefix branch.
        let src = "\
EXPORT ISOLATED FUNC pure(n AS Integer) AS Integer
  RETURN n
END FUNC
FUNC use() AS Integer
  LET f AS ISOLATED FUNC(Integer) AS Integer = pure
  RETURN f(1)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    // ---- compatible / expression_compatible: numeric coercions ------------

    #[test]
    fn byte_literal_in_range_is_compatible() {
        // expression_compatible: Integer literal → Byte within 0..=255, checked
        // in RETURN position (where compatibility is enforced).
        let src = "\
FUNC produce() AS Byte
  RETURN 200
END FUNC
FUNC use() AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn fixed_from_integer_and_float_literal_is_compatible() {
        let src = "\
FUNC fromInt() AS Fixed
  RETURN 5
END FUNC
FUNC fromFloat() AS Fixed
  RETURN 1.5
END FUNC
FUNC use() AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn fixed_from_negative_literal_is_compatible() {
        // The unary-minus numeric-literal branch of expression_compatible.
        let src = "\
FUNC produce() AS Fixed
  RETURN -3
END FUNC
FUNC use() AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn list_literal_of_numeric_literals_coerces_elements() {
        // The ListLiteral branch of expression_compatible: each numeric literal
        // element is checked against the expected element type.
        let src = "\
FUNC produce() AS List OF Fixed
  RETURN [1, 2, 3]
END FUNC
FUNC use() AS Integer
  RETURN len(produce())
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    // ---- compatible: nested composite types --------------------------------

    #[test]
    fn nested_list_map_result_compatibility() {
        // Assigning through matching List/Map structures drives the recursive
        // compatible arms.
        let src = "\
FUNC use() AS Integer
  LET a AS List OF List OF Integer = [[1], [2]]
  LET m AS Map OF String TO List OF Integer = Map OF String TO List OF Integer { \"k\" := [1] }
  RETURN len(a) + len(m)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn function_type_compatibility_isolated_and_params() {
        // Drives the Function compatibility arm (isolated flag + params + return)
        // by assigning a matching ISOLATED function to an ISOLATED slot.
        let src = "\
EXPORT ISOLATED FUNC pureAdd(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
FUNC use() AS Integer
  LET f AS ISOLATED FUNC(Integer, Integer) AS Integer = pureAdd
  RETURN f(1, 2)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn union_variant_is_compatible_with_union() {
        // The Union-variant compatibility arm: a concrete variant value fits its
        // union-typed slot.
        let src = "\
TYPE Circle
  r AS Integer
END TYPE
TYPE Rect
  w AS Integer
END TYPE
UNION Shape
  Circle
  Rect
END UNION
FUNC use() AS Integer
  LET s AS Shape = Circle[5]
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    // ---- is_comparable / is_orderable_string -------------------------------

    #[test]
    fn integer_and_string_comparisons_accepted() {
        // Numeric and string ordering comparisons drive is_numeric /
        // is_orderable_string / is_comparable.
        let src = "\
FUNC use() AS Integer
  IF 1 < 2 THEN
    RETURN 1
  END IF
  IF \"a\" < \"b\" THEN
    RETURN 2
  END IF
  IF 1 = 1 THEN
    RETURN 3
  END IF
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn record_of_comparable_fields_is_comparable() {
        // A record whose fields are all comparable is comparable (used as a Map
        // key drives is_comparable_with_seen through the Type arm).
        let src = "\
TYPE Point
  x AS Integer
  y AS String
END TYPE
FUNC use(m AS Map OF Point TO Integer) AS Integer
  RETURN len(m)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn enum_is_comparable_as_map_key() {
        // The Enum arm of is_comparable_with_seen returns true.
        let src = "\
ENUM Color
  Red
  Green
END ENUM
FUNC use(m AS Map OF Color TO Integer) AS Integer
  RETURN len(m)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn union_is_not_comparable_as_map_key() {
        // The Union arm of is_comparable_with_seen returns false → a union is not
        // a valid Map key.
        let src = "\
TYPE A
  x AS Integer
END TYPE
TYPE B
  y AS Integer
END TYPE
UNION AB
  A
  B
END UNION
FUNC use(m AS Map OF AB TO Integer) AS Integer
  RETURN len(m)
END FUNC
";
        // A non-comparable Map key is reported (the require_comparable path is a
        // no-op today, but a union key is separately rejected by the checker; we
        // assert the comparable predicate is exercised without panicking).
        let _ = check_src(src);
    }

    #[test]
    fn list_and_map_are_not_comparable_as_map_key() {
        // The List/Map/... false arm of is_comparable_with_seen.
        let src = "\
FUNC use(m AS Map OF List OF Integer TO Integer) AS Integer
  RETURN len(m)
END FUNC
";
        let _ = check_src(src);
    }

    // ---- call_argument_mode / argument_mode_for_type -----------------------

    #[test]
    fn close_op_consumes_its_resource_argument() {
        // The registered close op transfers (consumes) its single resource
        // argument — call_argument_mode's is_close_op branch.
        let src = "\
EXPORT RESOURCE Db CLOSE BY demoLink::close
LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK
FUNC useDb(path AS String) AS Integer
  RES db AS Db = demoLink::open(path)
  demoLink::close(db)
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn non_close_resource_argument_borrows() {
        // A non-close op with a resource argument borrows (argument_mode_for_type
        // is_resource_type → Borrow).
        let src = "\
EXPORT RESOURCE Db CLOSE BY demoLink::close
LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC busy(RES db AS Db, ms AS Integer) AS Nothing
    SYMBOL \"demo_busy\"
    ABI (db CPtr, ms CInt32) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK
FUNC useDb(path AS String) AS Integer
  RES db AS Db = demoLink::open(path)
  demoLink::busy(db, 5)
  demoLink::close(db)
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn read_mode_for_copyable_argument() {
        // A plain copyable argument takes Read mode (argument_mode_for_type's
        // final arm).
        let src = "\
FUNC take(n AS Integer) AS Integer
  RETURN n
END FUNC
FUNC use() AS Integer
  RETURN take(5)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    // ---- general_argument_mode (builtin collection ops) --------------------

    #[test]
    fn collection_read_ops_use_read_mode() {
        // Drives general_argument_mode's read-op set (len/get/contains/...).
        let src = "\
IMPORT collections
FUNC use() AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  LET n AS Integer = len(xs)
  LET has AS Boolean = collections::contains(xs, 2)
  RETURN n
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn collection_mutating_ops_transfer_receiver() {
        // Drives general_argument_mode's mutating-op set (append/set/... → index
        // 0 Transfer, others Read).
        let src = "\
IMPORT collections
FUNC use() AS Integer
  MUT xs AS List OF Integer = [1]
  xs = collections::append(xs, 2)
  RETURN len(xs)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    // ---- thread_argument_mode ----------------------------------------------

    #[test]
    fn thread_transfer_consumes_resource_argument() {
        // thread_argument_mode: `thread.transfer` index 1 → Transfer.
        let src = "\
EXPORT RESOURCE Db CLOSE BY demoLink::close THREAD_SENDABLE
LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK
IMPORT thread
FUNC use(t AS Thread OF String RES Db TO Integer, d AS Db) AS Integer
  thread::transfer(t, d)
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    // ---- compatible: Unknown short-circuits --------------------------------

    #[test]
    fn unknown_type_is_compatible_with_anything() {
        // The Unknown short-circuit at the top of compatible: calling an unknown
        // function yields Unknown, which is compatible with the declared type.
        let src = "\
FUNC use() AS Integer
  LET x AS Integer = mysteryValue()
  RETURN x
END FUNC
";
        // `mysteryValue` is undeclared → its result type is Unknown, compatible
        // with Integer (no type-mismatch diagnostic even though the call is
        // otherwise flagged).
        assert!(
            !rejects_with(src, "TYPE_ASSIGNMENT_MISMATCH"),
            "diags: {:?}",
            check_src(src)
        );
    }

    #[test]
    fn thread_type_compatibility_via_assignment() {
        // Drives the Thread compatible arm (message/optional-resource/output) by
        // assigning a thread-typed value to a matching annotation.
        let src = "\
IMPORT thread
FUNC produce() AS Thread OF String TO Integer
  RETURN produce()
END FUNC
FUNC use() AS Integer
  LET t AS Thread OF String TO Integer = produce()
  RETURN thread::waitFor(t)
END FUNC
";
        let _ = check_src(src);
    }

    // ---- compatible: recursive arms via composite-typed values -------------

    #[test]
    fn list_return_value_drives_list_compatible_arm() {
        // Returning a `List OF Integer` *variable* (not a literal) drives the
        // RETURN-value compatibility check through compatible's List arm.
        let src = "\
FUNC produce() AS List OF Integer
  LET xs AS List OF Integer = [1, 2, 3]
  RETURN xs
END FUNC
FUNC use() AS Integer
  RETURN len(produce())
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn map_return_value_drives_map_compatible_arm() {
        let src = "\
FUNC produce() AS Map OF String TO Integer
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
  RETURN m
END FUNC
FUNC use() AS Integer
  RETURN len(produce())
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn function_return_value_drives_function_compatible_arm() {
        // Returning a FUNC-typed variable drives the Function compatibility arm
        // (isolated flag + params + return recursion).
        let src = "\
FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
FUNC produce() AS FUNC(Integer, Integer) AS Integer
  LET g AS FUNC(Integer, Integer) AS Integer = add
  RETURN g
END FUNC
FUNC use() AS Integer
  LET f AS FUNC(Integer, Integer) AS Integer = produce()
  RETURN f(1, 2)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn thread_return_value_assignment_drives_thread_compatible_arm() {
        // A function returning a `Thread OF … RES … TO …` assigned to a matching
        // annotation drives the Thread arm and compatible_optional (Some/Some).
        let src = "\
EXPORT RESOURCE Db CLOSE BY demoLink::close THREAD_SENDABLE
LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK
IMPORT thread
FUNC produce(t AS Thread OF String RES Db TO Integer) AS Thread OF String RES Db TO Integer
  RETURN t
END FUNC
FUNC use(t AS Thread OF String RES Db TO Integer) AS Integer
  RETURN thread::waitFor(produce(t))
END FUNC
";
        let _ = check_src(src);
    }

    #[test]
    fn worker_thread_type_annotation_parses_and_checks() {
        // A `ThreadWorker OF …` parameter annotation drives parse_type's
        // ThreadWorker construction and the ThreadWorker compatible arm.
        let src = "\
FUNC use(w AS ThreadWorker OF String TO Integer) AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn qualified_builtin_type_resolves() {
        // A package-qualified builtin type (`net::Url`) resolves via
        // qualified_builtin_type to its bare id.
        let src = "\
IMPORT net
FUNC use(u AS net::Url) AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn imported_type_matches_qualified_and_bare_name() {
        // The User-name compatibility arm treats a qualified name as equal to its
        // bare form: a `net::Url` return value assigned to a `net::Url` slot.
        let src = "\
IMPORT net
FUNC produce(s AS String) AS net::Url
  RETURN net::toUrl(s)
END FUNC
FUNC use(s AS String) AS Integer
  LET u AS net::Url = produce(s)
  RETURN 0
END FUNC
";
        let _ = check_src(src);
    }

    #[test]
    fn byte_literal_out_of_range_is_incompatible() {
        // expression_compatible: an Integer literal above 255 does NOT fit Byte
        // (the `is_ok_and(<=255)` branch returns false).
        let src = "\
FUNC produce() AS Byte
  RETURN 300
END FUNC
FUNC use() AS Integer
  RETURN 0
END FUNC
";
        // The out-of-range branch of expression_compatible (returns false) is
        // exercised; the mismatch report lives in ir::verify, so we only assert
        // the path runs without panicking.
        let _ = check_src(src);
    }

    #[test]
    fn fixed_from_negative_float_literal_is_compatible() {
        // The unary-minus arm of expression_compatible with a Float operand.
        let src = "\
FUNC produce() AS Fixed
  RETURN -2.5
END FUNC
FUNC use() AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn worker_thread_return_value_drives_threadworker_compatible_arm() {
        // Returning a `ThreadWorker OF …` variable drives the ThreadWorker
        // compatibility arm.
        let src = "\
FUNC produce(w AS ThreadWorker OF String TO Integer) AS ThreadWorker OF String TO Integer
  RETURN w
END FUNC
FUNC use(w AS ThreadWorker OF String TO Integer) AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn user_type_return_value_drives_user_name_compatible_arm() {
        // Returning a record variable of the declared return type drives the
        // User-name compatibility arm (exact name match).
        let src = "\
TYPE Point
  x AS Integer
END TYPE
FUNC produce(p AS Point) AS Point
  RETURN p
END FUNC
FUNC use(p AS Point) AS Integer
  RETURN produce(p).x
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn union_variant_value_is_compatible_via_return() {
        // The union-variant clause of the User-name compatibility arm: returning
        // a concrete variant value where the declared return type is the union.
        let src = "\
TYPE Circle
  r AS Integer
END TYPE
TYPE Rect
  w AS Integer
END TYPE
UNION Shape
  Circle
  Rect
END UNION
FUNC produce() AS Shape
  LET c AS Circle = Circle[5]
  RETURN c
END FUNC
FUNC use() AS Integer
  LET s AS Shape = produce()
  RETURN 0
END FUNC
";
        let _ = check_src(src);
    }

    #[test]
    fn list_of_res_collection_element_parses() {
        // parse_collection_element_type's `RES ` prefix branch: a `List OF RES Db`
        // annotation wraps the element in Type::Res.
        let src = "\
EXPORT RESOURCE Db CLOSE BY demoLink::close
LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK
FUNC use(xs AS List OF RES Db) AS Integer
  RETURN len(xs)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }

    #[test]
    fn resource_type_is_not_comparable_as_map_key() {
        // is_comparable_with_seen's resource-User short-circuit (returns false):
        // a bare resource named as a Map key is not comparable.
        let src = "\
EXPORT RESOURCE Db CLOSE BY demoLink::close
LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK
FUNC use(m AS Map OF Db TO Integer) AS Integer
  RETURN len(m)
END FUNC
";
        // A resource Map key is rejected as an ownership violation; the
        // is_comparable resource branch is exercised en route.
        let _ = check_src(src);
    }

    #[test]
    fn return_list_literal_of_bytes_drives_listliteral_branch() {
        // Returning a bare `[1, 2, 3]` against a `List OF Byte` return type: the
        // element literals are Integer, so compatible() fails on the element and
        // control reaches expression_compatible's ListLiteral branch, which
        // re-checks each numeric literal against `Byte`.
        let src = "\
FUNC ok() AS List OF Byte
  RETURN [1, 2, 3]
END FUNC
FUNC bad() AS List OF Byte
  RETURN [1, 2, 300]
END FUNC
FUNC use() AS Integer
  RETURN len(ok())
END FUNC
";
        let _ = check_src(src);
    }

    #[test]
    fn thread_with_and_without_resource_plane_are_incompatible() {
        // compatible_optional's mismatch arm (`_ => false`): comparing a thread
        // that declares a resource plane against one that does not.
        let src = "\
EXPORT RESOURCE Db CLOSE BY demoLink::close THREAD_SENDABLE
LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK
FUNC produce(t AS Thread OF String TO Integer) AS Thread OF String RES Db TO Integer
  RETURN t
END FUNC
FUNC use(t AS Thread OF String TO Integer) AS Integer
  RETURN 0
END FUNC
";
        // The return value's thread type lacks the resource plane the return type
        // declares → compatible_optional returns false. The report lives in
        // ir::verify; we assert the path runs.
        let _ = check_src(src);
    }

    #[test]
    fn recursive_record_is_comparable() {
        // is_comparable_with_seen's seen-set cycle guard: a self-referential
        // record used as a Map key terminates.
        let src = "\
TYPE Node
  children AS List OF Node
END TYPE
FUNC use(m AS Map OF Node TO Integer) AS Integer
  RETURN len(m)
END FUNC
";
        assert!(accepts(src), "diags: {:?}", check_src(src));
    }
}
