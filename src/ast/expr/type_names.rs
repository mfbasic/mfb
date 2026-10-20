//! plan-176-M: the parser's type-name grammar — qualified and package-qualified names, type
//! names with template arguments, thread/resource-plane/function types. The expression grammar
//! lives in `ast/expr/mod.rs`.

use super::*;

impl<'a> FileParser<'a> {
    /// Normalize a package-qualified built-in name to the spelling the rest of
    /// the compiler declares it under.
    ///
    /// A package-qualified built-in *value* type (`net::Url`, `http::Response`,
    /// `net::PingStatus`, `json::JsonBool`) becomes its bare internal id, so
    /// every downstream stage sees one spelling (plan-03-http.md §A.1/§B.2). A
    /// package-qualified **resource** (`process::Process`) instead KEEPS its
    /// qualified identity — resources are package-scoped so a user
    /// `TYPE Process` no longer collides (plan-97 / bug-441).
    ///
    /// This is the single decision procedure for that rewrite. It was inlined in
    /// `parse_type_base_name` alone, which is why the qualified spelling
    /// resolved in a type ANNOTATION and nowhere else; bug-480 Defect B1 shares
    /// it with the two expression positions — a qualified identifier (the head
    /// of `net::PingStatus.Ok`) and a union `CASE` pattern
    /// (`CASE json::JsonBool(b)`). A name the registry does not resolve as a
    /// builtin type — a function, a constant, or a user package's type — is
    /// returned unchanged.
    ///
    /// The qualifier is the import BINDING, so `IMPORT net AS n` makes
    /// `n::PingStatus` mean the same type as `net::PingStatus`; the binding is
    /// resolved to its package before the registry is asked.
    pub(in crate::ast) fn normalize_qualified_builtin_type(&self, qualified: String) -> String {
        // Fast path: a name with no qualifier can never be a package-qualified
        // type, and both registry probes below intern a `Symbol` from the name
        // before they can say so. This runs on every identifier expression in
        // every program, so the bare case must not pay for that.
        //
        // The one exception is a built-in package's own injected companion, where
        // a BARE name may still be one of that package's value types: its members
        // are local to it, so the rule says it writes them without a prefix. The
        // declared identity is package-qualified all the same (bug-480 Phase 4b),
        // so the prefix is supplied here rather than hand-written into every
        // `builtins/<pkg>.mfb`.
        let Some((binding, leaf)) = qualified.split_once('.') else {
            return self.qualify_own_builtin_type(qualified);
        };
        // `binding` is what the author wrote; the registry is keyed by package.
        // An unknown binding is left alone — it may be a LINK alias, an imported
        // user package, or simply undeclared, all of which are diagnosed later
        // by name resolution, which reports them far better than a parser can.
        let resolved = match self.import_bindings.get(binding) {
            Some(package) if package != binding => format!("{package}.{leaf}"),
            _ => qualified.clone(),
        };
        if crate::codegen::builtins::is_qualified_builtin_resource(&resolved) {
            return resolved;
        }
        crate::codegen::builtins::qualified_builtin_type(&resolved).unwrap_or(qualified)
    }

