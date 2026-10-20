//! plan-176-M: the late pass — which builtin packages a project references, and injecting their
//! synthetic files. `Registry` and `build()` live in `codegen/registry/mod.rs`.

use super::*;

/// The project facts the builtin-source injectors gate on: which packages the
/// program `IMPORT`s, and which call callees it names.
///
/// plan-106-D: both pipelines inject the same builtin sources — the AST one
/// (`resolver::augment_project`, before monomorphization) and the former source checker's HIR
/// one — and the gates read exactly these two things. Collecting them once, from
/// either domain, is what lets ONE injector serve both. The alternative was a
/// second copy of `is_imported_by` plus the ~100-line `references_any` AST walk
/// per domain, which is the duplication this whole plan exists to remove.
#[derive(Clone, Default)]
pub(crate) struct ProjectView {
    packages: std::collections::HashSet<String>,
    /// Every call callee reduced to its final segment across `::` and `.`.
    ///
    /// A callee may be source-qualified (`strings::toScalars`), aliased
    /// (`s::toScalars`), or already canonicalized to the dotted form
    /// (`strings.toScalars`), so [`HelperGate::WhenUsed`] matched the final
    /// segment. Reducing once at collection is the same match, done once per
    /// project instead of once per gate. Over-matching (a user's own
    /// `toScalars`) only injects a helper unnecessarily, never wrongly.
    callees: std::collections::HashSet<String>,
    /// Every constructed record type, reduced the same way (plan-172-K) — what a
    /// [`HelperGate::WhenUsedOrConstructed`] gate matches. Kept apart from `callees`
    /// so a record that happens to share a gated function's name opens nothing.
    constructed: std::collections::HashSet<String>,
    /// Every identifier a type ANNOTATION spells (plan-164-B) — a binding's, a
    /// parameter's, a return type's, a record field's — one entry a word, so
    /// `List OF canvas::PolyCurve` contributes `List`, `OF`, `canvas` and `PolyCurve`.
    /// With `constructed`, what a [`HelperGate::WhenCalledOrNamed`] gate matches.
    annotated: std::collections::HashSet<String>,
}

/// The identifiers in a type spelling, each added to `out`.
pub(super) fn note_type(spelling: &str, out: &mut std::collections::HashSet<String>) {
    for word in spelling.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if !word.is_empty() && !out.contains(word) {
            out.insert(word.to_string());
        }
    }
}

impl ProjectView {
    pub(crate) fn of_ast(ast: &crate::ast::AstProject) -> Self {
        let mut view = Self::default();
        for file in &ast.files {
            view.absorb_ast_file(file);
        }
        view
    }

    pub(crate) fn of_hir(hir: &crate::hir::HirProject) -> Self {
        let mut view = Self::default();
        for file in &hir.files {
            view.absorb_imports(&file.imports);
            for item in &file.items {
                hir_item_callees(item, &mut view.callees, &mut view.constructed);
                hir_item_types(item, &mut view.annotated);
            }
        }
        view
    }

    /// Fold a newly injected source file into the view.
    ///
    /// The late passes run in a chain (`http` before `net` before `encoding`)
    /// precisely because an injected companion carries its OWN imports — the
    /// injected `http` source `IMPORT net`s, and `net`'s gate must see it. Each
    /// pass therefore re-reads a view that includes what the previous one added,
    /// exactly as the old chain re-read a progressively augmented `AstProject`.
    pub(crate) fn absorb_ast_file(&mut self, file: &crate::ast::AstFile) {
        self.absorb_imports(&file.imports);
        for item in &file.items {
            item_callees(item, &mut self.callees, &mut self.constructed);
            item_types(item, &mut self.annotated);
        }
    }

    fn absorb_imports(&mut self, imports: &[crate::ast::Import]) {
        for import in imports {
            self.packages.insert(import.package_name().to_string());
        }
    }

    /// Whether the program imports `package`.
    pub(crate) fn imports(&self, package: &str) -> bool {
        self.packages.contains(package)
    }

