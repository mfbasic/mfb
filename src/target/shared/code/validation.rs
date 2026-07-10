use super::*;

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

    pub(crate) fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-native-code-plan\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"buildMode\": {},\n",
                "  \"arch\": {},\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"imports\": [{}\n  ],\n",
                "  \"dataObjects\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(self.build_mode.as_str()),
            json_string(&self.arch),
            json_string(&self.project),
            self.entry_symbol
                .as_ref()
                .map(|symbol| json_string(symbol))
                .unwrap_or_else(|| "null".to_string()),
            join_json(&self.imports, 2),
            join_json(&self.data_objects, 2),
            join_json(&self.functions, 2)
        )
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
        Ok(())
    }
}

/// Record a variant's construction/dispatch tag, enforcing that a variant name
/// resolves to a **single, stable tag** across every union that includes it.
///
/// The native layout gives each union variant one context-free discriminant word
/// (`union_variant_tags`, keyed by variant name), so a union value carries the
/// same tag no matter which union it is viewed through — that is what lets a
/// narrower union value flow into a wider including union without re-tagging, and
/// lets `MATCH` compare the stored tag against a per-variant constant.
///
/// `expanded_nir_union_variants` numbers a union's variants by position, and the
/// map takes the last union processed to touch a name. If one variant sat at
/// *divergent positions* in two unions (e.g. an included union placed after a
/// differing count of preceding variants), the two positions would fight over the
/// single map slot, and two distinct variants could collapse onto the same tag
/// within one union — silently mis-dispatching a `MATCH` (bug-69: a value built as
/// one variant matched as another). The resolver permits this shape today
/// (`UNION A INCLUDES Base` and `UNION C INCLUDES Other, Base` place `Base`'s
/// variants at different offsets), so this guard rejects it loudly at the native
/// boundary — in every build, not just debug — instead of miscompiling. Programs
/// whose variants already have stable tags (the norm, including all `INCLUDES`
/// prefixes) insert each tag once per value and are unaffected: byte-identical.
fn check_union_variant_tag(
    tags: &mut HashMap<String, usize>,
    variant: &str,
    index: usize,
) -> Result<(), String> {
    if let Some(previous) = tags.insert(variant.to_string(), index) {
        if previous != index {
            return Err(format!(
                "native code plan: union variant '{variant}' resolves to conflicting \
                 tags ({previous} and {index}); every variant must have one stable \
                 tag across all unions that include it. This happens when a variant \
                 sits at divergent positions in two unions, which can collide two \
                 variants onto the same tag and mis-dispatch a MATCH."
            ));
        }
    }
    Ok(())
}

impl TypeModel {
    pub(super) fn empty() -> Self {
        Self {
            enum_members: HashMap::new(),
            record_fields: HashMap::new(),
            union_names: HashSet::new(),
            union_variants: HashMap::new(),
            union_variant_unions: HashMap::new(),
            union_variant_tags: HashMap::new(),
            union_variant_fields: HashMap::new(),
        }
    }

