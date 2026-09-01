use super::*;

impl TypeEnv {
    // 6. Type declarations, union includes, record cycles
    // ===========================================================================

    /// Structural well-formedness of the type table (the former source checker's
    /// `check_type_decl`), checkable directly on the IR. On decoded package IR
    /// these guard codegen's layout and drop assumptions: a union mixing data and
    /// resource variants (tag-dependent copyability / drop dispatch) or a record
    /// with no base case (infinite size) would mislead the layout/drop lowering.
    /// Reported at the type declaration line; the file is unset (a decoded
    /// package has no source).
    ///
    /// plan-114-B/D: the resource-field ban is **gone**. Its justification — that
    /// such a field would mislead the layout and drop lowering — was retired by
    /// letter B, which lays a resource field out as an ordinary 8-byte handle
    /// slot and relaxed the NIR backstop accordingly. Letter D then replaced
    /// `TYPE_RESOURCE_FIELD_FORBIDDEN` (now reserved, never emitted) with the two
    /// `RES`-marker rules a collection element already takes, routed through the
    /// same emitter so the positions cannot drift.
    pub(super) fn check_type_declarations(&self, project: &IrProject) {
        for ty in &project.types {
            self.current_file.replace(ty.file.clone());
            self.current_line.set(ty.loc.line);
            match ty.kind.as_str() {
                "type" | "record" => {
                    for field in &ty.fields {
                        self.current_line.set(field.loc.line);
                        self.check_map_key_comparable(&field.type_);
                        self.check_thread_sendability(&field.type_);
                        // plan-114-D: a record field is governed by the same two
                        // `RES`-marker rules a collection element is — a bare
                        // resource field needs the marker
                        // (`TYPE_RESOURCE_REQUIRES_RES`), and the marker may only
                        // mark a resource (`TYPE_RES_REQUIRES_RESOURCE`). This
                        // replaces `TYPE_RESOURCE_FIELD_FORBIDDEN`, which refused
                        // the field outright.
                        //
                        // It routes through the SAME emitter the element uses, so
                        // the two positions cannot drift; only the subject and the
                        // suggested spelling differ.
                        self.current_line.set(field.loc.line);
                        self.res_axis_slot(
                            &field.type_,
                            &format!("Record `{}` field `{}`", ty.name, field.name),
                            "`handle AS RES fs::File`",
                        );
                        // plan-114-E: a `STATE T` clause on a `RES` field is
                        // gated exactly as it is on a binding (§15.5) — `T` must
                        // be a copyable, defaultable DATA type.
                        //
                        // This needed routing, not just verifying: the binding
                        // check lives in `ops.rs` behind an `explicit_type` gate
                        // and never saw a field, so before this a field could
                        // declare `STATE fs::File` — a resource as its own state
                        // payload — and nothing rejected it, all the way to
                        // codegen. A binding with the identical clause has always
                        // been refused.
                        if let Some(state_type) = strip_res(&field.type_).state() {
                            if !self.is_defaultable(&state_type, &mut HashSet::new()) {
                                self.emit(
                                    "TYPE_STATE_INVALID",
                                    format!(
                                        "Record `{}` field `{}` STATE type `{state_type}` must be a copyable, defaultable data type.",
                                        ty.name, field.name
                                    ),
                                );
                            }
                        }
                        self.current_line.set(ty.loc.line);
                    }
                    let record = ParameterType::declared(&ty.name);
                    if self.record_field_cycle(&record, &record, &mut HashSet::new()) {
                        self.emit(
                            "TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION",
                            format!(
                                "Record `{}` refers to itself without passing through a List, Map, or UNION; such a record has no base case and cannot be constructed.",
                                ty.name
                            ),
                        );
                    }
                }
                "union" => {
                    // `INCLUDES` may only name other unions. A name that is a
                    // known non-union type (record or enum) is a malformed
                    // include. (Undeclared names are a different, resolve-time
                    // rule, so only reject names the IR positively knows.)
                    for include in &ty.includes {
                        // An `INCLUDES` entry names a declared type; the tables
                        // are keyed by that nominal (plan-111-B).
                        let include_type = ParameterType::declared(include);
                        if !self.unions.contains_key(&include_type)
                            && (self.records.contains_key(&include_type)
                                || self.enums.contains_key(&include_type))
                        {
                            self.emit(
                                "TYPE_UNION_INCLUDE_REQUIRES_UNION",
                                format!(
                                    "UNION `{}` includes `{}`, but `{}` is not a UNION.",
                                    ty.name, include, include
                                ),
                            );
                        }
                    }
                    // Each named member must be a concrete TYPE (record). A
                    // member that is itself a union or an enum is not a concrete
                    // type. (Records-registered variant names are fine; only a
                    // name that is *also* a declared union/enum is rejected.)
                    for variant in &ty.variants {
                        let variant_type = ParameterType::declared(&variant.name);
                        if self.unions.contains_key(&variant_type)
                            || self.enums.contains_key(&variant_type)
                        {
                            self.current_line.set(variant.loc.line);
                            self.emit(
                                "TYPE_UNION_MEMBER_REQUIRES_TYPE",
                                format!(
                                    "UNION `{}` member `{}` must be a concrete TYPE.",
                                    ty.name, variant.name
                                ),
                            );
                            self.current_line.set(ty.loc.line);
                        }
                    }
                    self.check_union_include_conflicts(ty);
                    self.current_line.set(ty.loc.line);
                    let resource_variants = ty
                        .variants
                        .iter()
                        .filter(|v| is_resource_name(&v.name))
                        .count();
                    if resource_variants > 0 && resource_variants < ty.variants.len() {
                        self.emit(
                            "TYPE_MIXED_RESOURCE_UNION",
                            format!(
                                "UNION `{}` mixes data and resource variants; a union must be all-data or all-resource.",
                                ty.name
                            ),
                        );
                    }
                }
                "enum" if ty.members.is_empty() => {
                    self.emit(
                        "TYPE_ENUM_REQUIRES_MEMBER",
                        format!("ENUM `{}` must declare at least one member.", ty.name),
                    );
                }
                _ => {}
            }
        }
    }

