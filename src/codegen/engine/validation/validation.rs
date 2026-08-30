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
        let mut union_variant_unions = HashMap::<ParameterType, HashSet<ParameterType>>::new();
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
                        ParameterType::declared(&type_.name),
                        type_
                            .fields
                            .iter()
                            .map(|field| (field.name.clone(), field.type_.clone()))
                            .collect(),
                    );
                }
                "enum" => {
                    for (index, member) in type_.members.iter().enumerate() {
                        enum_members.insert(
                            (ParameterType::declared(&type_.name), member.name.clone()),
                            index,
                        );
                    }
                }
                "union" => {
                    let union_type = ParameterType::declared(&type_.name);
                    union_names.insert(union_type.clone());
                    for variant in expanded_nir_union_variants(module, &type_.name).iter() {
                        let variant_type = ParameterType::declared(&variant.name);
                        union_variants
                            .entry(variant_type.clone())
                            .or_insert_with(|| union_type.clone());
                        union_variant_unions
                            .entry(variant_type.clone())
                            .or_default()
                            .insert(union_type.clone());
                        union_variant_fields.insert(
                            variant_type,
                            variant
                                .fields
                                .iter()
                                .map(|field| (field.name.clone(), field.type_.clone()))
                                .collect(),
                        );
                    }
                }
                "resource" => {
                    resource_names.insert(ParameterType::declared(&type_.name));
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
                resource_names.insert(function.return_type.without_state());
            }
        }
        // `Error` and `ErrorLoc` are read-only compiler/runtime records laid out
        // as ordinary 3-field records so construction, field access, copying, and
        // cleanup reuse the generic record machinery.
        record_fields.insert(
            ParameterType::named("Error"),
            vec![
                ("code".to_string(), ParameterType::Integer),
                ("message".to_string(), ParameterType::String),
                ("source".to_string(), ParameterType::named("ErrorLoc")),
            ],
        );
        record_fields.insert(
            ParameterType::named("ErrorLoc"),
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
            ParameterType::named("AttrSpan"),
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
            ParameterType::named("AttributedString"),
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
            .map(|resource| {
                (
                    ParameterType::declared(&resource.name),
                    resource.close_function.clone(),
                )
            })
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
        #[cfg(debug_assertions)]
        model.assert_type_keys_are_bijective();
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
            model.resource_names.extend(
                native_resources
                    .iter()
                    .map(|name| ParameterType::declared(name)),
            );
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
                    ParameterType::declared(&resource.type_name),
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
        #[cfg(debug_assertions)]
        model.assert_type_keys_are_bijective();
        Ok(model)
    }

    /// plan-111-C Phase 2: the equivalence check that stands in for the
    /// byte-level gate this letter does not get.
    ///
    /// Re-keying a table from a SPELLING to a `ParameterType` is safe exactly
    /// when the map from spellings to keys is a bijection over the keys actually
    /// present. Two failure modes, both silent:
    ///
    /// * a **merge** — two distinct spellings collapsing to one key, so one
    ///   table entry overwrites the other and a lookup returns the wrong record
    ///   layout, union tag or close op;
    /// * a **split** — one spelling reaching the table as two different keys, so
    ///   a lookup that used to hit now misses. Correction C1 is a real instance:
    ///   `ParameterType::named("Integer")` and the `Integer` variant are
    ///   different keys for the same declaration.
    ///
    /// Both are ruled out by asserting, for every key present, that it survives
    /// a round trip through its own spelling and that no two keys share one.
    /// Debug-only, and checked at CONSTRUCTION so it covers every module the
    /// corpus compiles rather than only the lookups a given program reaches.
    ///
    /// (plan-111-C Phase 2 specified this as temporary scaffolding with shadow
    /// string tables, to be deleted once the corpus was clean. This form needs
    /// no shadow table and costs nothing in release, so it stays — see the
    /// letter's Corrections.)
    #[cfg(debug_assertions)]
    fn assert_type_keys_are_bijective(&self) {
        use std::collections::HashMap as Keys;
        let mut seen: Keys<String, ParameterType> = Keys::new();
        let mut check = |key: &ParameterType, table: &str| {
            let spelled = key.name().into_owned();
            assert_eq!(
                &ParameterType::declared(&spelled),
                key,
                "TypeModel.{table}: key `{spelled}` does not round-trip — re-keying \
                 SPLIT it (plan-111-C Correction C1). Build the key with \
                 `ParameterType::declared`, which is what an `AS {spelled}` \
                 annotation elaborates to."
            );
            if let Some(previous) = seen.insert(spelled.clone(), key.clone()) {
                assert_eq!(
                    &previous, key,
                    "TypeModel: two distinct keys share the spelling `{spelled}` — \
                     re-keying MERGED them, so one table entry overwrites the other."
                );
            }
        };
        for key in self.record_fields.keys() {
            check(key, "record_fields");
        }
        for key in self.union_names.iter() {
            check(key, "union_names");
        }
        for key in self.union_variants.keys() {
            check(key, "union_variants");
        }
        for key in self.union_variant_unions.keys() {
            check(key, "union_variant_unions");
        }
        for key in self.union_variant_tags.keys() {
            check(key, "union_variant_tags");
        }
        for key in self.union_variant_fields.keys() {
            check(key, "union_variant_fields");
        }
        for key in self.resource_names.iter() {
            check(key, "resource_names");
        }
        for key in self.resource_closers.keys() {
            check(key, "resource_closers");
        }
        for (key, _) in self.enum_members.keys() {
            check(key, "enum_members");
        }
    }

    /// Assign each union variant a globally-canonical tag keyed by the variant
    /// name (sorted for determinism), independent of its position within any
    /// including union. `union_variant_fields` holds one entry per registered
    /// variant, so its keys are the complete variant set (bug-80).
    fn recompute_canonical_variant_tags(&mut self) {
        // plan-111-C: the ORDER is load-bearing — a tag is an emitted constant,
        // so the sort must stay by rendered NAME exactly as it was.
        // `ParameterType` is deliberately not `Ord` (an ordering over a type
        // tree would be arbitrary), so the sort key is the spelling and the map
        // key is the type it denotes.
        let names: std::collections::BTreeSet<String> = self
            .union_variant_fields
            .keys()
            .map(|type_| type_.name().into_owned())
            .collect();
        self.union_variant_tags = names
            .into_iter()
            .enumerate()
            .map(|(tag, name)| (ParameterType::declared(&name), tag))
            .collect();
    }

    fn add_package_type_export(
        &mut self,
        type_export: binary_repr::BinaryReprTypeExport,
    ) -> Result<(), String> {
        match type_export.kind {
            binary_repr::BinaryReprExportKind::Type => {
                self.record_fields.insert(
                    ParameterType::declared(&type_export.name),
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
                        .insert((ParameterType::declared(&type_export.name), member), index);
                }
            }
            binary_repr::BinaryReprExportKind::Union => {
                let union_type = ParameterType::declared(&type_export.name);
                self.union_names.insert(union_type.clone());
                for variant in type_export.variants.into_iter() {
                    let variant_type = ParameterType::declared(&variant.name);
                    self.union_variants
                        .entry(variant_type.clone())
                        .or_insert_with(|| union_type.clone());
                    self.union_variant_unions
                        .entry(variant_type.clone())
                        .or_default()
                        .insert(union_type.clone());
                    // Tags are assigned globally by recompute_canonical_variant_tags
                    // in from_module_and_packages once every module + package variant
                    // is registered (bug-80).
                    self.union_variant_fields.insert(
                        variant_type,
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
        union: &'a ParameterType,
    ) -> impl Iterator<Item = &'a ParameterType> + 'a {
        let mut variants: Vec<&'a ParameterType> = self
            .union_variant_unions
            .iter()
            .filter(move |(_, unions)| unions.contains(union))
            .map(|(variant, _)| variant)
            .collect();
        // plan-111-C: the tiebreak is still the rendered NAME. `ParameterType`
        // is not `Ord`, and more to the point an ordering over a type tree would
        // be arbitrary where this one is observable — the emitted per-variant
        // tag checks are in this order (bug-01).
        variants.sort_by_key(|variant| {
            (
                self.union_variant_tags
                    .get(*variant)
                    .copied()
                    .unwrap_or(usize::MAX),
                variant.name().into_owned(),
            )
        });
        variants.into_iter()
    }
}

impl CollectionTypeLayout {
    /// The block layout of a collection type.
    ///
    /// plan-106-E: dispatches on the variant. The element/key/value types it
    /// hands on are the variant's children, not substrings of its spelling.
    pub(crate) fn from_type(type_: &ParameterType) -> Option<Self> {
        if let crate::types::ParameterType::ListOf(value_type) = type_ {
            return Some(Self {
                // The single point that chooses a list's block representation
                // (plan-57-D). Every header writer takes `layout.kind` from
                // here, so the `kind` byte and the layout cannot disagree.
                kind: list_block_kind(&value_type.name()),
                key_type_code: COLLECTION_TYPE_NONE,
                value_type_code: collection_type_code(value_type)?,
            });
        }
        if let Some(element_type) =
            crate::codegen::engine::types::typed_set_element_type(type_).cloned()
        {
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
        let (key_type, value_type) = crate::codegen::engine::types::typed_map_type_parts(type_)?;
        Some(Self {
            kind: COLLECTION_KIND_MAP,
            key_type_code: collection_type_code(key_type)?,
            value_type_code: collection_type_code(value_type)?,
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

    fn record(name: &str, fields: &[(&str, ParameterType)]) -> NirType {
        NirType {
            kind: "record".to_string(),
            visibility: "private".to_string(),
            name: name.to_string(),
            fields: fields
                .iter()
                .map(|(field, type_)| crate::target::shared::nir::NirField {
                    name: (*field).to_string(),
                    type_: type_.clone(),
                    visibility: None,
                })
                .collect(),
            includes: Vec::new(),
            variants: Vec::new(),
            members: Vec::new(),
        }
    }

    /// plan-111-C Phase 2: the two key shapes most likely to differ between a
    /// spelling-keyed and a type-keyed lookup.
    ///
    /// A **nested container** (`List OF Map OF String TO Integer`) is the shape
    /// where a naive re-key could decompose differently at each level; a
    /// **stateful resource** (`File STATE Cursor`) is the one plan-111-A gave a
    /// variant, so its spelling and its structure stopped being the same thing.
    /// Both must resolve to the entry their declaration registered.
    #[test]
    fn a_type_model_resolves_nested_container_and_stateful_resource_keys() {
        let nested = ParameterType::parse("List OF Map OF String TO Integer");
        let stateful = ParameterType::parse("File STATE Cursor");
        let model = TypeModel::from_module(&module(vec![
            record("Holder", &[("items", nested.clone())]),
            record("Handle", &[("state", stateful.clone())]),
        ]))
        .expect("model builds");

        // The declarations are reachable by the type their own name denotes...
        let holder = model
            .record_fields
            .get(&ParameterType::declared("Holder"))
            .expect("Holder is registered");
        assert_eq!(
            holder[0].1, nested,
            "the nested container survives re-keying"
        );
        let handle = model
            .record_fields
            .get(&ParameterType::declared("Handle"))
            .expect("Handle is registered");
        assert_eq!(
            handle[0].1, stateful,
            "the stateful resource survives re-keying"
        );

        // ...and a key built from the SPELLING finds the same entry, which is the
        // property every emitter still holding a name depends on.
        assert!(model.record_fields.contains_key(&ParameterType::declared(
            &ParameterType::declared("Holder").name()
        )));

        // A composite is not a record key and must miss, both before and after.
        assert!(model.record_fields.get(&nested).is_none());
        assert!(model.record_fields.get(&stateful).is_none());
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
        assert_eq!(
            model.union_variant_tags.get(&ParameterType::declared("Sq")),
            Some(&0)
        );
        assert_eq!(
            model
                .union_variant_tags
                .get(&ParameterType::declared("Tri")),
            Some(&1)
        );
        assert_eq!(
            model.union_variant_tags.get(&ParameterType::declared("V1")),
            Some(&2)
        );
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
        assert_eq!(
            model.union_variant_tags.get(&ParameterType::declared("V1")),
            Some(&0)
        );
        assert_eq!(
            model.union_variant_tags.get(&ParameterType::declared("W1")),
            Some(&1)
        );
    }
}
