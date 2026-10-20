use super::*;
use crate::operators::{BinaryOp, UnaryOp};

mod body;
mod instantiate;
mod resolve;
mod types;
use types::expected_element_type;

/// Remove every occurrence of `qualifier` in `input` that begins a type-name
/// token — at position 0 or immediately after a non-identifier byte — leaving
/// substring occurrences inside a longer identifier untouched (so `io.` does not
/// bite into `radio.`). See `Monomorphizer::normalize_type` (bug-104).
fn strip_qualifier_prefixes(input: &str, qualifier: &str) -> String {
    if qualifier.is_empty() {
        return input.to_string();
    }
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i..].starts_with(qualifier) && (i == 0 || !is_ident(bytes[i - 1])) {
            i += qualifier.len();
            continue;
        }
        let ch = input[i..].chars().next().expect("valid char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// plan-140-A: the built-in resolver's enum oracle at the gap-fill dispatch check.
/// Answers from the declaration's KIND — a source enum in `concrete_types`, or an
/// imported package's — never from "is a declared type".
impl crate::codegen::builtins::TypeKinds for Monomorphizer<'_> {
    fn is_enum(&self, t: &ParameterType) -> bool {
        match self.concrete_types.get(t) {
            Some(type_decl) => matches!(type_decl.kind, TypeDeclKind::Enum),
            None => {
                self.imported_enums.contains(t)
                    || self.imported_enums.contains(&self.normalize_type(t))
            }
        }
    }

    /// bug-988: a declared union's variants, through its `INCLUDES`, else the
    /// built-in packages' unions.
    fn union_accepts(&self, union: &ParameterType, actual: &ParameterType) -> bool {
        let actual = actual.without_state();
        let mut pending = vec![union.without_state()];
        let mut seen = std::collections::HashSet::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            let Some(decl) = self
                .concrete_types
                .get(&current)
                .filter(|decl| matches!(decl.kind, TypeDeclKind::Union))
            else {
                continue;
            };
            if decl.variants.iter().any(|variant| variant.type_ == actual) {
                return true;
            }
            pending.extend(decl.includes.iter().cloned());
        }
        crate::codegen::registry::registry_union_accepts(union, &actual)
    }
}