    /// The full member-name set of `union_name`, expanding every `INCLUDES`d
    /// union transitively (cycle-guarded). Mirrors the former source checker's
    /// `expanded_union_variants`, but names only — dup detection needs no fields.
    pub(super) fn expanded_union_variant_names(
        &self,
        union_type: &ParameterType,
        visiting: &mut HashSet<ParameterType>,
    ) -> Vec<String> {
        if !visiting.insert(union_type.clone()) {
            return Vec::new();
        }
        let mut names = Vec::new();
        if let Some(info) = self.unions.get(union_type) {
            for include in &info.includes {
                names.extend(
                    self.expanded_union_variant_names(&ParameterType::declared(include), visiting),
                );
            }
            names.extend(info.variants.iter().map(|v| v.name().into_owned()));
        }
        visiting.remove(union_type);
        names
    }

    /// the former source checker's `report_expanded_union_member_conflicts` on the IR: a union
    /// member may not be provided by two different includes, nor by both an
    /// include and a local declaration. On decoded package IR a duplicated
    /// variant is an ambiguous tag → mis-dispatch, so this must run here too.
    pub(super) fn check_union_include_conflicts(&self, ty: &IrType) {
        let Some(info) = self.unions.get(&ParameterType::declared(&ty.name)) else {
            return;
        };
        // A member provided by two distinct includes.
        let mut included_members: HashMap<String, String> = HashMap::new();
        for include in &info.includes {
            let mut visiting = HashSet::new();
            for name in
                self.expanded_union_variant_names(&ParameterType::declared(include), &mut visiting)
            {
                if let Some(previous) = included_members.insert(name.clone(), include.clone()) {
                    self.current_line.set(ty.loc.line);
                    self.emit(
                        "TYPE_DUPLICATE_VARIANT",
                        format!(
                            "Member type `{}` in UNION `{}` is provided by both included UNION `{}` and included UNION `{}`.",
                            name, ty.name, previous, include
                        ),
                    );
                }
            }
        }
        // A local variant that collides with an included member.
        for variant in &ty.variants {
            if let Some(include) = included_members.get(&variant.name) {
                self.current_line.set(variant.loc.line);
                self.emit(
                    "TYPE_DUPLICATE_VARIANT",
                    format!(
                        "Member type `{}` in UNION `{}` conflicts with a member included from UNION `{}`.",
                        variant.name, ty.name, include
                    ),
                );
            }
        }
    }

    /// Whether `record` reaches `target` through a chain of direct record-typed
    /// fields (no List/Map/Union indirection) — i.e. an infinitely-sized record.
    pub(super) fn record_field_cycle(
        &self,
        record: &ParameterType,
        target: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> bool {
        if !seen.insert(record.clone()) {
            return false;
        }
        let Some(fields) = self.field_types.get(record) else {
            return false;
        };
        for field_type in fields.values() {
            // Only *direct* record fields propagate the cycle; a List/Map/Union
            // field is a legitimate base-case indirection.
            let base = resource_base_type(field_type);
            if base == *target {
                return true;
            }
            if self.records.contains_key(&base) && self.record_field_cycle(&base, target, seen) {
                return true;
            }
        }
        false
    }

    // ===========================================================================
}
