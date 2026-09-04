use super::pipeline::{node_count, placeholder_shape, substitute_placeholder};
use super::*;
use crate::operators::{BinaryOp, UnaryOp};

/// Maximum expression-nesting depth. Recursive-descent parsing turns each nesting
/// level into native stack frames, so an unbounded input (e.g. ~100k nested `(`,
/// or a long `NOT NOT …` / unary-minus / `^` chain) would overflow the stack with
/// a SIGSEGV before any diagnostic (bug-171 finding A). Matches the `MAX_DEPTH`
/// cap `ir::verify` uses for the same reason; no real source nests this deep.
const MAX_EXPR_DEPTH: usize = 256;

/// Maximum type-annotation-nesting depth. `parse_type_name` recurses for grouped
/// types, `Map`/`List`/`Result`/`Thread` element types, template args, and
/// function-type params/return; an unbounded nested type (e.g. `List OF List OF …`
/// or `(((…)))`) would otherwise overflow the native stack with no diagnostic
/// (bug-191). Matches `MAX_EXPR_DEPTH`; no real annotation nests this deep.
const MAX_TYPE_DEPTH: usize = 256;

/// Maximum number of left-operand nodes one `|>` stage may COPY. A right-hand
/// side with `k` placeholders receives `k` copies of the operand, so
/// `x |> f(_, _) |> f(_, _) …` doubles the tree per stage: sixteen stages of a
/// 200-byte source cost 720 MB and 100 s in the passes downstream (bug-501 B).
/// The charge is `(k - 1) × nodes(operand)` — the ordinary single `_` copies
/// nothing and is never charged — so no real pipeline comes near it, while the
/// doubling attack is refused at its thirteenth stage, before the copy is made.
const MAX_PIPELINE_COPIED_NODES: usize = 4096;

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
    pub(super) fn normalize_qualified_builtin_type(&self, qualified: String) -> String {
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

    /// Inside `builtins/<pkg>.mfb`, qualify a BARE name that `<pkg>` declares as
    /// one of its own value types; leave everything else alone.
    ///
    /// This is what lets the injected companion keep writing its own types the
    /// way the governing rule says a local name is written — unprefixed — while
    /// the namespace it lands in is package-scoped. User source gets no such
    /// treatment: a bare imported type there is exactly what Phase 4b refuses.
    pub(super) fn qualify_own_builtin_type(&self, name: String) -> String {
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

    /// Enter one expression-nesting level, reporting and returning `false` when
    /// the maximum depth is exceeded. On the `false` path the counter is already
    /// rewound, so the caller must simply bail (`return None`); otherwise it must
    /// pair a successful `enter_expr` with exactly one `leave_expr`.
    fn enter_expr(&mut self) -> bool {
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Expression nesting is too deep.",
                &token,
            );
            self.expr_depth -= 1;
            false
        } else {
            true
        }
    }

    fn leave_expr(&mut self) {
        self.expr_depth -= 1;
    }

    /// Record the tree depth of the expression just built (0 for a leaf,
    /// `1 + max(children)` for a node), reporting at token `at` and returning
    /// `false` when the tree exceeds `MAX_EXPR_DEPTH`; the caller then bails
    /// with `return None`, exactly like the `enter_expr` path.
    ///
    /// `enter_expr` bounds this parser's *recursion*; this bounds its *result*.
    /// The two differ on the left-associative loops (`a+b+c…`, `a.b.c…`), which
    /// deepen the tree by one per iteration without recursing, and on `|>`,
    /// which splices the left operand under the right one — a flat 40 KB
    /// `1+1+…` parsed fine and then overflowed the native stack in the passes
    /// that re-walk the tree (audit-3 FE-01 / bug-501). The parser is the one
    /// place the depth can be bounded before anything walks it. The convention
    /// matches `ir::verify::check_value_depth` (root at 0, rejected past
    /// `MAX_DEPTH`), so a tree the verifier accepted before is accepted here.
    pub(super) fn note_expr_tree_depth(&mut self, depth: usize, at: usize) -> bool {
        self.expr_tree_depth = depth;
        if depth > MAX_EXPR_DEPTH {
            let token = self.tokens[at].clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Expression nesting is too deep.",
                &token,
            );
            self.latch_hostile_expression();
            return false;
        }
        true
    }

    /// Recovery after a hostile-expression diagnostic: latch `depth_exceeded`
    /// and collapse the cursor to `Eof` (the statement- and type-depth guards'
    /// recovery, bug-183 / bug-191), so the enclosing parses unwind without
    /// consuming the rest of the pathological expression or emitting a trailing
    /// cascade of `Expected )` / `Expected end of statement` for it.
    fn latch_hostile_expression(&mut self) {
        self.depth_exceeded = true;
        self.seek_to_end();
    }

    pub(super) fn parse_expression(&mut self) -> Option<Expression> {
        if !self.enter_expr() {
            return None;
        }
        let result = self.parse_pipeline();
        self.leave_expr();
        result
    }

    pub(super) fn parse_pipeline(&mut self) -> Option<Expression> {
        let mut expression = self.parse_or()?;
        let mut depth = self.expr_tree_depth;
        while self.match_kind(TokenKind::PipeGreater) {
            let operator_at = self.current - 1;
            let token = self.previous().clone();
            let right = self.parse_or()?;
            let shape = placeholder_shape(&right);
            if shape.count == 0 {
                self.report(
                    "MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING",
                    "Pipeline right-hand side must contain `_` as the input placeholder.",
                    &token,
                );
                return None;
            }
            // Every `_` receives its own copy of the left operand, so a right-
            // hand side with several placeholders multiplies the tree; refuse the
            // copy before making it (bug-501 B).
            if shape.count > 1 {
                let copied = (shape.count - 1) * node_count(&expression);
                if copied > MAX_PIPELINE_COPIED_NODES {
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        &format!(
                            "Pipeline placeholder substitution is too large: `_` occurs {} \
                             times, which would copy {copied} nodes of the left-hand side \
                             (limit {MAX_PIPELINE_COPIED_NODES}).",
                            shape.count
                        ),
                        &token,
                    );
                    self.latch_hostile_expression();
                    return None;
                }
            }
            // The result is `right` with the operand spliced in at its `_` leaves.
            depth = shape.depth.max(shape.placeholder_depth + depth);
            if !self.note_expr_tree_depth(depth, operator_at) {
                return None;
            }
            expression = substitute_placeholder(right, &expression);
        }
        Some(expression)
    }

    pub(super) fn parse_or(&mut self) -> Option<Expression> {
        let mut expression = self.parse_and()?;
        let mut depth = self.expr_tree_depth;
        while self.match_any_keywords(&[Keyword::Or, Keyword::Xor]) {
            // coverage:off — the preceding match_any_keywords guarantees the
            // previous token is OR or XOR, both of which spell an operator.
            let Some(operator) = BinaryOp::from_token(&self.previous().kind) else {
                unreachable!()
            };
            // coverage:on
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_and()?;
            depth = depth.max(self.expr_tree_depth) + 1;
            if !self.note_expr_tree_depth(depth, operator_at) {
                return None;
            }
            expression = Expression::Binary {
                left: Box::new(expression),
                operator,
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_and(&mut self) -> Option<Expression> {
        let mut expression = self.parse_not()?;
        let mut depth = self.expr_tree_depth;
        while self.match_keyword(Keyword::And) {
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_not()?;
            depth = depth.max(self.expr_tree_depth) + 1;
            if !self.note_expr_tree_depth(depth, operator_at) {
                return None;
            }
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: BinaryOp::And,
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_not(&mut self) -> Option<Expression> {
        if self.match_keyword(Keyword::Not) {
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            if !self.enter_expr() {
                return None;
            }
            let operand = self.parse_not();
            self.leave_expr();
            let operand = operand?;
            if !self.note_expr_tree_depth(self.expr_tree_depth + 1, operator_at) {
                return None;
            }
            return Some(Expression::Unary {
                operator: UnaryOp::Not,
                operand: Box::new(operand),
                line,
                column,
            });
        }
        self.parse_comparison()
    }

    pub(super) fn parse_comparison(&mut self) -> Option<Expression> {
        let mut expression = self.parse_concat()?;
        let mut depth = self.expr_tree_depth;
        while self.match_any(&[
            TokenKind::Equal,
            TokenKind::NotEqual,
            TokenKind::Less,
            TokenKind::LessEqual,
            TokenKind::Greater,
            TokenKind::GreaterEqual,
        ]) {
            // coverage:off — the preceding match_any guarantees a comparison
            // operator token here.
            let Some(operator) = BinaryOp::from_token(&self.previous().kind) else {
                unreachable!()
            };
            // coverage:on
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_concat()?;
            depth = depth.max(self.expr_tree_depth) + 1;
            if !self.note_expr_tree_depth(depth, operator_at) {
                return None;
            }
            expression = Expression::Binary {
                left: Box::new(expression),
                operator,
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_concat(&mut self) -> Option<Expression> {
        let mut expression = self.parse_addition()?;
        let mut depth = self.expr_tree_depth;
        while self.match_kind(TokenKind::Ampersand) {
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_addition()?;
            depth = depth.max(self.expr_tree_depth) + 1;
            if !self.note_expr_tree_depth(depth, operator_at) {
                return None;
            }
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: BinaryOp::Concat,
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_addition(&mut self) -> Option<Expression> {
        let mut expression = self.parse_multiplication()?;
        let mut depth = self.expr_tree_depth;
        while self.match_any(&[TokenKind::Plus, TokenKind::Minus]) {
            // coverage:off — the preceding match_any guarantees `+` or `-`.
            let Some(operator) = BinaryOp::from_token(&self.previous().kind) else {
                unreachable!()
            };
            // coverage:on
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_multiplication()?;
            depth = depth.max(self.expr_tree_depth) + 1;
            if !self.note_expr_tree_depth(depth, operator_at) {
                return None;
            }
            expression = Expression::Binary {
                left: Box::new(expression),
                operator,
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_multiplication(&mut self) -> Option<Expression> {
        let mut expression = self.parse_power()?;
        let mut depth = self.expr_tree_depth;
        while self.match_any(&[TokenKind::Star, TokenKind::Slash])
            || self.match_any_keywords(&[Keyword::Mod, Keyword::Div])
        {
            // coverage:off — the preceding match guards guarantee `*`, `/`,
            // MOD, or DIV here.
            let Some(operator) = BinaryOp::from_token(&self.previous().kind) else {
                unreachable!()
            };
            // coverage:on
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_power()?;
            depth = depth.max(self.expr_tree_depth) + 1;
            if !self.note_expr_tree_depth(depth, operator_at) {
                return None;
            }
            expression = Expression::Binary {
                left: Box::new(expression),
                operator,
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_power(&mut self) -> Option<Expression> {
        let mut expression = self.parse_unary()?;
        let left_depth = self.expr_tree_depth;
        if self.match_kind(TokenKind::Caret) {
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            if !self.enter_expr() {
                return None;
            }
            let right = self.parse_power();
            self.leave_expr();
            let right = right?;
            let depth = left_depth.max(self.expr_tree_depth) + 1;
            if !self.note_expr_tree_depth(depth, operator_at) {
                return None;
            }
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: BinaryOp::Power,
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_unary(&mut self) -> Option<Expression> {
        if self.match_kind(TokenKind::Minus) {
            let operator_at = self.current - 1;
            let (line, column) = (self.previous().line, self.previous().start);
            if !self.enter_expr() {
                return None;
            }
            let operand = self.parse_unary();
            self.leave_expr();
            let operand = operand?;
            if !self.note_expr_tree_depth(self.expr_tree_depth + 1, operator_at) {
                return None;
            }
            return Some(Expression::Unary {
                operator: UnaryOp::Negate,
                operand: Box::new(operand),
                line,
                column,
            });
        }
        if self.match_keyword(Keyword::With) {
            return self.parse_with_update();
        }
        self.parse_member_access()
    }

    pub(super) fn parse_with_update(&mut self) -> Option<Expression> {
        let with_at = self.current - 1;
        let target = self.parse_member_access()?;
        let mut depth = self.expr_tree_depth;
        if !self.consume_kind(TokenKind::LBrace, "Expected `{` after WITH target.") {
            return None;
        }
        let mut updates = Vec::new();
        if !self.check_kind(&TokenKind::RBrace) {
            loop {
                let line = self.peek().line;
                let Some(field) =
                    self.consume_identifier("WITH update field must be an identifier.")
                else {
                    self.synchronize();
                    return None;
                };
                if !self.consume_kind(
                    TokenKind::ColonEqual,
                    "Expected `:=` between WITH update field and value.",
                ) {
                    return None;
                }
                let value = self.parse_expression()?;
                depth = depth.max(self.expr_tree_depth);
                updates.push(RecordUpdate { field, value, line });
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.consume_kind(TokenKind::RBrace, "Expected `}` after WITH updates.") {
            return None;
        }
        if !self.note_expr_tree_depth(depth + 1, with_at) {
            return None;
        }
        Some(Expression::WithUpdate {
            target: Box::new(target),
            updates,
        })
    }

    pub(super) fn parse_member_access(&mut self) -> Option<Expression> {
        let mut expression = self.parse_call_or_constructor()?;
        let mut depth = self.expr_tree_depth;
        while self.match_kind(TokenKind::Dot) {
            let dot_at = self.current - 1;
            let member = self.consume_identifier("Expected identifier after `.`.")?;
            depth += 1;
            if !self.note_expr_tree_depth(depth, dot_at) {
                return None;
            }
            expression = Expression::MemberAccess {
                target: Box::new(expression),
                member,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_call_or_constructor(&mut self) -> Option<Expression> {
        let start = self.peek().clone();
        let mut expression = self.parse_primary()?;
        loop {
            if self.match_kind(TokenKind::LParen) {
                let paren_at = self.current - 1;
                let callee = match expression {
                    Expression::Identifier(value) => value,
                    _ => {
                        let token = self.previous().clone();
                        self.report(
                            "MFB_PARSE_EXPECTED_EXPRESSION",
                            "Only identifiers can be called by the current parser.",
                            &token,
                        );
                        return None;
                    }
                };
                let arguments = self.parse_argument_list(TokenKind::RParen)?;
                // The callee is a bare identifier (a leaf), so the call is one
                // deeper than its deepest argument.
                if !self.note_expr_tree_depth(self.expr_tree_depth + 1, paren_at) {
                    return None;
                }
                expression = Expression::Call {
                    callee,
                    arguments,
                    line: start.line,
                    column: start.start,
                };
            } else if self.match_kind(TokenKind::LBracket) {
                let bracket_at = self.current - 1;
                let type_name = match expression {
                    // A package-qualified built-in type used as a constructor
                    // (`http::Response[...]`) normalizes to its bare id, matching
                    // the type-position rule (plan-03-http.md §A.1/§B.2). The
                    // identifier arm already normalized it, so this is idempotent
                    // and stays only to cover a constructor head that reached here
                    // by some other route.
                    Expression::Identifier(value) => self.normalize_qualified_builtin_type(value),
                    _ => {
                        let token = self.previous().clone();
                        self.report(
                            "MFB_PARSE_EXPECTED_EXPRESSION",
                            "Only identifiers can be used as constructors.",
                            &token,
                        );
                        return None;
                    }
                };
                let arguments = self.parse_constructor_argument_list(TokenKind::RBracket)?;
                if !self.note_expr_tree_depth(self.expr_tree_depth + 1, bracket_at) {
                    return None;
                }
                expression = Expression::Constructor {
                    type_name,
                    arguments,
                };
            } else {
                break;
            }
        }
        Some(expression)
    }

    /// On success `expr_tree_depth` is left at the deepest argument's depth
    /// (0 for an empty list) for the `Call` node that owns the list.
    pub(super) fn parse_argument_list(&mut self, closing: TokenKind) -> Option<Vec<CallArg>> {
        let mut arguments = Vec::new();
        let mut depth = 0;
        if !self.check_kind(&closing) {
            loop {
                if matches!(self.peek().kind, TokenKind::Identifier(_))
                    && self
                        .peek_next()
                        .is_some_and(|token| matches!(token.kind, TokenKind::ColonEqual))
                {
                    let line = self.peek().line;
                    let name =
                        self.consume_identifier("Call argument name must be an identifier.")?;
                    self.consume_kind(
                        TokenKind::ColonEqual,
                        "Expected `:=` between call argument name and value.",
                    );
                    let value = self.parse_expression()?;
                    depth = depth.max(self.expr_tree_depth);
                    arguments.push(CallArg::Named { name, value, line });
                } else {
                    arguments.push(CallArg::Positional(self.parse_expression()?));
                    depth = depth.max(self.expr_tree_depth);
                }
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        let detail = match closing {
            TokenKind::RParen => "Expected `)` after call arguments.",
            TokenKind::RBracket => "Expected `]` after constructor arguments.",
            _ => "Expected closing delimiter after arguments.",
        };
        if !self.consume_kind(closing, detail) {
            return None;
        }
        self.expr_tree_depth = depth;
        Some(arguments)
    }

    /// On success `expr_tree_depth` is left at the deepest argument's depth
    /// (0 for an empty list) for the `Constructor` node that owns the list.
    pub(super) fn parse_constructor_argument_list(
        &mut self,
        closing: TokenKind,
    ) -> Option<Vec<ConstructorArg>> {
        let mut arguments = Vec::new();
        let mut depth = 0;
        if !self.check_kind(&closing) {
            loop {
                if matches!(self.peek().kind, TokenKind::Identifier(_))
                    && self
                        .peek_next()
                        .is_some_and(|token| matches!(token.kind, TokenKind::ColonEqual))
                {
                    let line = self.peek().line;
                    let name =
                        self.consume_identifier("Constructor field name must be an identifier.")?;
                    self.consume_kind(
                        TokenKind::ColonEqual,
                        "Expected `:=` between constructor field and value.",
                    );
                    let value = self.parse_expression()?;
                    depth = depth.max(self.expr_tree_depth);
                    arguments.push(ConstructorArg::Named { name, value, line });
                } else {
                    arguments.push(ConstructorArg::Positional(self.parse_expression()?));
                    depth = depth.max(self.expr_tree_depth);
                }
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        let detail = match closing {
            TokenKind::RBracket => "Expected `]` after constructor arguments.",
            _ => "Expected closing delimiter after constructor arguments.",
        };
        if !self.consume_kind(closing, detail) {
            return None;
        }
        self.expr_tree_depth = depth;
        Some(arguments)
    }

    pub(super) fn parse_primary(&mut self) -> Option<Expression> {
        // At end-of-input `advance()` does not move the cursor and re-yields the
        // token *before* Eof (an already-consumed `(`/`[`), which would re-enter
        // the grouped-expression / list-literal arms with zero progress and
        // recurse until the native stack overflows (bug-89). Treat Eof as a hard
        // parse error here instead of re-reading `previous()`.
        if self.is_at_end() {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_EXPECTED_EXPRESSION",
                "Expected an expression.",
                &token,
            );
            return None;
        }
        let token = self.advance().clone();
        // A leaf is depth 0; the compound arms (grouping, literals, lambda)
        // overwrite this with their own depth as they build.
        self.expr_tree_depth = 0;
        match token.kind {
            TokenKind::String(value) => Some(Expression::String(value)),
            TokenKind::Number(value) => Some(Expression::Number(value)),
            TokenKind::Scalar(code_point) => Some(Expression::Scalar(code_point)),
            TokenKind::Keyword(Keyword::True) => Some(Expression::Boolean(true)),
            TokenKind::Keyword(Keyword::False) => Some(Expression::Boolean(false)),
            TokenKind::Keyword(Keyword::Nothing) => {
                Some(Expression::Identifier("NOTHING".to_string()))
            }
            TokenKind::Keyword(Keyword::Lambda) => self.parse_lambda(),
            TokenKind::Identifier(value) => {
                if value.eq_ignore_ascii_case("Map") && self.check_identifier_ci("OF") {
                    self.advance();
                    let key_type = self.parse_type_name()?;
                    if !self.check_identifier_ci("TO") && !self.check_keyword(Keyword::To) {
                        let token = self.peek().clone();
                        self.report(
                            "MFB_PARSE_UNEXPECTED_TOKEN",
                            "Expected `TO` in map literal type.",
                            &token,
                        );
                        return None;
                    }
                    self.advance();
                    // A `Map OF K TO RES File { … }` literal carries the resource
                    // ownership-axis marker on its value type (§15.6).
                    let value_res = self.match_keyword(Keyword::Res);
                    let value_type = self.parse_type_name()?;
                    let value_type = if value_res {
                        format!("RES {value_type}")
                    } else {
                        value_type
                    };
                    return self.parse_map_literal(key_type, value_type);
                }
                if value.eq_ignore_ascii_case("Set") && self.check_identifier_ci("OF") {
                    self.advance();
                    // A Set element can never be a resource (`RES` forbidden).
                    if self.check_keyword(Keyword::Res) {
                        let token = self.peek().clone();
                        self.report(
                            "MFB_PARSE_UNEXPECTED_TOKEN",
                            "A Set element cannot be a resource; `RES` is not allowed after `Set OF`.",
                            &token,
                        );
                        return None;
                    }
                    let element_type = self.parse_type_name()?;
                    return self.parse_set_literal(element_type);
                }
                // bug-480 Defect B1. A package-qualified name in an EXPRESSION
                // position gets the same normalization the type and constructor
                // positions have had since plan-03-http. Without it, the head of
                // `net::PingStatus.Ok` stayed spelled `net.PingStatus` while the
                // enum is declared bare, so every `enums` lookup missed, the
                // expression typed as nothing, and the unresolved name escaped
                // the front end to be reported by NIR as
                // `NIR local reference 'net.PingStatus' does not resolve` — no
                // file, no line, no code.
                //
                // Only a name the registry resolves as a builtin value type is
                // rewritten, so a qualified FUNCTION (`net::toUrl` as a value)
                // or CONSTANT (`math::pi`) is left exactly as it was.
                let name = self.finish_qualified_name(value)?;
                Some(Expression::Identifier(
                    self.normalize_qualified_builtin_type(name),
                ))
            }
            TokenKind::LParen => {
                let expression = self.parse_expression();
                self.consume_kind(TokenKind::RParen, "Expected `)` after expression.");
                expression
            }
            TokenKind::LBracket => self.parse_list_literal(),
            _ => {
                self.report(
                    "MFB_PARSE_EXPECTED_EXPRESSION",
                    "Expected an expression.",
                    &token,
                );
                None
            }
        }
    }

    pub(super) fn parse_qualified_name(&mut self, detail: &str) -> Option<String> {
        let name = self.consume_identifier(detail)?;
        self.finish_qualified_name(name)
    }

    pub(super) fn finish_qualified_name(&mut self, mut name: String) -> Option<String> {
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
    pub(super) fn parse_type_name(&mut self) -> Option<String> {
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

            if name.eq_ignore_ascii_case("List") || name.eq_ignore_ascii_case("Result") {
                // `List OF RES File` (§15.6): a resource-transfer list whose
                // element is a pointer whose scope-ownership transfers across a
                // function boundary. (`Result OF RES …` is not meaningful, but the
                // marker is harmless there and rejected later by type checking.)
                let element_res =
                    name.eq_ignore_ascii_case("List") && self.match_keyword(Keyword::Res);
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

            let mut args = vec![self.parse_type_name()?];
            while self.match_kind(TokenKind::Comma) {
                args.push(self.parse_type_name()?);
            }
            // coverage:off — `args` is seeded with one parsed type above, so it is
            // never empty here; this guard is defensive.
            if args.is_empty() {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_TOKEN",
                    "Expected at least one template type argument.",
                    &token,
                );
                return None;
            }
            // coverage:on
            name.push_str(" OF ");
            name.push_str(&args.join(", "));
        }
        Some(name)
    }

    /// Parse a thread type body after `<kind> OF`, supporting the optional
    /// resource plane: `Thread OF Msg TO Out`, `Thread OF Msg RES Res TO Out`,
    /// or the resource-only `Thread OF RES Res TO Out` (message defaults to
    /// `Nothing`). `kind` is the leading `Thread`/`ThreadWorker` token.
    pub(super) fn parse_thread_type_name(&mut self, kind: String) -> Option<String> {
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

    pub(super) fn parse_function_type_name(&mut self, isolated: bool) -> Option<String> {
        if !self.consume_kind(TokenKind::LParen, "Function type must include `(`.") {
            return None;
        }
        let mut params = Vec::new();
        if !self.check_kind(&TokenKind::RParen) {
            loop {
                params.push(self.parse_type_name()?);
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
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

    pub(super) fn parse_lambda(&mut self) -> Option<Expression> {
        let lambda_at = self.current - 1;
        if !self.consume_kind(TokenKind::LParen, "Lambda must include `(` after LAMBDA.") {
            return None;
        }
        let params = self.parse_params();
        if !self.consume_kind(TokenKind::RParen, "Lambda must close the parameter list.") {
            return None;
        }
        if !self.consume_kind(
            TokenKind::Arrow,
            "Lambda must include `->` before its body.",
        ) {
            return None;
        }
        // A lambda body of the form `name = <expr>` is an assignment (the same
        // `identifier =` lookahead the statement parser uses to tell assignment
        // from the `=` equality operator). It mutates `name` and yields Nothing;
        // this is the shape a non-escaping callback uses to update a captured
        // `MUT` binding.
        let assign_target = if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            if self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Equal))
            {
                self.advance();
                self.advance();
                Some(name)
            } else {
                None
            }
        } else {
            None
        };
        let body = self.parse_expression()?;
        if !self.note_expr_tree_depth(self.expr_tree_depth + 1, lambda_at) {
            return None;
        }
        Some(Expression::Lambda {
            params,
            body: Box::new(body),
            assign_target,
        })
    }

    pub(super) fn parse_type_base_name(&mut self, detail: &str) -> Option<String> {
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
        Some(self.normalize_qualified_builtin_type(qualified))
    }

    pub(super) fn parse_list_literal(&mut self) -> Option<Expression> {
        let bracket_at = self.current - 1;
        let mut values = Vec::new();
        let mut depth = 0;
        if !self.check_kind(&TokenKind::RBracket) {
            loop {
                values.push(self.parse_expression()?);
                depth = depth.max(self.expr_tree_depth);
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume_kind(TokenKind::RBracket, "Expected `]` after list literal.");
        if !self.note_expr_tree_depth(depth + 1, bracket_at) {
            return None;
        }
        Some(Expression::ListLiteral(values))
    }

    pub(super) fn parse_map_literal(
        &mut self,
        key_type: String,
        value_type: String,
    ) -> Option<Expression> {
        if !self.consume_kind(TokenKind::LBrace, "Expected `{` after map literal type.") {
            return None;
        }
        let brace_at = self.current - 1;
        let mut entries = Vec::new();
        let mut depth = 0;
        if !self.check_kind(&TokenKind::RBrace) {
            loop {
                let key = self.parse_expression()?;
                depth = depth.max(self.expr_tree_depth);
                if !self.consume_kind(
                    TokenKind::ColonEqual,
                    "Expected `:=` between map key and value.",
                ) {
                    return None;
                }
                let value = self.parse_expression()?;
                depth = depth.max(self.expr_tree_depth);
                entries.push((key, value));
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume_kind(TokenKind::RBrace, "Expected `}` after map literal.");
        if !self.note_expr_tree_depth(depth + 1, brace_at) {
            return None;
        }
        Some(Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        })
    }

    /// Parse a `Set OF T { e1, e2, … }` literal body (the `element_type` has been
    /// consumed). Empty `Set OF T { }` is permitted (the empty-set default).
    pub(super) fn parse_set_literal(&mut self, element_type: String) -> Option<Expression> {
        if !self.consume_kind(TokenKind::LBrace, "Expected `{` after set literal type.") {
            return None;
        }
        let brace_at = self.current - 1;
        let mut elements = Vec::new();
        let mut depth = 0;
        if !self.check_kind(&TokenKind::RBrace) {
            loop {
                elements.push(self.parse_expression()?);
                depth = depth.max(self.expr_tree_depth);
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume_kind(TokenKind::RBrace, "Expected `}` after set literal.");
        if !self.note_expr_tree_depth(depth + 1, brace_at) {
            return None;
        }
        Some(Expression::SetLiteral {
            element_type,
            elements,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse(src: &str) -> Result<crate::ast::AstFile, ()> {
        crate::ast::parse_source(Path::new("main.mfb"), "main.mfb", src)
    }

    // ---- Depth guards (bug-171 / bug-191): a nesting past MAX_*_DEPTH (256) ----
    // Parsing (then unwinding) ~300 recursive levels needs more than a test
    // thread's default stack, so run each on a generous one (matching the
    // existing statement-depth-cap test in `tests.rs`).

    fn on_big_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(f)
            .expect("spawn")
            .join()
            .expect("deep-nesting parse must not overflow the stack");
    }

    #[test]
    fn expr_depth_guard_grouping() {
        // ~300 nested `(` drives `parse_expression`'s `enter_expr` past the cap:
        // the `enter_expr` false body (26-33) and `parse_expression`'s bail (45).
        on_big_stack(|| {
            let src = format!(
                "FUNC f AS Integer\n  RETURN {}1{}\nEND FUNC\n",
                "(".repeat(300),
                ")".repeat(300),
            );
            assert!(parse(&src).is_err());
        });
    }

    #[test]
    fn expr_depth_guard_not_chain() {
        // A deep `NOT NOT …` chain trips `parse_not`'s own `enter_expr` bail (114).
        on_big_stack(|| {
            let src = format!(
                "FUNC f AS Integer\n  RETURN {}a\nEND FUNC\n",
                "NOT ".repeat(300),
            );
            assert!(parse(&src).is_err());
        });
    }

    #[test]
    fn expr_depth_guard_power_chain() {
        // A deep `^` chain trips `parse_power`'s `enter_expr` bail (236).
        on_big_stack(|| {
            let src = format!(
                "FUNC f AS Integer\n  RETURN 2{}\nEND FUNC\n",
                "^2".repeat(300),
            );
            assert!(parse(&src).is_err());
        });
    }

    #[test]
    fn expr_depth_guard_unary_minus_chain() {
        // A deep unary-minus chain trips `parse_unary`'s `enter_expr` bail (256).
        on_big_stack(|| {
            let src = format!(
                "FUNC f AS Integer\n  RETURN {}a\nEND FUNC\n",
                "-".repeat(300),
            );
            assert!(parse(&src).is_err());
        });
    }

    #[test]
    fn type_depth_guard_list_chain() {
        // ~300 nested `List OF …` trips `parse_type_name`'s `enter_type` false
        // body (569-582) and its bail (597).
        on_big_stack(|| {
            let src = format!("SUB s(x AS {}Integer)\nEND SUB\n", "List OF ".repeat(300),);
            assert!(parse(&src).is_err());
        });
    }

    // ---- Tree-depth guard (audit-3 FE-01 / bug-501) ----
    // The loops that build left-associative chains never recurse, so the native
    // guard above cannot see them; the BUILT tree is bounded instead, at the node
    // that crosses MAX_EXPR_DEPTH. No big stack is needed here — that is the point.

    fn returning(expression: &str) -> String {
        format!("FUNC f AS Integer\n  RETURN {expression}\nEND FUNC\n")
    }

    #[test]
    fn tree_depth_guard_addition_chain() {
        // 300 `+` build a 300-deep left spine with zero recursive re-entries.
        assert!(parse(&returning(&format!("1{}", "+1".repeat(300)))).is_err());
    }

    #[test]
    fn tree_depth_guard_admits_a_chain_at_the_cap() {
        // MAX_EXPR_DEPTH operators is the deepest chain `ir::verify` accepted
        // before this guard existed (root at 0, rejected past 256); the parser
        // must agree exactly, or a program that compiled would stop compiling.
        assert!(parse(&returning(&format!("1{}", "+1".repeat(256)))).is_ok());
        assert!(parse(&returning(&format!("1{}", "+1".repeat(257)))).is_err());
    }

    #[test]
    fn tree_depth_guard_member_chain() {
        assert!(parse(&returning(&format!("a{}", ".b".repeat(300)))).is_err());
    }

    #[test]
    fn tree_depth_guard_nested_groups_of_chains() {
        // Each grouping level and each chain stays far under the cap on its own
        // (30 groups, 20 terms each), so only the depth of the BUILT tree — a
        // 600-deep left spine — can see this. 250 × 20 overflowed the release
        // binary's stack in the lowering passes before the guard.
        let mut expression = String::from("1");
        for _ in 0..30 {
            expression = format!("({expression}{})", "+1".repeat(20));
        }
        assert!(parse(&returning(&expression)).is_err());
    }

    #[test]
    fn tree_depth_guard_pipeline_splice() {
        // `|>` splices the left operand under the `_`, so 300 stages of `f(_)`
        // nest 300 calls without a single recursive re-entry in the parser.
        assert!(parse(&returning(&format!("1{}", " |> f(_)".repeat(300)))).is_err());
    }

    #[test]
    fn pipeline_placeholder_copy_budget() {
        // Two placeholders per stage double the tree (bug-501 B): the copy is
        // refused at the stage where it would exceed MAX_PIPELINE_COPIED_NODES…
        assert!(parse(&returning(&format!("1{}", " |> f(_, _)".repeat(20)))).is_err());
        // …while an ordinary multi-placeholder pipeline is untouched.
        assert!(parse(&returning("1 |> f(_, _) |> f(_, _)")).is_ok());
    }

    // ---- parse_primary ----

    #[test]
    fn primary_eof_after_trailing_operator() {
        // A trailing operator with no following token (no newline before Eof)
        // re-enters `parse_primary` at end-of-input, hitting the bug-89 Eof guard
        // (460-466) instead of re-reading an already-consumed token.
        assert!(parse("FUNC f AS Integer\n  RETURN a +").is_err());
    }

    #[test]
    fn primary_scalar_literal() {
        // A backtick scalar literal reaches the `Scalar` primary arm (472).
        let file =
            crate::testutil::parse_file("FUNC f AS Integer\n  LET x = `A`\n  RETURN 0\nEND FUNC\n");
        // A successful parse is enough to exercise the arm; confirm it parsed.
        assert!(!file.items.is_empty());
    }

    #[test]
    fn set_literal_rejects_res_element() {
        // `Set OF RES …` in expression (literal) position is rejected (508-514).
        assert!(
            parse("FUNC f AS Integer\n  LET x = Set OF RES File { }\n  RETURN 0\nEND FUNC\n")
                .is_err()
        );
    }

    #[test]
    fn map_literal_missing_open_brace() {
        // `Map OF K TO V` with no `{` bails in `parse_map_literal` (882).
        assert!(parse(
            "FUNC f AS Integer\n  LET m = Map OF Integer TO Integer 5\n  RETURN 0\nEND FUNC\n"
        )
        .is_err());
    }

    #[test]
    fn set_literal_missing_open_brace() {
        // `Set OF T` with no `{` bails in `parse_set_literal` (913).
        assert!(
            parse("FUNC f AS Integer\n  LET s = Set OF Integer 5\n  RETURN 0\nEND FUNC\n").is_err()
        );
    }

    #[test]
    fn set_literal_multiple_elements() {
        // A non-empty set literal with a comma walks the element loop (918-921).
        let file = crate::testutil::parse_file(
            "FUNC f AS Integer\n  LET s = Set OF Integer { 1, 2 }\n  RETURN 0\nEND FUNC\n",
        );
        assert!(!file.items.is_empty());
    }

    // ---- Defensive `detail`-selection arms in the argument-list parsers ----
    // `parse_argument_list` is only ever invoked with `RParen` and
    // `parse_constructor_argument_list` only with `RBracket`, so the other
    // `match closing` arms that pick the error string are unreachable through the
    // grammar. Both fns are `pub(super)`, so drive those arms with a direct call
    // whose leading token is the requested closing delimiter (the loop is then
    // skipped and the `detail` match still runs).

    fn parser_over(src: &str) -> FileParser<'static> {
        let tokens = crate::lexer::lex(Path::new("m"), src).expect("lex");
        FileParser::new(Path::new("m"), tokens)
    }

    #[test]
    fn argument_list_detail_arms() {
        // `RBracket` closing → the RBracket detail arm (405).
        let mut p = parser_over("]");
        assert!(p.parse_argument_list(TokenKind::RBracket).is_some());
        // A closing that is neither `RParen` nor `RBracket` → the `_` arm (406).
        let mut p = parser_over("}");
        assert!(p.parse_argument_list(TokenKind::RBrace).is_some());
    }

    #[test]
    fn constructor_argument_list_detail_default_arm() {
        // A non-`RBracket` closing → the `_` detail arm (445).
        let mut p = parser_over(")");
        assert!(p
            .parse_constructor_argument_list(TokenKind::RParen)
            .is_some());
    }
}