impl<'a> Monomorphizer<'a> {
    pub(super) fn new(project_dir: &'a Path, source: &'a HirProject) -> Self {
        let mut type_templates = HashMap::new();
        let mut function_templates = HashMap::new();
        let mut concrete_types = HashMap::new();
        let mut concrete_functions = HashMap::new();
        let mut function_overloads: HashMap<String, Vec<HirFunction>> = HashMap::new();
        let mut overload_names = HashMap::new();
        let mut function_files: HashMap<String, String> = HashMap::new();
        let mut type_template_files: HashMap<String, String> = HashMap::new();

        for file in &source.files {
            for item in &file.items {
                match item {
                    HirItem::Binding(_) => {}
                    HirItem::Type(type_decl) if !type_decl.template_params.is_empty() => {
                        type_template_files.insert(type_decl.name.clone(), file.path.clone());
                        type_templates
                            .insert(ParameterType::declared(&type_decl.name), type_decl.clone());
                    }
                    HirItem::Type(type_decl) => {
                        concrete_types
                            .insert(ParameterType::declared(&type_decl.name), type_decl.clone());
                    }
                    HirItem::Function(function) if !function.template_params.is_empty() => {
                        function_files
                            .entry(function.name.clone())
                            .or_insert_with(|| file.path.clone());
                        function_templates
                            .entry(function.name.clone())
                            .or_insert_with(Vec::new)
                            .push((function.clone(), file.path.clone()));
                    }
                    HirItem::Function(function) => {
                        function_files
                            .entry(function.name.clone())
                            .or_insert_with(|| file.path.clone());
                        function_overloads
                            .entry(function.name.clone())
                            .or_default()
                            .push(function.clone());
                    }
                    // Native LINK resources, re-export aliases, and LINK blocks
                    // carry no template parameters and are passed through
                    // unchanged (plan-link-update.md §15 Phase 1).
                    HirItem::Resource(_) | HirItem::FuncAlias(_) | HirItem::Link(_) => {}
                    // DOC blocks carry no template parameters; passed through below.
                    HirItem::Doc(_) => {}
                    // TESTING blocks are lowered away before monomorphization.
                    HirItem::Testing(_) => {}
                }
            }
        }

        // bug-912: a file's PRIVATE `FUNC f` is file-scoped to `#<hash>$f`
        // (`ast::scope_privates`), and so is every reference to `f` inside that
        // file, while the file's own PUBLIC/EXPORT `FUNC f` keeps its name. Inside
        // the file they are one overload set, so the file-scoped name's set gains
        // the same-file siblings. The set a bare `f` names from any OTHER file is
        // untouched: it never holds a PRIVATE member.
        let mut template_views: HashMap<String, String> = HashMap::new();
        for file in source.files.iter().filter(|file| !file.internal) {
            let scoped: HashMap<&str, &str> = file
                .items
                .iter()
                .filter_map(|item| match item {
                    HirItem::Function(function) => {
                        crate::internal_name::private_name_parts(&function.name)
                            .map(|(_, plain)| (plain, function.name.as_str()))
                    }
                    _ => None,
                })
                .collect();
            if scoped.is_empty() {
                continue;
            }
            for item in &file.items {
                let HirItem::Function(function) = item else {
                    continue;
                };
                let Some(scoped_name) = scoped.get(function.name.as_str()) else {
                    continue;
                };
                if function.template_params.is_empty() {
                    function_overloads
                        .entry(scoped_name.to_string())
                        .or_default()
                        .push(function.clone());
                } else if !function_templates.contains_key(*scoped_name) {
                    template_views.insert(scoped_name.to_string(), function.name.clone());
                }
            }
        }

        for (set_name, functions) in &function_overloads {
            // A same-file sibling added above is named, and has its symbol
            // decided, by its own set.
            for function in functions.iter().filter(|member| member.name == *set_name) {
                // A user `FUNC` whose name is an overridable general built-in is
                // always force-mangled so its codegen symbol never equals the
                // built-in dispatch name (plan-01-overload.md §C Phase 5.1).
                let builtin_named =
                    crate::codegen::builtins::general::is_overridable(&function.name);
                // bug-653: an EXPORT's symbol is its name in the `.mfp` export table
                // and ABI index, and importers see only EXPORTs — so it is decided
                // by its EXPORT siblings alone. Counting a hidden PUBLIC/PRIVATE
                // sibling renamed a lone export `f` to `f$String`, which no importer
                // could map back to `f`. A non-exported member still counts the whole
                // set, so it is mangled whenever any sibling exists and its symbol
                // can never equal an export's.
                let exported = function.visibility == crate::ast::Visibility::Export;
                let siblings = functions
                    .iter()
                    .filter(|other| !exported || other.visibility == function.visibility)
                    .collect::<Vec<_>>();
                // A return-type overload set: ≥2 declarations share this name *and*
                // parameter types, differing only by return type (§F.1). Their
                // concrete symbols must also encode the return type to stay distinct.
                let return_disambiguated = siblings
                    .iter()
                    .filter(|other| param_types_eq(other, function))
                    .count()
                    > 1;
                let concrete_name = overload_concrete_name(
                    function,
                    siblings.len() > 1 || builtin_named,
                    return_disambiguated,
                );
                overload_names.insert(
                    overload_key(
                        &function.name,
                        &function.params,
                        opt_type_name(&function.returns).as_deref(),
                    ),
                    concrete_name.clone(),
                );
                if let Some(path) = function_files.get(&function.name).cloned() {
                    function_files.insert(concrete_name.clone(), path);
                }
                let mut concrete = function.clone();
                concrete.name = concrete_name.clone();
                concrete_functions.insert(concrete_name, concrete);
            }
        }

        let (imported_overloads, package_qualifiers) =
            collect_imported_overloads(project_dir, source);

        Self {
            project_dir,
            source,
            type_templates,
            function_templates,
            concrete_types,
            imported_records: collect_imported_records(project_dir, source),
            imported_enums: collect_imported_enums(project_dir, source),
            concrete_functions,
            function_overloads,
            template_views,
            overload_names,
            imported_overloads,
            file_imports: source
                .files
                .iter()
                .map(|file| (file.path.clone(), file.import_bindings()))
                .collect(),
            package_qualifiers,
            type_instantiations: HashMap::new(),
            emitted_type_keys: HashSet::new(),
            emitted_function_keys: HashSet::new(),
            concrete_symbol_keys: HashMap::new(),
            function_instantiation_names: HashMap::new(),
            // Binding names (including aliases) of every `collections` import — used
            // to map a `binding.member` callee onto the source-generic implementation.
            collections_bindings: source
                .files
                .iter()
                .flat_map(|file| file.imports.iter())
                .filter(|import| import.package_name() == "collections")
                .map(|import| import.binding_name().to_string())
                .collect(),
            function_files,
            type_template_files,
            internal_files: source
                .files
                .iter()
                .filter(|file| file.internal)
                .map(|file| file.path.clone())
                .collect(),
            // Top-level `LET`/`MUT` bindings with an explicit `AS` type, so a call
            // or overload whose argument names a global can be typed (bug-103).
            // `source` is immutable, so this is derived once here instead of per
            // lowered function (plan-117 Phase 2).
            globals: source
                .files
                .iter()
                .flat_map(|file| &file.items)
                .filter_map(|item| match item {
                    HirItem::Binding(binding) if binding.explicit_type => {
                        Some((binding.name.clone(), binding.type_.clone()))
                    }
                    _ => None,
                })
                .collect(),
            current_file: None,
            current_instantiation_site: None,
            template_instantiation_depth: 0,
            total_instantiations: 0,
            instantiation_limit_reached: false,
            had_error: false,
        }
    }

