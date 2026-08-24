// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::binary_repr;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::function::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
impl NativeCodePlan {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.target.is_empty() {
            return Err("native code plan target must not be empty".to_string());
        }
        if self.arch.is_empty() {
            return Err("native code plan arch must not be empty".to_string());
        }
        if self.project.is_empty() {
            return Err("native code plan project name must not be empty".to_string());
        }
        if self.functions.is_empty() {
            return Err("native code plan requires at least one function".to_string());
        }
        if let Some(entry_symbol) = &self.entry_symbol {
            if !self
                .functions
                .iter()
                .any(|function| &function.symbol == entry_symbol)
            {
                return Err(format!(
                    "native code plan entry symbol '{entry_symbol}' does not resolve"
                ));
            }
        }
        let defined_symbols = self
            .functions
            .iter()
            .map(|function| function.symbol.clone())
            .collect::<Vec<_>>();
        let imported_symbols = self
            .imports
            .iter()
            .map(|import| import.symbol.clone())
            .collect::<Vec<_>>();
        for import in &self.imports {
            if import.library.is_empty() || import.symbol.is_empty() {
                return Err("native code plan contains an incomplete import".to_string());
            }
        }
        let data_symbols = self
            .data_objects
            .iter()
            .map(|object| object.symbol.clone())
            .collect::<Vec<_>>();
        for object in &self.data_objects {
            if object.symbol.is_empty() || object.kind.is_empty() || object.layout.is_empty() {
                return Err("native code plan contains an incomplete data object".to_string());
            }
            if object.align == 0 || object.size == 0 {
                return Err(format!(
                    "native code data object '{}' must have nonzero size and alignment",
                    object.symbol
                ));
            }
        }
        for function in &self.functions {
            function.validate(&defined_symbols, &imported_symbols, &data_symbols)?;
        }
        Ok(())
    }
}