    /// The same normalization, in a position that is known to name a TYPE: a
    /// type annotation, a union variant, a `CASE` pattern, or a constructor
    /// head. It additionally canonicalizes an imported USER package's type.
    ///
    /// bug-632: an imported user type's identity is PACKAGE-QUALIFIED —
    /// `<package>.<Name>`, spelled with the package name, never the import
    /// binding — exactly as a built-in package's value type is (bug-480
    /// Phase 4b: `net.PingStatus`). `binding::A` therefore becomes `<package>.A`.
    /// This used to de-qualify to the bare leaf (`8f0ebfeb8`), which made the
    /// consumer's `ov::A` indistinguishable from its own `A` and from another
    /// package's `A` everywhere downstream. The imported package's declarations,
    /// signatures and IR are qualified to the same spelling where the `.mfp` is
    /// read, so both sides name one type.
    ///
    /// Built-in packages are answered first by `normalize_qualified_builtin_type`
    /// and never reach the user-package arm. A name under a built-in binding that
    /// the registry did not resolve is not one of that package's types at all,
    /// and stays as written so name resolution reports it.
    ///
    /// This is the type-position form deliberately: the plain qualified
    /// identifier `pkg::name` is far more often a FUNCTION or a constant, which
    /// keeps its binding spelling for call resolution.
    ///
    /// The rewrite is gated on the package ACTUALLY EXPORTING the leaf
    /// ([`package_exports_type`](Self::package_exports_type)). A name the package
    /// does not export stays as written, so `resolve_package_qualified_name`
    /// reports it against the package (`tests/syntax/packages/package-unknown-member-invalid`).
    pub(in crate::ast) fn normalize_qualified_type_name(&self, qualified: String) -> String {
        let normalized = self.normalize_qualified_builtin_type(qualified);
        let Some((binding, leaf)) = normalized.split_once('.') else {
            return normalized;
        };
        let package = self
            .import_bindings
            .get(binding)
            .map(String::as_str)
            .unwrap_or(binding);
        if self.import_bindings.contains_key(binding)
            && !crate::codegen::builtins::is_builtin_import(package)
            && self.package_exports_type(package, leaf)
        {
            return format!("{package}.{leaf}");
        }
        normalized
    }

    /// Whether `package` exports `leaf` as a TYPE — a record, union, enum, union
    /// variant, or resource type — according to the dependency `.mfp` index the
    /// project parse path supplied (`package_type_names`).
    ///
    /// `leaf` is matched on its HEAD segment, because a type position may carry a
    /// trailing selection (`pkg::Color.Red`): the package owns `Color`, and what
    /// follows is resolved against it downstream.
    ///
    /// An absent package answers `false`. That is the direction that cannot
    /// invent a resolution: with no index (every entry point but the project
    /// parse) or an unreadable dependency, the qualified spelling survives and
    /// name resolution reports it — the behaviour before the type-position
    /// normalizer existed.
    fn package_exports_type(&self, package: &str, leaf: &str) -> bool {
        let head = leaf.split('.').next().unwrap_or(leaf);
        self.package_type_names
            .get(package)
            .is_some_and(|names| names.contains(head))
    }

    /// Inside `builtins/<pkg>.mfb`, qualify a BARE name that `<pkg>` declares as
    /// one of its own value types; leave everything else alone.
    ///
    /// This is what lets the injected companion keep writing its own types the
    /// way the governing rule says a local name is written — unprefixed — while
    /// the namespace it lands in is package-scoped. User source gets no such
    /// treatment: a bare imported type there is exactly what Phase 4b refuses.
    pub(in crate::ast) fn qualify_own_builtin_type(&self, name: String) -> String {
        if self.builtin_package.is_none() {
            return name;
        }
        let resolves = |candidate: &str| {
            crate::codegen::builtins::is_qualified_builtin_resource(candidate)
                || crate::codegen::builtins::qualified_builtin_type(candidate).is_some()
        };
        // The owning package first: its own members are what an injected companion
        // names most, and it must win over an import when both declare the leaf.
        if let Some(package) = self.builtin_package.as_deref() {
            let candidate = format!("{package}.{name}");
            if resolves(&candidate) {
                return candidate;
            }
        }
        // Then the packages this file IMPORTS. A cross-package helper chunk -- the
        // `term`<->`astrings` bridge is the one in tree -- is authored as `term`'s
        // source but names `astrings`' types, and names them bare because that is how
        // the rule reads inside a package that imports them. Resolving through the
        // import list qualifies those without hand-editing the chunk.
        let mut candidates: Vec<String> = self
            .import_bindings
            .values()
            .map(|package| format!("{package}.{name}"))
            .filter(|candidate| resolves(candidate))
            .collect();
        candidates.sort();
        candidates.dedup();
        // Exactly one owner, or nothing: an ambiguous leaf is left bare rather than
        // guessed at.
        match candidates.as_slice() {
            [only] => only.clone(),
            _ => name,
        }
    }

    pub(in crate::ast) fn parse_qualified_name(&mut self, detail: &str) -> Option<String> {
        let name = self.consume_identifier(detail)?;
        self.finish_qualified_name(name)
    }