    pub(super) fn run(&mut self) {
        // Both tables are `HashMap`s, whose iteration order changes from one
        // process to the next. Lowering reports diagnostics as it goes and claims
        // collision-suffixed instantiation symbols first-come (bug-226), so walk
        // them in a fixed order — functions by declaring file, line and name,
        // types by line and name — or the same program prints its diagnostics in
        // a different order on every build (found with bug-822).
        let mut types = self.concrete_types.values().cloned().collect::<Vec<_>>();
        types.sort_by(|left, right| (left.line, &left.name).cmp(&(right.line, &right.name)));
        for type_decl in types {
            let lowered = self.lower_type(type_decl, &HashMap::new(), None);
            self.concrete_types
                .insert(ParameterType::declared(&lowered.name), lowered);
        }

        let mut functions = self
            .concrete_functions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        functions.sort_by(|left, right| {
            let file = |function: &HirFunction| self.function_files.get(&function.name).cloned();
            (file(left), left.line, &left.name).cmp(&(file(right), right.line, &right.name))
        });
        for function in functions {
            let lowered = self.lower_function(function, &HashMap::new(), None);
            self.concrete_functions
                .insert(lowered.name.clone(), lowered);
        }
    }

    pub(super) fn into_project(mut self) -> HirProject {
        let mut emitted_types = HashSet::new();
        let mut emitted_functions = HashSet::new();
        let mut files = self
            .source
            .files
            .iter()
            .map(|file| {
                let mut items = Vec::new();
                for item in &file.items {
                    match item {
                        HirItem::Binding(binding) => {
                            // A global's initializer is lowered in its own file,
                            // whose import bindings its calls resolve through
                            // (bug-914) and whose diagnostics are its own.
                            let saved_file = self.current_file.replace(file.path.clone());
                            items.push(HirItem::Binding(self.lower_binding(binding.clone())));
                            self.current_file = saved_file;
                        }
                        HirItem::Type(type_decl) if type_decl.template_params.is_empty() => {
                            if let Some(concrete) = self
                                .concrete_types
                                .get(&ParameterType::declared(&type_decl.name))
                            {
                                emitted_types.insert(concrete.name.clone());
                                items.push(HirItem::Type(concrete.clone()));
                            }
                        }
                        HirItem::Function(function) if function.template_params.is_empty() => {
                            let concrete_name = self
                                .overload_names
                                .get(&overload_key(
                                    &function.name,
                                    &function.params,
                                    opt_type_name(&function.returns).as_deref(),
                                ))
                                .map(String::as_str)
                                .unwrap_or(&function.name);
                            if let Some(concrete) = self.concrete_functions.get(concrete_name) {
                                emitted_functions.insert(concrete.name.clone());
                                items.push(HirItem::Function(concrete.clone()));
                            }
                        }
                        // Native LINK constructs are not monomorphized; preserve
                        // them verbatim so later stages (resolve, the former source checker,
                        // package metadata) still see them.
                        HirItem::Resource(resource) => {
                            items.push(HirItem::Resource(resource.clone()));
                        }
                        HirItem::FuncAlias(alias) => {
                            items.push(HirItem::FuncAlias(alias.clone()));
                        }
                        HirItem::Link(link) => {
                            items.push(HirItem::Link(link.clone()));
                        }
                        // Preserve DOC blocks verbatim so the post-monomorph
                        // resolve and IR lowering still see the documentation.
                        HirItem::Doc(doc) => {
                            items.push(HirItem::Doc(doc.clone()));
                        }
                        _ => {}
                    }
                }
                HirFile {
                    path: file.path.clone(),
                    imports: file.imports.clone(),
                    // Copied verbatim, and NOT widened by the union below: this
                    // is the file's own declaration list, which the union is
                    // deliberately not (see `HirFile::own_imports`).
                    own_imports: file.own_imports.clone(),
                    items,
                    internal: file.internal,
                }
            })
            .collect::<Vec<_>>();

        // bug-911: a generated instantiation is emitted into the file that
        // DECLARES its template, so every later stage keyed on a function's file
        // — the post-monomorph resolve's import bindings, the PRIVATE-visibility
        // checks, diagnostics, and `IrFunction::file` (the runtime `ErrorLoc`) —
        // sees the scope the template body was written in, whatever that file's
        // place in the source order. A template the toolchain declares (a
        // built-in package's injected source, the prelude) has no user file: its
        // instantiations go into the first file, as they always have, and carry
        // their own call-site attribution (`HirFunction::instantiated_at`).
        let user_files: HashMap<&str, usize> = self
            .source
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| !file.internal && !file.path.starts_with('<'))
            .map(|(index, file)| (file.path.as_str(), index))
            .collect();
        let host =
            |path: Option<&String>| path.and_then(|path| user_files.get(path.as_str()).copied());