impl CodeFunction {
    fn validate(
        &self,
        defined_symbols: &[String],
        imported_symbols: &[String],
        data_symbols: &[String],
    ) -> Result<(), String> {
        if self.name.is_empty() || self.symbol.is_empty() {
            return Err("native code function name and symbol must not be empty".to_string());
        }
        if self.instructions.is_empty() {
            return Err(format!(
                "native code function '{}' has no instructions",
                self.name
            ));
        }
        if !self
            .instructions
            .iter()
            .any(|instruction| instruction.op == CodeOp::Ret)
        {
            return Err(format!(
                "native code function '{}' has no return instruction",
                self.name
            ));
        }
        for relocation in &self.relocations {
            if relocation.from != self.symbol {
                return Err(format!(
                    "native code relocation source '{}' does not match function '{}'",
                    relocation.from, self.symbol
                ));
            }
            match relocation.binding.as_str() {
                "internal" => {
                    if !defined_symbols.contains(&relocation.to) {
                        return Err(format!(
                            "native code internal relocation target '{}' is not defined",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_some() {
                        return Err(format!(
                            "native code internal relocation '{}' must not name a library",
                            relocation.to
                        ));
                    }
                }
                "external" => {
                    if !imported_symbols.contains(&relocation.to) {
                        return Err(format!(
                            "native code external relocation target '{}' is not imported",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_none() {
                        return Err(format!(
                            "native code external relocation '{}' must name a library",
                            relocation.to
                        ));
                    }
                }
                "data" => {
                    if !data_symbols.contains(&relocation.to)
                        && !defined_symbols.contains(&relocation.to)
                    {
                        return Err(format!(
                            "native code data relocation target '{}' is not a data object or defined symbol",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_some() {
                        return Err(format!(
                            "native code data relocation '{}' must not name a library",
                            relocation.to
                        ));
                    }
                }
                other => {
                    return Err(format!(
                        "native code relocation '{}' has invalid binding '{}'",
                        relocation.to, other
                    ));
                }
            }
        }
        for instruction in &self.instructions {
            instruction.validate()?;
        }
        // bug-300 E9: `CodeInstruction::validate` only checks that a branch HAS a
        // `target` field, never that the label it names exists. A codegen bug
        // emitting a branch to an undefined label therefore passed
        // `plan.validate()` and was caught only much later by the encoder ("branch
        // target label does not resolve"). Resolving it here fails at the layer
        // that owns the invariant, with the function named.
        // Borrow each label name (a `Raw` string lends its `&str`); a `Raw` label
        // never allocates for the set, and the membership test below borrows too.
        let defined_labels = self
            .instructions
            .iter()
            .filter(|instruction| instruction.op == CodeOp::Label)
            .filter_map(|instruction| instruction.operand("name").map(|n| n.rendered()))
            .collect::<std::collections::HashSet<std::borrow::Cow<'_, str>>>();
        for instruction in &self.instructions {
            // Only label-targeting branches: `bl`/`blr` target a symbol (covered by
            // the relocation checks above) and `branch_self` takes no target.
            if !matches!(
                instruction.op,
                CodeOp::Branch
                    | CodeOp::BranchEq
                    | CodeOp::BranchNe
                    | CodeOp::BranchGe
                    | CodeOp::BranchGt
                    | CodeOp::BranchLe
                    | CodeOp::BranchLt
                    | CodeOp::BranchHi
                    | CodeOp::BranchLo
                    | CodeOp::BranchLs
                    | CodeOp::BranchMi
                    | CodeOp::BranchVc
                    | CodeOp::BranchVs
            ) {
                continue;
            }
            if let Some(target) = instruction.operand("target") {
                let target = target.rendered();
                if !defined_labels.contains(target.as_ref()) {
                    return Err(format!(
                        "native code function '{}' branches to label '{target}', which it \
                         does not define",
                        self.name
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The native layout gives each union variant one context-free discriminant word
/// (`union_variant_tags`, keyed by variant name), so a union value carries the same
/// tag no matter which union it is viewed through — that is what lets a narrower
/// union value flow into a wider including union without re-tagging, and lets
/// `MATCH` compare the stored tag against a per-variant constant. The tag is
/// assigned **globally-canonically** (sorted variant name) by
/// `recompute_canonical_variant_tags`, independent of a variant's position within
/// any including union, so a variant included at divergent positions in two unions
/// dispatches consistently (bug-80; replaced the earlier positional scheme + its
/// `check_union_variant_tag` rejection).
impl TypeModel {
    pub(crate) fn empty() -> Self {
        Self {
            enum_members: HashMap::new(),
            record_fields: HashMap::new(),
            union_names: HashSet::new(),
            union_variants: HashMap::new(),
            union_variant_unions: HashMap::new(),
            union_variant_tags: HashMap::new(),
            union_variant_fields: HashMap::new(),
            resource_names: HashSet::new(),
            resource_closers: HashMap::new(),
        }
    }

    pub(crate) fn from_module(module: &NirModule) -> Result<Self, String> {
        let mut enum_members = HashMap::new();
        let mut record_fields = HashMap::new();
        let mut union_names = HashSet::new();
        let mut union_variants = HashMap::new();
        let mut union_variant_unions = HashMap::<String, HashSet<String>>::new();
        // bug-80: variant tags are assigned globally-canonically (keyed by variant
        // name, independent of position within any including union) by
        // recompute_canonical_variant_tags at the end, so a variant included at
        // divergent positions in two unions dispatches consistently. MATCH lowers to
        // `cmp tag, <value>` compares (not a dense jump table), so a sparse/reordered
        // tag space is fine.
        let union_variant_tags = HashMap::new();
        let mut union_variant_fields = HashMap::new();
        let mut resource_names = HashSet::new();
        for type_ in &module.types {
            match type_.kind.as_str() {
                "type" | "record" => {
                    record_fields.insert(
                        type_.name.clone(),
                        type_
                            .fields
                            .iter()
                            .map(|field| (field.name.clone(), field.type_.clone()))
                            .collect(),
                    );
                }
                "enum" => {
                    for (index, member) in type_.members.iter().enumerate() {
                        enum_members.insert((type_.name.clone(), member.name.clone()), index);
                    }
                }
                "union" => {
                    union_names.insert(type_.name.clone());
                    for variant in expanded_nir_union_variants(module, &type_.name).iter() {
                        union_variants
                            .entry(variant.name.clone())
                            .or_insert_with(|| type_.name.clone());
                        union_variant_unions
                            .entry(variant.name.clone())
                            .or_default()
                            .insert(type_.name.clone());
                        union_variant_fields.insert(
                            variant.name.clone(),
                            variant
                                .fields
                                .iter()
                                .map(|field| (field.name.clone(), field.type_.clone()))
                                .collect(),
                        );
                    }
                }
                "resource" => {
                    resource_names.insert(type_.name.clone());
                }
                other => {
                    return Err(format!(
                        "native code plan does not know type kind '{other}'"
                    ));
                }
            }
        }
        // A project that declares its own `RESOURCE R CLOSE BY …` alongside a
        // `LINK` block carries the resource only on the link functions that
        // produce it — `module.types` has no entry for it — so the names are
        // derived from there as well as from the `"resource"` kind above.
        for function in &module.link_functions {
            if function.return_resource {
                resource_names.insert(
                    crate::codegen::resource::base_resource_name(&function.return_type).to_string(),
                );
            }
        }
        // `Error` and `ErrorLoc` are read-only compiler/runtime records laid out
        // as ordinary 3-field records so construction, field access, copying, and
        // cleanup reuse the generic record machinery.
        record_fields.insert(
            "Error".to_string(),
            vec![
                ("code".to_string(), ParameterType::Integer),
                ("message".to_string(), ParameterType::String),
                ("source".to_string(), ParameterType::named("ErrorLoc")),
            ],
        );
        record_fields.insert(
            "ErrorLoc".to_string(),
            vec![
                ("filename".to_string(), ParameterType::String),
                ("line".to_string(), ParameterType::Integer),
                ("char".to_string(), ParameterType::Integer),
            ],
        );
        // plan-89-A/B: `AttributedString` is an opaque built-in laid out internally
        // as an ordinary 2-field record — a visible `text` String plus a `spans`
        // attribute overlay. Modeling it as a record lets construction
        // (`astrings::fromString`), value-semantic copy, scope-drop, and defaulting
        // all reuse the generic record machinery. These fields are codegen-internal
        // only — the frontend exposes NO user-visible fields (opacity).
        //
        // The overlay element `AttrSpan` (plan-89-B) is a codegen-internal flat
        // record: an inclusive `[start,end]` scalar range, an insertion `seq` for
        // the higher-start-wins tie-break, and a flat encoding of one attribute
        // (`class` 0=flag/1=text/2=number, the enum-member ordinal, plus the String
        // and Integer payloads). Registered UNCONDITIONALLY so `AttributedString`'s
        // layout is fully resolvable even in a program that never imports `astrings`
        // (a defaulted/parameter `AttributedString` still copies and drops). The
        // companion declares a matching `AttrSpan` so the `.mfb` bridge can read and
        // build spans; the two must stay field-identical.
        record_fields.insert(
            "AttrSpan".to_string(),
            vec![
                ("start".to_string(), ParameterType::Integer),
                // `last` (not `end`): `end` is a reserved keyword and cannot follow
                // `.` in the companion's member access. Field-identical to the
                // companion's `AttrSpan`.
                ("last".to_string(), ParameterType::Integer),
                ("seq".to_string(), ParameterType::Integer),
                ("class".to_string(), ParameterType::Integer),
                ("member".to_string(), ParameterType::Integer),
                ("text".to_string(), ParameterType::String),
                ("number".to_string(), ParameterType::Integer),
            ],
        );
        record_fields.insert(
            "AttributedString".to_string(),
            vec![
                ("text".to_string(), ParameterType::String),
                (
                    "spans".to_string(),
                    ParameterType::list_of(ParameterType::named("AttrSpan")),
                ),
            ],
        );
        // bug-374: record each user-declared resource's `CLOSE BY` op so
        // scope-drop can call it. Stored as the declared *name*, not a resolved
        // symbol: `resource_cleanup_symbol` resolves it through
        // `function_symbols`, the same table an explicit `sql::close(db)` call
        // goes through, so both spellings the name can take resolve alike — the
        // dotted `alias.func`, and the bare alias a re-exported close op is
        // serialized as (`EXPORT FUNC close AS sqliteLink::close`, which is what
        // `bindings/sqlite3` does). Matching `link_functions` on the dotted form
        // alone would silently miss every re-exported closer.
        let resource_closers = module
            .native_resources
            .iter()
            .map(|resource| (resource.name.clone(), resource.close_function.clone()))
            .collect();
        let mut model = Self {
            enum_members,
            record_fields,
            union_names,
            union_variants,
            union_variant_unions,
            union_variant_tags,
            union_variant_fields,
            resource_names,
            resource_closers,
        };
        // Assign canonical variant tags over this module's unions (bug-80). When
        // packages are also present, from_module_and_packages re-derives them over
        // the combined set.
        model.recompute_canonical_variant_tags();
        Ok(model)
    }

    pub(crate) fn from_module_and_packages(
        module: &NirModule,
        packages: &[PathBuf],
    ) -> Result<Self, String> {
        let mut model = Self::from_module(module)?;
        for package in packages {
            // A native `LINK` resource is exported as a zero-field opaque type for
            // naming, but its runtime value is a raw `CPtr` scalar handle — never a
            // record. Registering it as a record would make the backend copy it by
            // value on bind/return (an empty copy that loses the handle), so skip
            // native resource type exports and let them default to 8-byte scalars
            // (plan-linker.md §12, plan-link-update.md §10).
            let exported_resources: Vec<_> = binary_repr::read_package_resources(package)?
                .into_iter()
                .filter(|resource| resource.native)
                .collect();
            let native_resources: HashSet<String> = exported_resources
                .iter()
                .map(|resource| resource.type_name.clone())
                .collect();
            // An imported binding's resource is still a resource here (bug-372):
            // it is skipped as a *record* above, but codegen must recognize the
            // name to give it a closed-resource default on an inline `TRAP`'s
            // error path.
            model
                .resource_names
                .extend(native_resources.iter().cloned());
            // bug-374: an imported binding's resource drops at scope exit in the
            // importing program too, but a decoded package carries no
            // `native_resources` (`ir/binary.rs` drops them by contract), so the
            // close op comes from the package's RESOURCE_TABLE instead.
            //
            // The name there is package-internal (the bare re-export alias
            // `close`), while the importing module routes it as
            // `<package>.close` — so qualify it exactly as `ir::package`'s merge
            // qualifies the routing alias it has to match. Only a re-exported
            // close op is reachable from an importer at all, which is the form
            // this resolves.
            //
            // bug-377: `<package>.close` alone is NOT how the merged module
            // spells it. `merge_packages` runs `prefix_package_symbols` over
            // every imported package, so the definition `resource_cleanup_symbol`
            // has to find in `function_symbols` is the content-addressed
            // `<id>.<package>.close`. Registering the unprefixed name made that
            // lookup miss for *every* imported resource, so no
            // `ActiveCleanup::Resource` was pushed and the handle leaked
            // silently — bug-374's fix reached same-project `RESOURCE`
            // declarations only, which is all its regression test covers.
            let package_name = binary_repr::read_package_info(package)?.manifest_name;
            let identity = binary_repr::read_package_identity_id(package)?;
            for resource in exported_resources {
                let Some(close_function) = resource.close_function else {
                    continue;
                };
                model.resource_closers.insert(
                    resource.type_name,
                    format!("{identity}.{package_name}.{close_function}"),
                );
            }
            for type_export in binary_repr::read_package_type_exports(package)? {
                if native_resources.contains(&type_export.name) {
                    continue;
                }
                model.add_package_type_export(type_export)?;
            }
        }
        // Re-derive canonical variant tags over the FULL set (module + every
        // imported package union), so a variant shared across the boundary gets one
        // globally-consistent tag regardless of registration order (bug-80).
        model.recompute_canonical_variant_tags();
        Ok(model)
    }

    /// Assign each union variant a globally-canonical tag keyed by the variant
    /// name (sorted for determinism), independent of its position within any
    /// including union. `union_variant_fields` holds one entry per registered
    /// variant, so its keys are the complete variant set (bug-80).
    fn recompute_canonical_variant_tags(&mut self) {
        let names: std::collections::BTreeSet<String> =
            self.union_variant_fields.keys().cloned().collect();
        self.union_variant_tags = names
            .into_iter()
            .enumerate()
            .map(|(tag, name)| (name, tag))
            .collect();
    }

    fn add_package_type_export(
        &mut self,
        type_export: binary_repr::BinaryReprTypeExport,
    ) -> Result<(), String> {
        match type_export.kind {
            binary_repr::BinaryReprExportKind::Type => {
                self.record_fields.insert(
                    type_export.name,
                    type_export
                        .fields
                        .into_iter()
                        .map(|field| (field.name, ParameterType::parse(&field.type_)))
                        .collect(),
                );
            }
            binary_repr::BinaryReprExportKind::Enum => {
                for (index, member) in type_export.members.into_iter().enumerate() {
                    self.enum_members
                        .insert((type_export.name.clone(), member), index);
                }
            }
            binary_repr::BinaryReprExportKind::Union => {
                self.union_names.insert(type_export.name.clone());
                for variant in type_export.variants.into_iter() {
                    self.union_variants
                        .entry(variant.name.clone())
                        .or_insert_with(|| type_export.name.clone());
                    self.union_variant_unions
                        .entry(variant.name.clone())
                        .or_default()
                        .insert(type_export.name.clone());
                    // Tags are assigned globally by recompute_canonical_variant_tags
                    // in from_module_and_packages once every module + package variant
                    // is registered (bug-80).
                    self.union_variant_fields.insert(
                        variant.name,
                        variant
                            .fields
                            .into_iter()
                            .map(|field| (field.name, ParameterType::parse(&field.type_)))
                            .collect(),
                    );
                }
            }
            binary_repr::BinaryReprExportKind::Func | binary_repr::BinaryReprExportKind::Sub => {}
        }
        Ok(())
    }

    /// A union's variants in **deterministic canonical order**: ascending
    /// declaration/tag index (`union_variant_tags`), name as a tiebreak. The
    /// backing `union_variant_unions` is a `HashMap`, whose iteration order
    /// leaked into codegen (the resource-union drop dispatch emitted its
    /// per-variant tag checks in map order, so the same source produced
    /// different binaries run-to-run — bug-01). Pinning the order here makes
    /// every consumer deterministic without per-call-site changes; tags and
    /// layout are untouched (only emitted instruction order was ever affected).
    pub(crate) fn variants_for_union<'a>(
        &'a self,
        union: &'a str,
    ) -> impl Iterator<Item = &'a String> + 'a {
        let mut variants: Vec<&'a String> = self
            .union_variant_unions
            .iter()
            .filter(move |(_, unions)| unions.contains(union))
            .map(|(variant, _)| variant)
            .collect();
        variants.sort_by_key(|variant| {
            (
                self.union_variant_tags
                    .get(*variant)
                    .copied()
                    .unwrap_or(usize::MAX),
                (*variant).clone(),
            )
        });
        variants.into_iter()
    }
}

impl CollectionTypeLayout {
    pub(crate) fn from_type(type_: &str) -> Option<Self> {
        if let Some(value_type) = type_.strip_prefix("List OF ") {
            return Some(Self {
                // The single point that chooses a list's block representation
                // (plan-57-D). Every header writer takes `layout.kind` from
                // here, so the `kind` byte and the layout cannot disagree.
                kind: list_block_kind(value_type),
                key_type_code: COLLECTION_TYPE_NONE,
                value_type_code: collection_type_code(value_type)?,
            });
        }
        if let Some(element_type) = crate::codegen::engine::types::set_element_type(type_) {
            // `Set OF T` (plan-63): a Map-shaped block whose element is the key and
            // whose value is a 1-byte `Boolean` (always TRUE — see plan-63-B
            // Corrections). Keeping a real (if trivial) value lets every Map
            // emitter — set/remove/probe/projection/copy/free — be reused verbatim,
            // with the element as the key. The LookupEntry + bucket layout is
            // identical to a Map's.
            return Some(Self {
                kind: COLLECTION_KIND_SET,
                key_type_code: collection_type_code(&element_type)?,
                value_type_code: COLLECTION_TYPE_BOOLEAN,
            });
        }
        let (key_type, value_type) = map_type_parts(type_)?;
        Some(Self {
            kind: COLLECTION_KIND_MAP,
            key_type_code: collection_type_code(&key_type)?,
            value_type_code: collection_type_code(&value_type)?,
        })
    }
}

#[cfg(test)]
mod union_tag_tests {
    use super::*;
    use crate::target::shared::nir::{NirModule, NirType, NirVariant};

    fn union(name: &str, includes: &[&str], variants: &[&str]) -> NirType {
        NirType {
            kind: "union".to_string(),
            visibility: "private".to_string(),
            name: name.to_string(),
            fields: Vec::new(),
            includes: includes.iter().map(|s| s.to_string()).collect(),
            variants: variants
                .iter()
                .map(|s| NirVariant {
                    name: s.to_string(),
                    fields: Vec::new(),
                })
                .collect(),
            members: Vec::new(),
        }
    }

    fn module(types: Vec<NirType>) -> NirModule {
        NirModule {
            target: "test".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: crate::codegen::error::constants::STDIN_LOG_CAP_DEFAULT,
            project: "test".to_string(),
            entry: None,
            globals: Vec::new(),
            types,
            imports: Vec::new(),
            runtime_helpers: Vec::new(),
            functions: Vec::new(),
            link_functions: Vec::new(),
            link_cstructs: Vec::new(),
            native_resources: Vec::new(),
            native_libraries: Default::default(),
            max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        }
    }

    /// Tags are globally-canonical: keyed by the (sorted) variant name, not by a
    /// variant's position within any including union (bug-80). Variants Sq, Tri,
    /// V1 sort to tags 0, 1, 2.
    #[test]
    fn canonical_tags_are_name_sorted() {
        let types = vec![
            union("UV", &[], &["V1"]),
            union("Shape", &["UV"], &["Sq"]),
            union("Wide", &["Shape"], &["Tri"]),
        ];
        let model = TypeModel::from_module(&module(types)).expect("must resolve");
        assert_eq!(model.union_variant_tags.get("Sq"), Some(&0));
        assert_eq!(model.union_variant_tags.get("Tri"), Some(&1));
        assert_eq!(model.union_variant_tags.get("V1"), Some(&2));
    }

    /// A variant at *divergent* positions across two unions (`W1` follows `V1` in
    /// `A` but is first in `L2`) is no longer rejected — the canonical scheme gives
    /// it ONE stable tag everywhere (sorted: V1=0, W1=1), so a MATCH dispatches
    /// consistently regardless of which union viewed it (bug-80).
    #[test]
    fn divergent_positions_resolve_to_one_canonical_tag() {
        let types = vec![
            union("UV", &[], &["V1"]),
            union("UW", &[], &["W1"]),
            union("A", &["UV", "UW"], &[]),
            union("L2", &["UW"], &[]),
        ];
        let model =
            TypeModel::from_module(&module(types)).expect("divergent positions must now resolve");
        assert_eq!(model.union_variant_tags.get("V1"), Some(&0));
        assert_eq!(model.union_variant_tags.get("W1"), Some(&1));
    }
}
