use super::*;

impl TypeEnv {
    // 6. Type declarations, union includes, record cycles
    // ===========================================================================

    /// Structural well-formedness of the type table (`syntaxcheck`'s
    /// `check_type_decl`), checkable directly on the IR. On decoded package IR
    /// these guard codegen's layout and drop assumptions: a record that owns a
    /// resource field, a union mixing data and resource variants (tag-dependent
    /// copyability / drop dispatch), or a record with no base case (infinite
    /// size) would all mislead the layout/drop lowering. Reported at the type
    /// declaration line; the file is unset (a decoded package has no source).
    pub(super) fn check_type_declarations(&self, project: &IrProject) {
        for ty in &project.types {
            self.current_file.replace(ty.file.clone());
            self.current_line.set(ty.loc.line);
            match ty.kind.as_str() {
                "type" | "record" => {
                    for field in &ty.fields {
                        self.current_line.set(field.loc.line);
                        self.check_map_key_comparable(&field.type_);
                        self.current_line.set(ty.loc.line);
                        if is_resource_name(resource_base_type(&field.type_)) {
                            self.current_line.set(field.loc.line);
                            self.emit(
                                "TYPE_RESOURCE_FIELD_FORBIDDEN",
                                format!(
                                    "Record `{}` field `{}` is resource `{}`; records cannot own resources.",
                                    ty.name, field.name, field.type_
                                ),
                            );
                            self.current_line.set(ty.loc.line);
                        }
                    }
                    if self.record_field_cycle(&ty.name, &ty.name, &mut HashSet::new()) {
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
                        if !self.unions.contains_key(include)
                            && (self.records.contains_key(include)
                                || self.enums.contains_key(include))
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
                        if self.unions.contains_key(&variant.name)
                            || self.enums.contains_key(&variant.name)
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
    /// union transitively (cycle-guarded). Mirrors `syntaxcheck`'s
    /// `expanded_union_variants`, but names only — dup detection needs no fields.
    pub(super) fn expanded_union_variant_names(
        &self,
        union_name: &str,
        visiting: &mut HashSet<String>,
    ) -> Vec<String> {
        if !visiting.insert(union_name.to_string()) {
            return Vec::new();
        }
        let mut names = Vec::new();
        if let Some(info) = self.unions.get(union_name) {
            for include in &info.includes {
                names.extend(self.expanded_union_variant_names(include, visiting));
            }
            names.extend(info.variants.iter().cloned());
        }
        visiting.remove(union_name);
        names
    }

    /// `syntaxcheck::report_expanded_union_member_conflicts` on the IR: a union
    /// member may not be provided by two different includes, nor by both an
    /// include and a local declaration. On decoded package IR a duplicated
    /// variant is an ambiguous tag → mis-dispatch, so this must run here too.
    pub(super) fn check_union_include_conflicts(&self, ty: &IrType) {
        let Some(info) = self.unions.get(&ty.name) else {
            return;
        };
        // A member provided by two distinct includes.
        let mut included_members: HashMap<String, String> = HashMap::new();
        for include in &info.includes {
            let mut visiting = HashSet::new();
            for name in self.expanded_union_variant_names(include, &mut visiting) {
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
        record: &str,
        target: &str,
        seen: &mut HashSet<String>,
    ) -> bool {
        if !seen.insert(record.to_string()) {
            return false;
        }
        let Some(fields) = self.field_types.get(record) else {
            return false;
        };
        for field_type in fields.values() {
            // Only *direct* record fields propagate the cycle; a List/Map/Union
            // field is a legitimate base-case indirection.
            let base = resource_base_type(field_type);
            if base == target {
                return true;
            }
            if self.records.contains_key(base) && self.record_field_cycle(base, target, seen) {
                return true;
            }
        }
        false
    }

    // ===========================================================================
}