    pub(super) fn from_module(module: &NirModule) -> Result<Self, String> {
        let mut enum_members = HashMap::new();
        let mut record_fields = HashMap::new();
        let mut union_names = HashSet::new();
        let mut union_variants = HashMap::new();
        let mut union_variant_unions = HashMap::<String, HashSet<String>>::new();
        let mut union_variant_tags = HashMap::new();
        let mut union_variant_fields = HashMap::new();
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
                    for (index, variant) in expanded_nir_union_variants(module, &type_.name)
                        .iter()
                        .enumerate()
                    {
                        union_variants
                            .entry(variant.name.clone())
                            .or_insert_with(|| type_.name.clone());
                        union_variant_unions
                            .entry(variant.name.clone())
                            .or_default()
                            .insert(type_.name.clone());
                        check_union_variant_tag(
                            &mut union_variant_tags,
                            &variant.name,
                            index,
                        )?;
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
                "resource" => {}
                other => {
                    return Err(format!(
                        "native code plan does not know type kind '{other}'"
                    ));
                }
            }
        }
        for type_name in ["Address", "Datagram", "DatagramText"] {
            if let Some(fields) = builtins::net::builtin_type_fields(type_name) {
                record_fields.insert(
                    type_name.to_string(),
                    fields
                        .iter()
                        .map(|(name, type_)| ((*name).to_string(), (*type_).to_string()))
                        .collect(),
                );
            }
        }
        for type_name in ["TermColor", "TermSize"] {
            if let Some(fields) = builtins::term::builtin_type_fields(type_name) {
                record_fields.insert(
                    type_name.to_string(),
                    fields
                        .iter()
                        .map(|(name, type_)| ((*name).to_string(), (*type_).to_string()))
                        .collect(),
                );
            }
        }
        // `Error` and `ErrorLoc` are read-only compiler/runtime records laid out
        // as ordinary 3-field records so construction, field access, copying, and
        // cleanup reuse the generic record machinery.
        record_fields.insert(
            "Error".to_string(),
            vec![
                ("code".to_string(), "Integer".to_string()),
                ("message".to_string(), "String".to_string()),
                ("source".to_string(), "ErrorLoc".to_string()),
            ],
        );
        record_fields.insert(
            "ErrorLoc".to_string(),
            vec![
                ("filename".to_string(), "String".to_string()),
                ("line".to_string(), "Integer".to_string()),
                ("char".to_string(), "Integer".to_string()),
            ],
        );
        Ok(Self {
            enum_members,
            record_fields,
            union_names,
            union_variants,
            union_variant_unions,
            union_variant_tags,
            union_variant_fields,
        })
    }

    pub(super) fn from_module_and_packages(
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
            let native_resources: HashSet<String> = binary_repr::read_package_resources(package)?
                .into_iter()
                .filter(|resource| resource.native)
                .map(|resource| resource.type_name)
                .collect();
            for type_export in binary_repr::read_package_type_exports(package)? {
                if native_resources.contains(&type_export.name) {
                    continue;
                }
                model.add_package_type_export(type_export)?;
            }
        }
        Ok(model)
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
                        .map(|field| (field.name, field.type_))
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
                for (index, variant) in type_export.variants.into_iter().enumerate() {
                    self.union_variants
                        .entry(variant.name.clone())
                        .or_insert_with(|| type_export.name.clone());
                    self.union_variant_unions
                        .entry(variant.name.clone())
                        .or_default()
                        .insert(type_export.name.clone());
                    check_union_variant_tag(
                        &mut self.union_variant_tags,
                        &variant.name,
                        index,
                    )?;
                    self.union_variant_fields.insert(
                        variant.name,
                        variant
                            .fields
                            .into_iter()
                            .map(|field| (field.name, field.type_))
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
    pub(super) fn variants_for_union<'a>(
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
    pub(super) fn from_type(type_: &str) -> Option<Self> {
        if let Some(value_type) = type_.strip_prefix("List OF ") {
            return Some(Self {
                kind: COLLECTION_KIND_LIST,
                key_type_code: COLLECTION_TYPE_NONE,
                value_type_code: collection_type_code(value_type)?,
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
            project: "test".to_string(),
            entry: None,
            globals: Vec::new(),
            types,
            imports: Vec::new(),
            runtime_helpers: Vec::new(),
            functions: Vec::new(),
            link_functions: Vec::new(),
        }
    }

    /// A variant reached through a chain of `INCLUDES` at a *stable* position
    /// (`V1` is always tag 0, `Sq` always tag 1) builds a consistent tag map.
    #[test]
    fn stable_include_positions_resolve() {
        let types = vec![
            union("UV", &[], &["V1"]),
            union("Shape", &["UV"], &["Sq"]),
            union("Wide", &["Shape"], &["Tri"]),
        ];
        let model = TypeModel::from_module(&module(types)).expect("stable positions must resolve");
        assert_eq!(model.union_variant_tags.get("V1"), Some(&0));
        assert_eq!(model.union_variant_tags.get("Sq"), Some(&1));
        assert_eq!(model.union_variant_tags.get("Tri"), Some(&2));
    }

    /// A variant at *divergent* positions across two unions (`W1` is tag 0 in
    /// `UW`/`L2` but tag 1 in `A`, because `UV`'s variant precedes it there) is
    /// rejected — it would otherwise collapse two variants onto one tag and
    /// mis-dispatch a `MATCH` (bug-69).
    #[test]
    fn divergent_positions_are_rejected() {
        let types = vec![
            union("UV", &[], &["V1"]),
            union("UW", &[], &["W1"]),
            union("A", &["UV", "UW"], &[]),
            union("L2", &["UW"], &[]),
        ];
        let error = match TypeModel::from_module(&module(types)) {
            Ok(_) => panic!("divergent variant positions must be rejected"),
            Err(error) => error,
        };
        assert!(
            error.contains("conflicting tags") && error.contains("W1"),
            "unexpected error message: {error}"
        );
    }
}