    pub(in crate::ast) fn finish_qualified_name(&mut self, mut name: String) -> Option<String> {
        if self.match_kind(TokenKind::DoubleColon) {
            let part = self.consume_qualified_identifier_part()?;
            name.push('.');
            name.push_str(&part);
        }
        while self.match_kind(TokenKind::DoubleColon) {
            let token = self.previous().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Package-qualified names must have exactly two parts.",
                &token,
            );
            self.consume_qualified_identifier_part()?;
        }
        Some(name)
    }

    /// Enter one type-annotation-nesting level, reporting and returning `false`
    /// when the maximum depth is exceeded. On the `false` path the counter is
    /// already rewound (the caller must simply bail); otherwise the caller must
    /// pair a successful `enter_type` with exactly one `leave_type`.
    fn enter_type(&mut self) -> bool {
        self.type_depth += 1;
        if self.type_depth > MAX_TYPE_DEPTH {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Type annotation nesting is too deep.",
                &token,
            );
            // Latch after the one diagnostic, then collapse the cursor to `Eof`
            // (same recovery as the statement-depth guard, bug-183): the enclosing
            // type/statement parses unwind without recursing further or emitting a
            // trailing cascade for the same pathological annotation.
            self.depth_exceeded = true;
            self.seek_to_end();
            self.type_depth -= 1;
            false
        } else {
            true
        }
    }

    fn leave_type(&mut self) {
        self.type_depth -= 1;
    }

    /// Guarded entry point for type-name parsing. Every recursive sub-parse
    /// re-enters through here, so the depth guard bounds native recursion across
    /// grouped types, element types, template args, and function types (bug-191).
    pub(in crate::ast) fn parse_type_name(&mut self) -> Option<String> {
        if !self.enter_type() {
            return None;
        }
        let result = self.parse_type_name_inner();
        self.leave_type();
        result
    }

    fn parse_type_name_inner(&mut self) -> Option<String> {
        if self.match_keyword(Keyword::Func) {
            return self.parse_function_type_name(false);
        }
        if self.match_keyword(Keyword::Isolated) {
            if self.consume_keyword(Keyword::Func, "ISOLATED type must be followed by FUNC.") {
                return self.parse_function_type_name(true);
            }
            return None;
        }
        if self.match_kind(TokenKind::LParen) {
            let name = self.parse_type_name()?;
            self.consume_kind(TokenKind::RParen, "Expected `)` after grouped type.");
            return Some(format!("({name})"));
        }
        let mut name = self.parse_type_base_name("Expected a type name.")?;
        if self.check_identifier_ci("OF") {
            self.advance();
            if name.eq_ignore_ascii_case("Thread") || name.eq_ignore_ascii_case("ThreadWorker") {
                return self.parse_thread_type_name(name);
            }

            if name.eq_ignore_ascii_case("Map") || name.eq_ignore_ascii_case("MapEntry") {
                let first = self.parse_type_name()?;
                if !self.check_identifier_ci("TO") && !self.check_keyword(Keyword::To) {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        "Expected `TO` in map type.",
                        &token,
                    );
                    return None;
                }
                self.advance();
                // A `RES` value marks a resource-transfer collection
                // (`Map OF K TO RES File`, §15.6): the value is a resource pointer
                // whose scope-ownership transfers across a function boundary.
                let value_res = self.match_keyword(Keyword::Res);
                let second = self.parse_type_name()?;
                // A stateful resource-union value carries a uniform `STATE T`
                // clause on the element (`Map OF K TO RES Stream STATE S`,
                // bug-427), folded into the value type string exactly as the
                // thread resource plane does (`parse_resource_plane_type`).
                let second = self.parse_optional_element_state(second, value_res)?;
                name.push_str(" OF ");
                name.push_str(&first);
                name.push_str(" TO ");
                if value_res {
                    name.push_str("RES ");
                }
                name.push_str(&second);
                return Some(name);
            }

            if name.eq_ignore_ascii_case("List") {
                // `List OF RES File` (§15.6): a resource-transfer list whose
                // element is a pointer whose scope-ownership transfers across a
                // function boundary.
                let element_res = self.match_keyword(Keyword::Res);
                let arg = self.parse_type_name()?;
                // A stateful resource-union element carries a uniform `STATE T`
                // clause (`List OF RES Stream STATE S`, bug-427), folded into the
                // element type string exactly as the thread resource plane does
                // (`parse_resource_plane_type`).
                let arg = self.parse_optional_element_state(arg, element_res)?;
                name.push_str(" OF ");
                if element_res {
                    name.push_str("RES ");
                }
                name.push_str(&arg);
                return Some(name);
            }

            if name.eq_ignore_ascii_case("Set") {
                // `Set OF T` (plan-63, §4.7): a single comparable element type —
                // no `TO` (unlike `Map`), and no `RES` marker (unlike `List`), since
                // a resource handle is not comparable and can never be a Set element.
                if self.check_keyword(Keyword::Res) {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        "A Set element cannot be a resource; `RES` is not allowed after `Set OF`.",
                        &token,
                    );
                    return None;
                }
                let arg = self.parse_type_name()?;
                name.push_str(" OF ");
                name.push_str(&arg);
                return Some(name);
            }

            // bug-833: a template's argument list is ONE type, or a parenthesized
            // list `Name OF (A, B)` (owner decision 2026-10-05). A bare argument
            // never reads past a comma, so the comma belongs to whatever list
            // encloses the type (a parameter list, a function type, another
            // template's arguments) and `FUNC push OF T(s AS Stack OF T, v AS T)`
            // parses. One parenthesized argument is a grouped type, as before.
            if self.match_kind(TokenKind::LParen) {
                self.type_list_depth += 1;
                let args = self.parse_template_argument_list();
                self.type_list_depth -= 1;
                let args = args?;
                if !self.consume_kind(
                    TokenKind::RParen,
                    "Expected `)` after the template type arguments.",
                ) {
                    return None;
                }
                name.push_str(" OF (");
                name.push_str(&args.join(", "));
                name.push(')');
                return Some(name);
            }
            let arg = self.parse_type_name()?;
            // Outside every comma-separated list a comma cannot follow a type, so
            // one here is the old bare multi-argument spelling: name the form
            // that replaced it, at the comma.
            if self.type_list_depth == 0 && self.check_kind(&TokenKind::Comma) {
                let token = self.peek().clone();
                let mut args = vec![arg];
                while self.match_kind(TokenKind::Comma) {
                    let Some(next) = self.parse_type_name() else {
                        break;
                    };
                    args.push(next);
                }
                self.report(
                    "MFB_PARSE_UNEXPECTED_TOKEN",
                    &format!(
                        "A template with more than one type argument takes a parenthesized list: write `{} OF ({})`.",
                        name.replace('.', "::"),
                        args.join(", ").replace('.', "::")
                    ),
                    &token,
                );
                return None;
            }
            name.push_str(" OF ");
            name.push_str(&arg);
        }
        Some(name)
    }

    /// The types of a parenthesized template argument list, after its `(`:
    /// `Type {, Type}` (bug-833). The closing `)` is the caller's.
    fn parse_template_argument_list(&mut self) -> Option<Vec<String>> {
        let mut args = vec![self.parse_type_name()?];
        while self.match_kind(TokenKind::Comma) {
            args.push(self.parse_type_name()?);
        }
        Some(args)
    }

    /// Parse a thread type body after `<kind> OF`, supporting the optional
    /// resource plane: `Thread OF Msg TO Out`, `Thread OF Msg RES Res TO Out`,
    /// or the resource-only `Thread OF RES Res TO Out` (message defaults to
    /// `Nothing`). `kind` is the leading `Thread`/`ThreadWorker` token.
    pub(in crate::ast) fn parse_thread_type_name(&mut self, kind: String) -> Option<String> {
        let canonical = if kind.eq_ignore_ascii_case("ThreadWorker") {
            "ThreadWorker"
        } else {
            "Thread"
        };

        let mut message: Option<String> = None;
        let mut resource: Option<String> = None;

        if self.match_keyword(Keyword::Res) {
            resource = Some(self.parse_resource_plane_type()?);
        } else {
            message = Some(self.parse_type_name()?);
            if self.match_keyword(Keyword::Res) {
                resource = Some(self.parse_resource_plane_type()?);
            }
        }

        if !self.check_identifier_ci("TO") && !self.check_keyword(Keyword::To) {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Expected `TO` in thread type.",
                &token,
            );
            return None;
        }
        self.advance();
        let output = self.parse_type_name()?;

        let message = message.unwrap_or_else(|| "Nothing".to_string());
        Some(match resource {
            Some(resource) if message == "Nothing" => {
                format!("{canonical} OF RES {resource} TO {output}")
            }
            Some(resource) => format!("{canonical} OF {message} RES {resource} TO {output}"),
            None => format!("{canonical} OF {message} TO {output}"),
        })
    }

    /// Parse a thread plane's `RES` element: the resource type plus an optional
    /// ` STATE T` clause (plan-54), folded into one type string
    /// (`File STATE Cursor`) so the plane names the transferred resource's state
    /// and `thread::transfer`/`accept` can check it (closes bug-257). A bare
    /// element (no `STATE`) is unchanged.
    fn parse_resource_plane_type(&mut self) -> Option<String> {
        let resource = self.parse_type_name()?;
        match self.parse_optional_state() {
            Some(state) => Some(format!("{resource} STATE {state}")),
            None => Some(resource),
        }
    }

    /// Fold an optional `STATE T` clause into a collection element/value type
    /// (`RES Stream STATE PendingState`, bug-427), mirroring the thread resource
    /// plane's `parse_resource_plane_type`. A stateful resource union may name a
    /// uniform STATE type across its variants, and a `List`/`Map` element must be
    /// able to carry it so an extracted element can read `.state`.
    ///
    /// A `STATE` clause is only meaningful on a `RES` element (a resource). After
    /// a non-`RES` element it is rejected with a clear parse diagnostic rather
    /// than being left as a dangling token, so a bare (no-`RES`) resource element
    /// still surfaces `TYPE_RESOURCE_REQUIRES_RES` at type checking. When no
    /// `STATE` clause follows, the element string is returned unchanged.
    fn parse_optional_element_state(
        &mut self,
        element: String,
        element_res: bool,
    ) -> Option<String> {
        if !self.check_identifier_ci("STATE") {
            return Some(element);
        }
        if !element_res {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "A `STATE` clause requires a `RES` collection element; a \
                 non-resource element cannot carry state.",
                &token,
            );
            return None;
        }
        let state = self.parse_optional_state()?;
        Some(format!("{element} STATE {state}"))
    }

    pub(in crate::ast) fn parse_function_type_name(&mut self, isolated: bool) -> Option<String> {
        if !self.consume_kind(TokenKind::LParen, "Function type must include `(`.") {
            return None;
        }
        let mut params = Vec::new();
        if !self.check_kind(&TokenKind::RParen) {
            self.type_list_depth += 1;
            let listed = self.parse_function_type_params(&mut params);
            self.type_list_depth -= 1;
            listed?;
        }
        if !self.consume_kind(TokenKind::RParen, "Function type must close with `)`.") {
            return None;
        }
        if !self.consume_keyword(Keyword::As, "Function type must include `AS`.") {
            return None;
        }
        let returns = self.parse_type_name()?;
        Some(format!(
            "{}FUNC({}) AS {}",
            if isolated { "ISOLATED " } else { "" },
            params.join(", "),
            returns
        ))
    }

    /// A function type's comma-separated parameter types, up to (not including)
    /// its `)`; a trailing comma is allowed (bug-821).
    fn parse_function_type_params(&mut self, params: &mut Vec<String>) -> Option<()> {
        loop {
            params.push(self.parse_type_name()?);
            if !self.match_kind(TokenKind::Comma) {
                return Some(());
            }
            if self.check_kind(&TokenKind::RParen) {
                return Some(());
            }
        }
    }

    pub(in crate::ast) fn parse_type_base_name(&mut self, detail: &str) -> Option<String> {
        let name = match self.peek().kind.clone() {
            TokenKind::Identifier(value) => {
                self.advance();
                value
            }
            TokenKind::Keyword(Keyword::Nothing) => {
                self.advance();
                "Nothing".to_string()
            }
            _ => {
                let token = self.peek().clone();
                self.report("MFB_PARSE_INVALID_IDENTIFIER", detail, &token);
                return None;
            }
        };
        let qualified = self.finish_qualified_name(name)?;
        Some(self.normalize_qualified_type_name(qualified))
    }
}