    /// Whether the program calls any of `names` (a [`HelperGate::WhenUsed`] gate).
    pub(crate) fn references_any(&self, names: &[&'static str]) -> bool {
        names.iter().any(|name| self.callees.contains(*name))
    }

    /// Whether the program constructs a record of any of `types` (a
    /// [`HelperGate::WhenUsedOrConstructed`] gate).
    pub(crate) fn constructs_any(&self, types: &[&'static str]) -> bool {
        types.iter().any(|name| self.constructed.contains(*name))
    }

    /// Whether the program names any of `types` as a type — constructs one, or spells
    /// one in an annotation (a [`HelperGate::WhenCalledOrNamed`] gate).
    pub(crate) fn names_any(&self, types: &[&'static str]) -> bool {
        types
            .iter()
            .any(|name| self.constructed.contains(*name) || self.annotated.contains(*name))
    }
}

/// Inject a late-pass package's source companion (`http`/`net`/`encoding`) into
/// an AST project, with its gated helpers (see [`late_pass_files`]).
///
/// These three are skipped by [`Registry::augment_project`]'s single pass and
/// injected in their own dependency order afterwards, because each carries
/// transitive imports the single pass cannot see (`http`'s companion `IMPORT
/// net`s). plan-106-D: one implementation, since all three bodies were identical.
pub(crate) fn inject_late_pass(
    ast: &crate::ast::AstProject,
    package: &str,
    label: &str,
    doc: &str,
) -> Result<crate::ast::AstProject, ()> {
    let files = late_pass_files(package, label, doc, &ProjectView::of_ast(ast))?;
    let mut augmented = ast.clone();
    for file in files {
        if !augmented
            .files
            .iter()
            .any(|present| present.path == file.path)
        {
            augmented.files.push(file);
        }
    }
    Ok(augmented)
}

/// The same injection onto the elaborated project the former source checker consumes.
pub(crate) fn inject_late_pass_hir(
    hir: &crate::hir::HirProject,
    package: &str,
    label: &str,
    doc: &str,
) -> Result<crate::hir::HirProject, ()> {
    let files = late_pass_files(package, label, doc, &ProjectView::of_hir(hir))?;
    let mut augmented = hir.clone();
    for file in files {
        if !augmented
            .files
            .iter()
            .any(|present| present.path == file.path)
        {
            augmented.files.push(crate::hir::elaborate_file(&file));
        }
    }
    Ok(augmented)
}

/// The files a late pass injects: the package's `get_mfb` companion, then each of its
/// [`HelperGate::WhenUsed`] helpers whose gate the late pass's view opens.
///
/// `get_mfb` renders no gated helper, so a package whose members all rewrite onto gated
/// helpers (`compress`) would otherwise inject only its `IMPORT` lines. Its gates are
/// opened here by the calls in the source earlier passes injected — canvas's PNG decoder
/// calling `compress::zlibDecode` (plan-137-C) — which the generic pass never sees. The
/// callers skip a file whose path the project already has, so a helper the generic pass
/// injected for a program that imports the package itself keeps its position and is not
/// injected twice.
fn late_pass_files(
    package: &str,
    label: &str,
    doc: &str,
    view: &ProjectView,
) -> Result<Vec<crate::ast::AstFile>, ()> {
    let Some(pkg) = registry().resolve_package(package) else {
        return Ok(Vec::new());
    };
    if !pkg.is_imported_by(view) {
        return Ok(Vec::new());
    }
    let mut files = vec![crate::ast::parse_source_internal(
        std::path::Path::new(label),
        doc,
        &pkg.get_mfb(),
    )?];
    for helper in pkg.helpers() {
        let (Some(body), HelperGate::WhenUsed(names)) = (helper.body, &helper.gate) else {
            continue;
        };
        if !view.references_any(names) {
            continue;
        }
        // Labelled and parsed exactly as `Registry::synthetic_files` does, so the path
        // matches a copy the generic pass already injected.
        let label = format!("<builtin-{}>", helper.name);
        let doc = format!("builtins/{}.mfb", helper.name);
        files.push(crate::ast::parse_source_builtin(
            std::path::Path::new(&label),
            &doc,
            &format!("{}\n", body.trim_end()),
            pkg.import_name(),
        )?);
    }
    Ok(files)
}

/// The final segment of a callee across `::` and `.` — the form a `WhenUsed` gate
/// matches.
pub(super) fn short_callee(callee: &str) -> &str {
    callee
        .rsplit("::")
        .next()
        .unwrap_or(callee)
        .rsplit('.')
        .next()
        .unwrap_or(callee)
}
