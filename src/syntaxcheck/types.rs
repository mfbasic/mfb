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
            if let Some((key, value)) = split_map_body(rest) {
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
                if expected_name == actual_name {
                    return true;
                }
                let expected_bare = expected_name.rsplit('.').next().unwrap_or(expected_name);
                let actual_bare = actual_name.rsplit('.').next().unwrap_or(actual_name);
                let expected_info = self
                    .type_infos
                    .get(expected_name)
                    .or_else(|| self.type_infos.get(expected_bare));
                // A union accepts any of its variant values (a variant value fits
                // its union slot).
                if expected_info.is_some_and(|info| {
                    matches!(info.kind, TypeDeclKind::Union)
                        && info
                            .variants
                            .iter()
                            .any(|variant| variant.name == *actual_bare)
                }) {
                    return true;
                }
                if expected_bare != actual_bare {
                    return false;
                }
                // The bare names coincide. An imported package's types are
                // registered under their bare name (`Db`), while a qualified
                // reference written by the importer resolves to `binding.Db`
                // (plan-link-update.md §5a) — so a qualified name must equate to
                // its bare form. But two genuinely distinct declarations that
                // merely share a final path segment (an imported `geo.Point` and a
                // local `Point` with different fields) must NOT unify (bug-41):
                // only unify when both names resolve to the *same* registered
                // `TypeInfo`. When either side is unregistered — a built-in `User`
                // type such as `net.Url`, or a template parameter — the shared bare
                // name is authoritative.
                let actual_info = self
                    .type_infos
                    .get(actual_name)
                    .or_else(|| self.type_infos.get(actual_bare));
                match (expected_info, actual_info) {
                    (Some(expected_info), Some(actual_info)) => {
                        std::ptr::eq(expected_info, actual_info)
                    }
                    _ => true,
                }
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

/// Whether a `Map`/`MapEntry`/`Thread`/`ThreadWorker` `OF`-construct — each of
/// which owns exactly one top-level ` TO ` — begins at byte `at` of `body`. The
/// match must sit on a word boundary so a user template whose name merely ends
/// in `Map` (`MyMap OF T`, which owns no ` TO `) is not counted.
fn owns_a_to_separator(body: &str, at: usize) -> bool {
    let bytes = body.as_bytes();
    if at > 0 {
        let prev = bytes[at - 1];
        // An identifier-continue byte (or any UTF-8 continuation/lead byte of a
        // multi-byte identifier char) before the keyword means we are mid-word.
        if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.' || prev == b':' || prev >= 0x80
        {
            return false;
        }
    }
    // `List OF`/`Result OF`/user templates own no ` TO ` and are excluded. Each
    // keyword is checked with its trailing ` OF ` so `MapEntry OF` is not seen as
    // `Map OF` and `ThreadWorker OF` is not seen as `Thread OF`.
    ["MapEntry OF ", "ThreadWorker OF ", "Map OF ", "Thread OF "]
        .iter()
        .any(|keyword| body[at..].starts_with(keyword))
}

/// Split a `Map OF` body `K TO V` (the text after the outer `Map OF ` prefix) on
/// the ` TO ` that separates the outer key from its value. A leftmost
/// `split_once(" TO ")` mis-parses a key that itself carries a ` TO `
/// (`Map OF Map OF String TO Integer TO Boolean`, bug-41): this scan skips the
/// ` TO ` owned by each nested `Map`/`MapEntry`/`Thread`/`ThreadWorker` sub-type
/// and ignores separators inside parenthesized / `FUNC(...)` groups. Returns
/// `None` when there is no top-level ` TO ` (a malformed body), so the caller
/// falls through and the whole string is treated as a plain type name.
fn split_map_body(body: &str) -> Option<(&str, &str)> {
    let bytes = body.as_bytes();
    let mut depth: usize = 0;
    // Nested `OF`-constructs seen at depth 0 whose ` TO ` has not yet appeared.
    let mut pending: usize = 0;
    let mut index = 0;
    while index < body.len() {
        match bytes[index] {
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            _ if depth == 0 && body[index..].starts_with(" TO ") => {
                if pending > 0 {
                    pending -= 1;
                    index += 4;
                } else {
                    return Some((&body[..index], &body[index + 4..]));
                }
            }
            _ if depth == 0 && owns_a_to_separator(body, index) => {
                pending += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
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

    // ---- bug-41 (3): radix/separator Byte-literal RECOVER range check -------

    fn byte_recover_src(literal: &str) -> String {
        // An inline-TRAP RECOVER against a `Byte` success type is the surviving
        // consumer of `expression_compatible`'s Byte arm (checking.rs:320).
        format!(
            "FUNC parseByte(v AS Integer) AS Byte\n  IF v < 0 THEN FAIL error(404, \"neg\")\n  RETURN toByte(v)\nEND FUNC\nFUNC main AS Integer\n  LET b AS Byte = parseByte(-1) TRAP(e)\n    RECOVER {literal}\n  END TRAP\n  RETURN 0\nEND FUNC\n"
        )
    }

    #[test]
    fn byte_recover_accepts_radix_and_separator_literals() {
        // The lexer canonicalizes radix/separator literals to decimal before the
        // Byte range check (`0xFF`->`255`, `2_00`->`200`), so an in-range Byte is
        // accepted — not spuriously rejected with TYPE_RECOVER_TYPE_MISMATCH
        // (bug-41 (3)). Decimal `200` is the pre-existing baseline.
        for literal in ["200", "0xFF", "0b1111_1111", "2_00"] {
            assert!(
                !check_src(&byte_recover_src(literal))
                    .iter()
                    .any(|rule| rule == "TYPE_RECOVER_TYPE_MISMATCH"),
                "RECOVER {literal} against a Byte type should be accepted"
            );
        }
    }

    #[test]
    fn byte_recover_rejects_out_of_range_radix_literal() {
        // `0x100` == 256 is out of Byte range and must still be rejected.
        assert!(check_src(&byte_recover_src("0x100"))
            .iter()
            .any(|rule| rule == "TYPE_RECOVER_TYPE_MISMATCH"));
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

    // ---- bug-41 (2): nesting-aware `Map OF K TO V` split -------------------

    #[test]
    fn split_map_body_handles_nested_key_and_value() {
        use super::split_map_body;
        // Simple body: leftmost and balanced agree.
        assert_eq!(split_map_body("String TO Integer"), Some(("String", "Integer")));
        // Nested KEY carries its own ` TO `: the key is the whole inner map, the
        // value is `Boolean` (bug-41 — leftmost split gave `Map OF Map OF String`).
        assert_eq!(
            split_map_body("Map OF String TO Integer TO Boolean"),
            Some(("Map OF String TO Integer", "Boolean"))
        );
        // Nested VALUE map (already correct under leftmost split) still parses.
        assert_eq!(
            split_map_body("String TO Map OF Integer TO Boolean"),
            Some(("String", "Map OF Integer TO Boolean"))
        );
        // A `FUNC(...) AS R` key is kept whole (parens/`AS` carry no top-level TO).
        assert_eq!(
            split_map_body("FUNC(Integer) AS Boolean TO Integer"),
            Some(("FUNC(Integer) AS Boolean", "Integer"))
        );
        // A parenthesized nested-map key round-trips (the caller strips the group).
        assert_eq!(
            split_map_body("(Map OF String TO Integer) TO Boolean"),
            Some(("(Map OF String TO Integer)", "Boolean"))
        );
        // A RES-marked value stays attached (the caller's element parser strips it).
        assert_eq!(
            split_map_body("String TO RES File"),
            Some(("String", "RES File"))
        );
        // No top-level ` TO ` at all → None (caller falls through to a type name).
        assert_eq!(split_map_body("Integer"), None);
    }

    #[test]
    fn parse_type_nested_map_key_structure() {
        use super::{SyntaxChecker, Type};
        let dir = std::path::Path::new(".");
        let project = crate::ast::AstProject {
            name: "t".to_string(),
            files: vec![],
        };
        let checker = SyntaxChecker::new(dir, &project);
        // `Map OF Map OF String TO Integer TO Boolean` must build
        // `Map(Map(String, Integer), Boolean)`, not the mis-split
        // `Map(User("Map OF Map OF String"), …)`.
        let Type::Map(key, value) = checker.parse_type("Map OF Map OF String TO Integer TO Boolean")
        else {
            panic!("expected a Map type");
        };
        assert!(matches!(*value, Type::Boolean));
        let Type::Map(inner_key, inner_value) = *key else {
            panic!("expected the key to be a nested Map");
        };
        assert!(matches!(*inner_key, Type::String));
        assert!(matches!(*inner_value, Type::Integer));
    }

    // ---- bug-41 (1): bare-name User unification needs same declaration -----

    #[test]
    fn bare_name_user_types_need_same_declaration() {
        use super::{FieldInfo, SyntaxChecker, Type, TypeInfo};
        use crate::ast::{TypeDeclKind, Visibility};
        let dir = std::path::Path::new(".");
        let project = crate::ast::AstProject {
            name: "t".to_string(),
            files: vec![],
        };
        let mut checker = SyntaxChecker::new(dir, &project);
        let record = |field: &str| TypeInfo {
            kind: TypeDeclKind::Type,
            visibility: Visibility::Export,
            file_path: String::new(),
            fields: vec![FieldInfo {
                name: field.to_string(),
                type_: Type::Integer,
                visibility: Visibility::Public,
            }],
            variants: Vec::new(),
            members: std::collections::HashSet::new(),
        };
        // Two genuinely distinct declarations that share the final segment `Point`.
        checker
            .type_infos
            .insert("geo.Point".to_string(), record("lat"));
        checker.type_infos.insert("Point".to_string(), record("x"));
        // bug-41: distinct declarations must NOT unify on the shared bare name.
        assert!(!checker.compatible(
            &Type::User("geo.Point".to_string()),
            &Type::User("Point".to_string())
        ));
        // The legitimate qualified==bare case (both resolve to the same registered
        // `TypeInfo`) still unifies: a qualified alias of the bare `Point`.
        assert!(checker.compatible(
            &Type::User("mod.Point".to_string()),
            &Type::User("Point".to_string())
        ));
    }
}
