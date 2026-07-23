use super::*;

impl TypeEnv {
    // 3. Value walk: literal ranges and const-literal bounds
    // ===========================================================================

    /// Enforce the semantic rules on a value expression and recurse into its
    /// sub-values. Argument and sub-expression checks run before the node's own
    /// rule so the innermost violation surfaces first.
    pub(super) fn check_value(&self, value: &IrValue, locals: &HashMap<String, String>) {
        self.check_value_depth(value, locals, 0);
    }

    /// Depth-bounded body of `check_value`. Value expressions can nest as deeply
    /// as a crafted `.mfp` (or synthesized IR) allows, so — mirroring `check_ops`'
    /// statement-nesting cap — the recursion is bounded to `MAX_DEPTH` levels and
    /// fails gracefully with the same `VERIFY_TYPE` diagnostic rather than
    /// overflowing the stack.
    pub(super) fn check_value_depth(
        &self,
        value: &IrValue,
        locals: &HashMap<String, String>,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            self.emit(
                VERIFY_TYPE,
                format!("expression nesting exceeds the {MAX_DEPTH} level limit"),
            );
            return;
        }
        // bug-301 G2: `allow_sub_call` is a single shared `Cell` set for a
        // statement-position value, and only the `Call` arm consumes it. But this
        // walker recurses into operands and arguments BEFORE the wrapping node's
        // own rule, so for a shape like `Eval(Binary(a, Call(sub)))` the DFS
        // reached the nested SUB call with the flag still set -- marking it
        // statement position and skipping `TYPE_SUB_HAS_NO_VALUE`. The intent is
        // that only a value whose ROOT is the call may be value-less, so any other
        // node clears the flag before descending.
        if !matches!(value, IrValue::Call { .. } | IrValue::CallResult { .. }) {
            self.allow_sub_call.set(false);
        }
        match value {
            IrValue::MemberAccess { target, member, .. } => {
                self.check_value_depth(target, locals, depth + 1);
                self.check_member_access(target, member, locals);
                self.check_member_access_type(target, member, value, locals);
            }
            IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
                // Statement position permits a value-less SUB call; any nested
                // call (arguments, operands, initializers) is value position.
                let statement_position = self.allow_sub_call.replace(false);
                if !statement_position
                    && self
                        .functions
                        .get(target)
                        .is_some_and(|sig| sig.kind == "sub")
                {
                    self.emit(
                        "TYPE_SUB_HAS_NO_VALUE",
                        format!(
                            "SUB `{target}` produces no value; its call is a statement, not an expression."
                        ),
                    );
                }
                for arg in args {
                    self.check_value_depth(arg, locals, depth + 1);
                }
                self.check_call_arity(target, args.len(), locals);
                self.check_call_argument_types(target, args, locals);
                self.check_builtin_call_args(target, args, locals);
                self.check_call_result_type(target, value, args, locals);
            }
            IrValue::Constructor { type_, args } => {
                for arg in args {
                    self.check_value_depth(arg, locals, depth + 1);
                }
                self.check_constructor(type_, args, locals);
            }
            IrValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => {
                self.check_value_depth(value, locals, depth + 1);
                self.check_union_wrap(union_type, member_type);
            }
            IrValue::Closure { captures, .. } => {
                for capture in captures {
                    self.check_value_depth(capture, locals, depth + 1);
                }
            }
            IrValue::UnionExtract { type_, value } => {
                self.check_value_depth(value, locals, depth + 1);
                self.check_union_extract(type_, value, locals);
            }
            IrValue::ResultIsOk { value }
            | IrValue::ResultValue { value, .. }
            | IrValue::ResultError { value } => {
                self.check_value_depth(value, locals, depth + 1);
            }
            IrValue::Unary { op, operand, .. } => {
                self.check_value_depth(operand, locals, depth + 1);
                self.check_unary_operand(op, operand, locals);
                self.check_operator_result_type(
                    value,
                    derived_unary_type(op, self.infer_type(operand, locals).as_deref()),
                );
            }
            IrValue::Binary {
                op, left, right, ..
            } => {
                self.check_value_depth(left, locals, depth + 1);
                self.check_value_depth(right, locals, depth + 1);
                self.check_binary_operands(op, left, right, locals);
                self.check_operator_result_type(
                    value,
                    derived_binary_type(
                        op,
                        self.infer_type(left, locals).as_deref(),
                        self.infer_type(right, locals).as_deref(),
                    ),
                );
            }
            IrValue::WithUpdate {
                type_,
                target,
                updates,
            } => {
                self.check_value_depth(target, locals, depth + 1);
                // Compiler/runtime-owned records may never be updated —
                // syntaxcheck's TYPE_READ_ONLY_RECORD_UPDATE (message differs for
                // the Error pair vs the compiler-owned handle records). When
                // lowering could not stamp the update's type (e.g. the target
                // is a member access it didn't resolve), infer the target here.
                let inferred;
                let mut base = resource_base_type(type_);
                if base.is_empty() || base == "Unknown" {
                    inferred = self.infer_type(target, locals);
                    if let Some(t) = &inferred {
                        base = resource_base_type(t);
                    }
                }
                if matches!(base, "Error" | "ErrorLoc") {
                    self.emit(
                        "TYPE_READ_ONLY_RECORD_UPDATE",
                        format!("`{base}` is a read-only built-in record and cannot be updated."),
                    );
                } else if read_only_record_type(base) {
                    self.emit(
                        "TYPE_READ_ONLY_RECORD_UPDATE",
                        format!("TYPE `{base}` is read-only and cannot be updated."),
                    );
                }
                // Each WITH update must match its field's declared type —
                // syntaxcheck's WITH arm of TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH.
                let fields = self.field_types.get(resource_base_type(type_));
                let mut seen_fields: HashSet<&str> = HashSet::new();
                for update in updates {
                    self.check_value_depth(&update.value, locals, depth + 1);
                    // A WITH block may set each field at most once.
                    if !seen_fields.insert(update.field.as_str()) {
                        self.emit(
                            "TYPE_DUPLICATE_FIELD",
                            format!("WITH update sets field `{}` more than once.", update.field),
                        );
                    }
                    let Some(expected) = fields.and_then(|f| f.get(&update.field)) else {
                        continue;
                    };
                    let Some(actual) = self.infer_type(&update.value, locals) else {
                        continue;
                    };
                    if !self.expression_compatible(expected, &actual, &update.value) {
                        self.emit(
                            "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                            format!(
                                "WITH update for `{}` has type {actual}, expected {expected}.",
                                update.field
                            ),
                        );
                    }
                }
            }
            IrValue::ListLiteral { type_, values } => {
                for v in values {
                    self.check_value_depth(v, locals, depth + 1);
                }
                // plan-59-E: storing a non-`RES`-binding in a resource collection
                // used to be rejected here (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`,
                // retired). Under scope ownership the collection holds pointers to
                // resources owned by the outermost scope that touches them, so a
                // temporary is as admissible as a binding; the resource is still
                // closed exactly once, by that scope.
                // A crafted list whose elements do not match its element type is
                // a type confusion: codegen lays out and reads elements
                // uniformly by the declared element type.
                if let Some(element) = type_.strip_prefix("List OF ") {
                    for v in values {
                        self.check_literal_range(element, v);
                        if let Some(actual) = self.infer_type(v, locals) {
                            if !self.expression_compatible(element, &actual, v) {
                                self.emit(
                                    "TYPE_LIST_ELEMENT_MISMATCH",
                                    format!("List element has type {actual}, expected {element}."),
                                );
                            }
                        }
                    }
                }
            }
            IrValue::MapLiteral { type_, entries } => {
                for (k, v) in entries {
                    self.check_value_depth(k, locals, depth + 1);
                    self.check_value_depth(v, locals, depth + 1);
                }
                self.check_map_key_comparable(type_);
                if let Some((key_type, value_type)) = parse_map(type_) {
                    for (k, v) in entries {
                        self.check_literal_range(key_type, k);
                        self.check_literal_range(value_type, v);
                        if let Some(actual) = self.infer_type(k, locals) {
                            if !self.expression_compatible(key_type, &actual, k) {
                                self.emit(
                                    "TYPE_MAP_KEY_MISMATCH",
                                    format!("Map key has type {actual}, expected {key_type}."),
                                );
                            }
                        }
                        if let Some(actual) = self.infer_type(v, locals) {
                            if !self.expression_compatible(value_type, &actual, v) {
                                self.emit(
                                    "TYPE_MAP_VALUE_MISMATCH",
                                    format!("Map value has type {actual}, expected {value_type}."),
                                );
                            }
                        }
                    }
                }
            }
            IrValue::Const { .. }
            | IrValue::Local(_)
            | IrValue::Global(_)
            | IrValue::LocalRef { .. }
            | IrValue::FunctionRef { .. }
            | IrValue::Capture { .. } => {}
        }
    }

    /// Check a numeric literal in a position that expects `expected` against
    /// that type's range (`syntaxcheck`'s TYPE_*_LITERAL_OVERFLOW/UNDERFLOW).
    /// The check is contextual — keyed on the *expected* type, not the literal
    /// node's own type — because lowering does not push the expected type
    /// through a `-` negation (`-1` into `Byte` lowers to `Unary("-",
    /// Const{Integer,"1"})`, with `Byte` only on the enclosing bind). Matches
    /// the AST checker, which validates the literal against the expected type.
    pub(super) fn check_literal_range(&self, expected: &str, value: &IrValue) {
        // Only a *numeric* literal can overflow a numeric range; a non-numeric
        // Const in a numeric position (e.g. a String arg where Integer is
        // expected) is an argument/assignment mismatch, not a literal overflow.
        let numeric = |t: &str| {
            matches!(
                t,
                "Integer" | "Byte" | "Float" | "Fixed" | "Money" | "Scalar"
            )
        };
        match value {
            IrValue::Const { type_, value } if numeric(type_) => {
                self.check_const_literal(expected, value)
            }
            IrValue::Unary { op, operand, .. } if op == "-" => {
                if let IrValue::Const { type_, value } = operand.as_ref() {
                    if numeric(type_) {
                        self.check_negated_const_literal(expected, value);
                    }
                }
            }
            _ => {}
        }
    }

    /// The positive/overflow direction of the literal-range check.
    pub(super) fn check_const_literal(&self, type_: &str, value: &str) {
        match type_ {
            "Byte" if !value.contains('.') => {
                if value.parse::<u16>().map_or(true, |n| n > u8::MAX as u16) {
                    self.emit(
                        "TYPE_BYTE_LITERAL_OVERFLOW",
                        format!("Integer literal `{value}` is outside the Byte range 0..255."),
                    );
                }
            }
            "Integer" if !value.contains('.') => {
                if value.parse::<i64>().is_err() {
                    self.emit(
                        "TYPE_INTEGER_LITERAL_OVERFLOW",
                        format!("Integer literal `{value}` is outside the Integer range."),
                    );
                }
            }
            "Float" => {
                if let Ok(f) = value.parse::<f64>() {
                    if !f.is_finite() {
                        self.emit(
                            "TYPE_FLOAT_LITERAL_OVERFLOW",
                            format!("Numeric literal `{value}` is outside the Float range."),
                        );
                    }
                }
            }
            "Fixed" => {
                if let Ok(f) = value.parse::<f64>() {
                    if f >= 2147483648.0 {
                        self.emit(
                            "TYPE_FIXED_LITERAL_OVERFLOW",
                            format!("Numeric literal `{value}` is outside the Fixed range."),
                        );
                    }
                }
            }
            // Scalar is a Unicode scalar value: a codepoint in 0..=0x10FFFF that
            // is not a UTF-16 surrogate (0xD800..=0xDFFF). Source literals are
            // range-checked at lex time; a hand-crafted `.mfp` can carry an
            // arbitrary decimal here, so the verifier is the sole rejecter on the
            // package-decode path (bug-265 / PKG-08).
            "Scalar" if !value.contains('.') => {
                let invalid = match value.parse::<u64>() {
                    Ok(cp) => cp > 0x10_FFFF || (0xD800..=0xDFFF).contains(&cp),
                    Err(_) => true,
                };
                if invalid {
                    self.emit(
                        "TYPE_SCALAR_LITERAL_INVALID",
                        format!(
                            "Scalar literal `{value}` is not a Unicode scalar value (0..1114111, excluding surrogates 55296..57343)."
                        ),
                    );
                }
            }
            // Money is exact base-10: range and excess-precision are decided by the
            // exact converter, not an `f64` bound (plan-29-A §4.4, plan-29-B).
            "Money" => match crate::numeric::money_conversion_from_decimal(value) {
                Ok(converted) if converted.lost_precision => self.emit(
                    "TYPE_MONEY_LITERAL_PRECISION",
                    format!(
                        "Money literal `{value}` has more than 5 fractional digits; the value beyond the 5th is rounded away."
                    ),
                ),
                Ok(_) => {}
                Err(_) => self.emit(
                    "TYPE_MONEY_LITERAL_OVERFLOW",
                    format!("Numeric literal `{value}` is outside the Money range."),
                ),
            },
            _ => {}
        }
    }

    /// The underflow direction of the literal-range check for a `-<literal>`.
    pub(super) fn check_negated_const_literal(&self, type_: &str, value: &str) {
        match type_ {
            "Byte" if !value.contains('.') && value != "0" => {
                self.emit(
                    "TYPE_BYTE_LITERAL_UNDERFLOW",
                    format!("Integer literal `-{value}` is outside the Byte range 0..255."),
                );
            }
            "Integer" if !value.contains('.') => {
                if format!("-{value}").parse::<i64>().is_err() {
                    self.emit(
                        "TYPE_INTEGER_LITERAL_OVERFLOW",
                        format!("Integer literal `-{value}` is outside the Integer range."),
                    );
                }
            }
            // A negative codepoint is never a Unicode scalar value (only `-0`
            // would coincide with 0); reject the negated form outright.
            "Scalar" if !value.contains('.') && value != "0" => {
                self.emit(
                    "TYPE_SCALAR_LITERAL_INVALID",
                    format!(
                        "Scalar literal `-{value}` is not a Unicode scalar value (0..1114111, excluding surrogates 55296..57343)."
                    ),
                );
            }
            "Fixed" => {
                if let Ok(f) = value.parse::<f64>() {
                    if -f < -2147483648.0 {
                        self.emit(
                            "TYPE_FIXED_LITERAL_UNDERFLOW",
                            format!("Numeric literal `-{value}` is outside the Fixed range."),
                        );
                    }
                }
            }
            // The most-negative Money (`-92233720368547.75808`) has no
            // positive-magnitude literal, so the negated path checks the exact
            // converter on the signed text (plan-29-B §4.2).
            "Money" => match crate::numeric::money_conversion_from_decimal(&format!("-{value}")) {
                Ok(converted) if converted.lost_precision => self.emit(
                    "TYPE_MONEY_LITERAL_PRECISION",
                    format!(
                        "Money literal `-{value}` has more than 5 fractional digits; the value beyond the 5th is rounded away."
                    ),
                ),
                Ok(_) => {}
                Err(_) => self.emit(
                    "TYPE_MONEY_LITERAL_UNDERFLOW",
                    format!("Numeric literal `-{value}` is outside the Money range."),
                ),
            },
            "Float" => {
                if let Ok(f) = value.parse::<f64>() {
                    if !(-f).is_finite() {
                        self.emit(
                            "TYPE_FLOAT_LITERAL_UNDERFLOW",
                            format!("Numeric literal `-{value}` is outside the Float range."),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // ===========================================================================
    // 4. Member access + visibility
    // ===========================================================================

    /// Reject a `MemberAccess` whose target provably cannot carry the member: a
    /// primitive-typed target, or a known record that does not declare it.
    pub(super) fn check_member_access(
        &self,
        target: &IrValue,
        member: &str,
        locals: &HashMap<String, String>,
    ) {
        // `Enum.Member` selection: the target is the bare enum TYPE name (no
        // local shadows it), so the member must be one of the enum's declared
        // members — syntaxcheck's TYPE_UNKNOWN_ENUM_MEMBER.
        if let IrValue::Local(name) = target {
            if !locals.contains_key(name) {
                if let Some(members) = self.enums.get(name) {
                    if !members.contains(member) {
                        self.emit(
                            "TYPE_UNKNOWN_ENUM_MEMBER",
                            format!("ENUM `{name}` has no member `{member}`."),
                        );
                    }
                    return;
                }
            }
        }
        let Some(type_name) = self.infer_type(target, locals) else {
            return;
        };
        // Reading `.state` off a resource that declares none. Diagnosed here so it
        // names STATE, matching the write path's `TYPE_STATE_INVALID` (plan-52-C
        // §4). Without this the read degrades to `Unknown` and the error surfaces
        // wherever that Unknown lands — observed as a `TYPE_CALL_ARGUMENT_MISMATCH`
        // blaming `toString`'s argument types, which never mentions STATE and
        // points at the wrong line. It was always *rejected*; it just said so
        // unhelpfully.
        //
        // A bare `RES p AS File` parameter reaches this on purpose: bare means
        // "opaque" at a parameter, so `.state` is inaccessible through it even
        // though the caller's owner attached one (§15.5).
        //
        // Skipped inside a state ASSIGNMENT: `s.state = WITH s.state { … }` reads
        // `s.state` as part of the update, so this rule would fire on the
        // sub-expression and report the same line twice, alongside the assign
        // path's more precise "`s` has no STATE to assign". One error per
        // statement, from whichever rule knows the most.
        if member == "state"
            && !self.checking_state_assign.get()
            && crate::builtins::resource::state_type_name(&type_name).is_none()
            && self.is_resource_or_resource_union(resource_base_type(&type_name))
        {
            let base = resource_base_type(&type_name);
            self.emit(
                "TYPE_STATE_INVALID",
                format!(
                    "`{base}` here has no STATE to read; declare the resource with `STATE T`. Bare means \"no state\" on an owner, and \"opaque\" on a `RES` parameter — a bare parameter cannot read the state its caller attached."
                ),
            );
            return;
        }
        // The `t.result` field is removed; worker outcomes come only through
        // `thread::waitFor(t)` (syntaxcheck's TYPE_THREAD_RESULT_REMOVED).
        if resource_base_type(&type_name).starts_with("Thread") && member == "result" {
            self.emit(
                "TYPE_THREAD_RESULT_REMOVED",
                "Thread values have no `result` field; use `thread::waitFor(t)` to retrieve the worker outcome."
                    .to_string(),
            );
            return;
        }
        if PRIMITIVE_TYPES.contains(&type_name.as_str()) {
            self.emit(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                format!("field access requires a record value, got `{type_name}`."),
            );
            return;
        }
        // Only a record can be member-accessed. When the target resolves to a
        // record whose complete field set is known, the member must be present;
        // otherwise (collections, unions, unresolved includes, unknown types)
        // the access is left unchecked.
        if let Some(fields) = self.record_fields(&type_name) {
            if !fields.contains(member) {
                self.emit(
                    "TYPE_UNKNOWN_FIELD",
                    format!("record `{type_name}` has no member `{member}`."),
                );
            } else if self.hidden_from_here(&type_name, member) {
                self.emit(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    format!("Field `{type_name}::{member}` is not visible from this file."),
                );
            }
        }
    }

    /// Whether `member` of `type_name` is explicitly private and the current
    /// file is not the type's declaring file (syntaxcheck's `visible_from`).
    pub(super) fn hidden_from_here(&self, type_name: &str, member: &str) -> bool {
        if !self
            .private_fields
            .get(type_name)
            .is_some_and(|p| p.contains(member))
        {
            return false;
        }
        self.type_decl_info
            .get(type_name)
            .is_some_and(|(file, _)| !file.is_empty() && *file != *self.current_file.borrow())
    }

    // ===========================================================================
    // 5. Operand typing (binary, money, comparability, map keys)
    // ===========================================================================

    /// Reject a binary operator applied to operands whose types it cannot
    /// accept — the IR-level counterpart of `syntaxcheck`'s `infer_binary`
    /// operand rule (`TYPE_BINARY_OPERATOR_MISMATCH` / `TYPE_REQUIRES_COMPARABLE`).
    /// On decoded package IR this is a memory-safety gate: codegen selects the
    /// machine instruction from the operand *types*, so a crafted `String - Integer`
    /// would emit an integer subtract over a string pointer (pointer arithmetic
    /// on attacker data). Only rejects when both operand types are known and
    /// provably incompatible; `Unknown` is treated as any type (matching
    /// `is_numeric(Unknown) == true`), so no valid program is ever rejected.
    pub(super) fn check_binary_operands(
        &self,
        op: &str,
        left: &IrValue,
        right: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let (Some(lt), Some(rt)) = (
            self.infer_type(left, locals),
            self.infer_type(right, locals),
        ) else {
            return; // an operand type is unknown → skip (no false reject)
        };
        // Money is a *dimensioned* numeric: any operator with a Money operand
        // obeys the dimensional lattice and the Money-only comparison rule
        // (plan-29-A §4.2/§4.3), not the ordinary numeric acceptance. `Unknown`
        // on either side stays permissive (no false reject).
        if (lt == "Money" || rt == "Money") && lt != "Unknown" && rt != "Unknown" {
            self.check_money_operands(op, &lt, &rt);
            return;
        }
        // `Money` is included so `Money <op> Unknown` (the companion operand
        // could not be typed) stays permissive: the strict Money branch above
        // only fires when *both* sides are known, so an Unknown companion falls
        // through here and must not be rejected (module "Unknown stays
        // permissive" contract, :1834).
        let numeric = |t: &str| {
            matches!(
                t,
                "Integer" | "Byte" | "Float" | "Fixed" | "Money" | "Unknown"
            )
        };
        let string = |t: &str| matches!(t, "String" | "Unknown");
        let boolean = |t: &str| matches!(t, "Boolean" | "Unknown");
        // Scalar orders by codepoint value; non-numeric, and never orders against
        // String (plan-41-A). Both operands must be Scalar (Unknown permissive).
        let scalar = |t: &str| matches!(t, "Scalar" | "Unknown");
        let ok = match op {
            "AND" | "OR" | "XOR" => boolean(&lt) && boolean(&rt),
            "&" => string(&lt) && string(&rt),
            "<" | ">" | "<=" | ">=" => {
                (numeric(&lt) && numeric(&rt))
                    || (string(&lt) && string(&rt))
                    || (scalar(&lt) && scalar(&rt))
            }
            // Equality (`=`/`<>`): numeric pairs compare, otherwise both
            // operands must be compatible AND comparable. A crafted comparison
            // of non-comparable values (collections, functions, resources,
            // unions) would mislead codegen's comparison lowering.
            "=" | "<>" => {
                if numeric(&lt) && numeric(&rt) {
                    true
                } else if self.compatible(&lt, &rt) || self.compatible(&rt, &lt) {
                    self.is_comparable(&lt) && self.is_comparable(&rt)
                } else {
                    // Incompatible operands: an operator mismatch, not a
                    // comparability failure — reported below with the right id.
                    false
                }
            }
            // Everything else is arithmetic / bitwise: numeric operands only.
            _ => numeric(&lt) && numeric(&rt),
        };
        if !ok {
            if matches!(op, "=" | "<>") {
                // Compatible-but-not-comparable is a comparability failure;
                // incompatible operands are an operator mismatch.
                let rule = if self.compatible(&lt, &rt) || self.compatible(&rt, &lt) {
                    "TYPE_REQUIRES_COMPARABLE"
                } else {
                    "TYPE_BINARY_OPERATOR_MISMATCH"
                };
                self.emit(
                    rule,
                    format!(
                        "Operator `{op}` requires compatible comparable operands, got {lt} and {rt}."
                    ),
                );
                return;
            }
            let requirement = match op {
                "AND" | "OR" | "XOR" => "Boolean operands",
                "&" => "String operands",
                "<" | ">" | "<=" | ">=" => "numeric or String operands",
                _ => "numeric operands",
            };
            self.emit(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                format!("Operator `{op}` requires {requirement}, got {lt} and {rt}."),
            );
        }
    }

    /// Enforce the Money dimensional algebra for a binary operator that has at
    /// least one Money operand (plan-29-A §4.2/§4.3). Same-dimension add/subtract,
    /// scalar scaling, `M/M` ratio, `M MOD M`, and Money-only comparison are
    /// accepted; every other pairing emits `TYPE_MONEY_OPERATION_INVALID` with a
    /// message that explains *why*.
    pub(super) fn check_money_operands(&self, op: &str, lt: &str, rt: &str) {
        let l_money = lt == "Money";
        let r_money = rt == "Money";
        if matches!(op, "=" | "<>" | "<" | ">" | "<=" | ">=") {
            // Money compares only with Money (both operands, both directions).
            if l_money != r_money {
                self.emit(
                    "TYPE_MONEY_OPERATION_INVALID",
                    format!(
                        "Operator `{op}` requires both operands to be Money; got {lt} and {rt}. Compare a Money only with a Money (use `toMoney(...)` to convert)."
                    ),
                );
            }
            return;
        }
        if crate::numeric::money_result_type(op, l_money, r_money).is_some() {
            return;
        }
        // Craft an explanation for the specific invalid pairing.
        let reason = match op {
            "+" | "-" | "MOD" => {
                "requires both operands to be Money (a Money and a non-Money value cannot be combined)"
            }
            "*" if l_money && r_money => "cannot multiply two Money values (money² is not Money)",
            "/" if r_money && !l_money => {
                "cannot divide a non-Money value by a Money value"
            }
            "^" => "does not support exponentiation of a Money value",
            _ => "is not valid for Money operands",
        };
        self.emit(
            "TYPE_MONEY_OPERATION_INVALID",
            format!("Operator `{op}` {reason}; got {lt} and {rt}."),
        );
    }

    /// Whether a value of type `type_` can be compared for equality
    /// (`syntaxcheck::is_comparable`): primitives/enums yes; collections,
    /// functions, results, resources, and unions no; a record only if every
    /// field is comparable. `Unknown` is comparable (never a false rejection).
    pub(super) fn is_comparable(&self, type_: &str) -> bool {
        self.is_comparable_seen(resource_base_type(type_), &mut HashSet::new())
    }

    /// Every `Map OF K TO V` nested anywhere in `type_` must have a comparable
    /// key — `syntaxcheck`'s map-key arm of `TYPE_REQUIRES_COMPARABLE` (an
    /// incomparable key breaks the map's hash/equality contract at runtime).
    pub(super) fn check_map_key_comparable(&self, type_: &str) {
        let t = resource_base_type(type_);
        if let Some(inner) = t.strip_prefix("List OF ") {
            self.check_map_key_comparable(inner);
            return;
        }
        if let Some((key, value)) = parse_map(t) {
            // A resource/thread may never be a Map key (handles are not
            // comparable and ordinary collections cannot own them) —
            // syntaxcheck's TYPE_COLLECTION_OWNERSHIP_VIOLATION key arm.
            if !key.is_empty()
                && key != "Unknown"
                && self.contains_resource_or_thread(key, &mut HashSet::new())
            {
                self.emit(
                    "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
                    format!(
                        "Ordinary collections cannot store key values of type `{key}` because they contain a resource or thread handle."
                    ),
                );
            }
            if !key.is_empty() && key != "Unknown" && !self.is_comparable(key) {
                self.emit(
                    "TYPE_REQUIRES_COMPARABLE",
                    format!("Map key type requires a comparable type, got `{key}`."),
                );
            }
            self.check_map_key_comparable(key);
            self.check_map_key_comparable(value);
        }
    }

    pub(super) fn is_comparable_seen(&self, type_: &str, seen: &mut HashSet<String>) -> bool {
        match type_ {
            "Boolean" | "Byte" | "Error" | "ErrorLoc" | "Fixed" | "Float" | "Integer" | "Money"
            | "Nothing" | "Scalar" | "String" | "Unknown" => return true,
            _ => {}
        }
        if type_.starts_with("List OF ")
            || type_.starts_with("Map OF ")
            || type_.starts_with("Result OF ")
            || type_.starts_with("FUNC(")
            || type_.starts_with("Thread ")
            || type_.starts_with("ThreadWorker ")
        {
            return false;
        }
        if is_resource_name(type_) {
            return false;
        }
        if self.unions.contains_key(type_) {
            return false;
        }
        if self.enums.contains_key(type_) {
            return true;
        }
        if !seen.insert(type_.to_string()) {
            return false; // a cycle → not a base case
        }
        if let Some(fields) = self.field_types.get(type_) {
            let all = fields
                .values()
                .all(|ft| self.is_comparable_seen(resource_base_type(ft), seen));
            seen.remove(type_);
            return all;
        }
        // Unknown user type — permissive (no false rejection).
        true
    }

    // ===========================================================================
}