        // Keyed as `concrete_types` keys them — for an instantiation, the mangled
        // nominal `type_instantiations` is keyed by too.
        let mut generated_types = self
            .concrete_types
            .iter()
            .filter(|(_, type_decl)| !emitted_types.contains(&type_decl.name))
            .map(|(key, type_decl)| (key.clone(), type_decl.clone()))
            .collect::<Vec<_>>();
        generated_types.sort_by(|(_, left), (_, right)| left.name.cmp(&right.name));
        let mut generated_functions = self
            .concrete_functions
            .values()
            .filter(|function| !emitted_functions.contains(&function.name))
            .cloned()
            .collect::<Vec<_>>();
        generated_functions.sort_by(|left, right| left.name.cmp(&right.name));

        // The first file always hosts (and is always widened, below): it is where
        // the toolchain's instantiations land.
        let mut hosts: Vec<usize> = Vec::new();
        if !files.is_empty() {
            hosts.push(0);
        }
        // Per file, the generated types precede the generated functions, each
        // sorted by name — the order the first file has always had.
        let mut placed: Vec<(usize, HirItem)> = Vec::new();
        for (key, type_decl) in generated_types {
            let template = self
                .type_instantiations
                .get(&key)
                .map(|(template, _)| template);
            let index = host(template.and_then(|template| self.type_template_files.get(template)));
            placed.push((index.unwrap_or(0), HirItem::Type(type_decl)));
        }
        for function in generated_functions {
            let index = host(self.function_files.get(&function.name));
            placed.push((index.unwrap_or(0), HirItem::Function(function)));
        }
        for (index, item) in placed {
            let Some(file) = files.get_mut(index) else {
                continue;
            };
            if !hosts.contains(&index) {
                hosts.push(index);
            }
            file.items.push(item);
        }

