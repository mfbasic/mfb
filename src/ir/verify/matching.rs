use super::*;

impl TypeEnv {
    // 9. Match exhaustiveness + patterns
    // ===========================================================================

    /// Reject a `MATCH` on an enum or union that neither covers every
    /// member/variant nor has an unguarded catch-all (`syntaxcheck`'s
    /// `TYPE_MATCH_NOT_EXHAUSTIVE`). On decoded package IR this is a
    /// memory-safety gate: a non-exhaustive match falls through with no arm
    /// selected, leaving a typed value uninitialized. Only checked when the
    /// scrutinee resolves to a known enum/union with a complete member set
    /// (Result-matches lower to a Boolean flag and are skipped); guarded cases
    /// do not count toward coverage, matching the source rule.
    pub(super) fn check_match_exhaustive(
        &self,
        value: &IrValue,
        cases: &[super::super::IrMatchCase],
        locals: &HashMap<String, String>,
    ) {
        let Some(ty) = self.infer_type(value, locals) else {
            return;
        };
        let ty = resource_base_type(&ty).to_string();
        // A Result scrutinee's CASE Ok/Error arms are rejected by
        // TYPE_RESULT_NOT_MATCHABLE; suppress the secondary exhaustiveness
        // cascade like syntaxcheck does. Unknown types are skipped as always.
        if ty.is_empty() || ty == "Unknown" || ty == "Result" || ty.starts_with("Result OF ") {
            return;
        }
        // The complete member/variant set, and whether it is a union (for the
        // diagnostic wording). Any other *known* type is an open type: only an
        // unguarded CASE ELSE can make its MATCH exhaustive.
        let (all, is_union) = if let Some(variants) = self.union_variants(&ty) {
            (variants, true)
        } else if let Some(members) = self.enums.get(&ty) {
            (members.clone(), false)
        } else {
            if !cases.iter().any(|case| {
                case.guard.is_none() && matches!(case.pattern, super::super::IrMatchPattern::Else)
            }) {
                self.emit(
                    "TYPE_MATCH_NOT_EXHAUSTIVE",
                    format!("MATCH on open type {ty} requires an unguarded CASE ELSE."),
                );
            }
            return;
        };
        let pattern_name = |v: &IrValue| -> Option<String> {
            match v {
                IrValue::Local(name) => Some(name.clone()),
                IrValue::MemberAccess { member, .. } => Some(member.clone()),
                _ => None,
            }
        };
        let mut covered: HashSet<String> = HashSet::new();
        for case in cases {
            if case.guard.is_some() {
                continue; // a guarded arm may not fire → does not cover
            }
            match &case.pattern {
                super::super::IrMatchPattern::Else => return, // unguarded catch-all
                super::super::IrMatchPattern::Value(v) => {
                    if let Some(name) = pattern_name(v) {
                        covered.insert(name);
                    }
                }
                super::super::IrMatchPattern::OneOf(vs) => {
                    for v in vs {
                        if let Some(name) = pattern_name(v) {
                            covered.insert(name);
                        }
                    }
                }
            }
        }
        if all.difference(&covered).next().is_none() {
            return;
        }
        // Missing-member lists mirror syntaxcheck's wording exactly: unions list
        // the uncovered variants in declaration order; enums list sorted
        // `Type.member` names.
        let missing = if is_union {
            let mut ordered: Vec<String> = self
                .unions
                .get(&ty)
                .map(|info| {
                    info.variant_order
                        .iter()
                        .filter(|v| !covered.contains(*v))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            // Variants reached through INCLUDES have no declaration slot here;
            // append them sorted so the list is complete and deterministic.
            let mut extra: Vec<String> = all
                .difference(&covered)
                .filter(|v| !ordered.contains(v))
                .cloned()
                .collect();
            extra.sort();
            ordered.extend(extra);
            ordered.join(", ")
        } else {
            let mut members: Vec<String> = all
                .difference(&covered)
                .map(|m| format!("{ty}.{m}"))
                .collect();
            members.sort();
            members.join(", ")
        };
        let detail = if is_union {
            format!(
                "MATCH on UNION `{ty}` does not cover {missing}; add unguarded CASE arms or CASE ELSE."
            )
        } else {
            format!(
                "MATCH on enum `{ty}` does not cover {missing}; add unguarded CASE arms or CASE ELSE."
            )
        };
        self.emit("TYPE_MATCH_NOT_EXHAUSTIVE", detail);
    }

    /// `syntaxcheck`'s `TYPE_MATCH_PATTERN_MISMATCH` on the IR: a CASE pattern
    /// must fit the scrutinee — a union CASE must name one of the union's
    /// variants, a type-named CASE requires a union scrutinee, and a literal
    /// pattern's type must be compatible with the scrutinee type. Unknown
    /// scrutinee or pattern types are skipped (sound skip-if-unknown).
    pub(super) fn check_match_patterns(
        &self,
        value: &IrValue,
        cases: &[super::super::IrMatchCase],
        locals: &HashMap<String, String>,
    ) {
        let Some(scrutinee) = self.infer_type(value, locals) else {
            return;
        };
        let scrutinee = resource_base_type(&scrutinee).to_string();
        if scrutinee.is_empty() || scrutinee == "Unknown" {
            return;
        }
        let union_variants = self.union_variants(&scrutinee);
        let check_pattern = |v: &IrValue| {
            // `Result` is internal: `CASE Ok`/`CASE Error` are never valid
            // match arms (syntaxcheck's TYPE_RESULT_NOT_MATCHABLE). Only fires
            // when the name is not a real variant of the scrutinee's union.
            if let IrValue::Local(n) | IrValue::MemberAccess { member: n, .. } = v {
                if matches!(n.as_str(), "Ok" | "Error" | "Err")
                    && !union_variants
                        .as_ref()
                        .is_some_and(|vs| vs.contains(n.as_str()))
                {
                    self.emit(
                        "TYPE_RESULT_NOT_MATCHABLE",
                        format!(
                            "`CASE {n}` is not a valid match arm; handle failure with an inline `TRAP` instead."
                        ),
                    );
                    return;
                }
            }
            // A pattern that names a declared type is a union-variant arm.
            let type_name = match v {
                IrValue::Local(name) => Some(name),
                IrValue::MemberAccess { member, .. } => Some(member),
                _ => None,
            }
            .filter(|n| {
                self.records.contains_key(n.as_str())
                    || self.unions.contains_key(n.as_str())
                    || self.enums.contains_key(n.as_str())
            });
            if let Some(type_name) = type_name {
                match &union_variants {
                    Some(variants) => {
                        if !variants.contains(type_name.as_str()) {
                            self.emit(
                                "TYPE_MATCH_PATTERN_MISMATCH",
                                format!(
                                    "CASE `{type_name}` is not a member of UNION `{scrutinee}`."
                                ),
                            );
                        }
                    }
                    None => {
                        // An enum scrutinee's member arms share member names
                        // with no type; a declared-type CASE against any
                        // non-union scrutinee is malformed.
                        self.emit(
                            "TYPE_MATCH_PATTERN_MISMATCH",
                            format!("CASE `{type_name}` requires a UNION value, got {scrutinee}."),
                        );
                    }
                }
                return;
            }
            // A literal (or expression) pattern: its type must fit the
            // scrutinee. Enum member arms are Local names with no local type
            // (infer_type -> None), so they fall through harmlessly here.
            if let Some(pattern_type) = self.infer_type(v, locals) {
                if !self.expression_compatible(&scrutinee, &pattern_type, v) {
                    self.emit(
                        "TYPE_MATCH_PATTERN_MISMATCH",
                        format!("CASE pattern has type {pattern_type}, expected {scrutinee}."),
                    );
                }
            }
        };
        for case in cases {
            self.current_line.set(case.loc.line);
            match &case.pattern {
                super::super::IrMatchPattern::Else => {}
                super::super::IrMatchPattern::Value(v) => check_pattern(v),
                super::super::IrMatchPattern::OneOf(vs) => {
                    for v in vs {
                        check_pattern(v);
                    }
                }
            }
        }
    }

    // ===========================================================================
}