        // A hosted body can name packages its file never imported: a type argument
        // is spelled by the CALLING file (`<package>.<Type>`, already canonical),
        // and a built-in package's body keeps calling its own package's helpers.
        // So a hosting file's resolution scope is widened with every other file's
        // imports. The file's OWN bindings always win — an alias the declaring
        // file binds is the one its template bodies mean — and a foreign import
        // whose alias is already taken there is still made known, under a binding
        // no source can spell, when nothing else imports its package: a
        // package-qualified type resolves through any binding of its package.
        for index in hosts {
            let file = &mut files[index];
            let mut bindings: HashSet<String> = file
                .imports
                .iter()
                .map(|import| import.binding_name().to_string())
                .collect();
            let mut shadowed = Vec::new();
            for import in self.source.files.iter().flat_map(|file| &file.imports) {
                if bindings.insert(import.binding_name().to_string()) {
                    file.imports.push(import.clone());
                } else {
                    shadowed.push(import);
                }
            }
            let mut packages: HashSet<String> = file
                .imports
                .iter()
                .map(|import| import.package_name().to_string())
                .collect();
            for import in shadowed {
                if packages.insert(import.package_name().to_string()) {
                    file.imports.push(crate::ast::Import {
                        module: import.module.clone(),
                        alias: Some(format!(
                            "{}{}",
                            crate::internal_name::INTERNAL_SIGIL,
                            import.package_name()
                        )),
                        line: import.line,
                    });
                }
            }
        }

        HirProject {
            name: self.source.name.clone(),
            files,
        }
    }

    fn report(&mut self, rule: &str, detail: &str, line: usize) {
        self.had_error = true;
        // bug-642: inside a built-in package's body, report at the user call that
        // instantiated it — the body's own file and lines are not the user's.
        if let Some(site) = self.current_instantiation_site.clone() {
            let path = self.project_dir.join(&site.file);
            rules::show_diagnostic(rule, &site.describe(detail), &path, site.line, 1, 1);
            return;
        }
        // Prefer the file whose body is currently being lowered (bug-107); fall
        // back to the first project file only when the frame is unknown.
        let relative = self
            .current_file
            .clone()
            .or_else(|| self.source.files.first().map(|file| file.path.clone()));
        let path = relative
            .map(|rel| self.project_dir.join(rel))
            .unwrap_or_else(|| self.project_dir.join("src/main.mfb"));
        rules::show_diagnostic(rule, detail, &path, line, 1, 1);
    }

    /// Charge one new concrete instantiation (function or user type) against the
    /// global total-instantiation budget, which bounds *wide* fan-out the per-path
    /// depth cap cannot (bug-399). Returns `false` — halting the caller — once any
    /// instantiation limit has stopped enumeration: either this budget (reporting a
    /// single `TYPE_INSTANTIATION_BUDGET_EXCEEDED` the first time it is exhausted)
    /// or the depth cap (which sets `instantiation_limit_reached` at its own site).
    /// After the first stop it returns `false` with no further work or diagnostics,
    /// so the remaining (exponential) tree is never explored.
    fn charge_instantiation(&mut self, name: &str, line: usize) -> bool {
        if self.instantiation_limit_reached {
            return false;
        }
        if self.total_instantiations >= MAX_TOTAL_INSTANTIATIONS {
            self.instantiation_limit_reached = true;
            self.report(
                "TYPE_INSTANTIATION_BUDGET_EXCEEDED",
                &format!(
                    "Monomorphization exceeded the {MAX_TOTAL_INSTANTIATIONS}-instantiation \
                     budget at `{name}`: the program fans out into too many distinct generic \
                     instantiations. This usually means a generic recurses through two or more \
                     type-widening self-calls."
                ),
                line,
            );
            return false;
        }
        self.total_instantiations += 1;
        true
    }

    fn report_instantiation_too_deep(&mut self, name: &str, line: usize) {
        self.report(
            "TYPE_INSTANTIATION_TOO_DEEP",
            &format!(
                "Template instantiation of `{name}` exceeds the {MAX_TEMPLATE_INSTANTIATION_DEPTH} level limit."
            ),
            line,
        );
    }
}

#[cfg(test)]
mod tests;
