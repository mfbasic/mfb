//! Clean-room builtin registry (north star — see `planning/todo.md`).
//!
//! This is a **standalone** replacement for the descriptor vocabulary in
//! `target::shared::registry`, built in parallel and deliberately NOT wired into
//! the existing pipeline. Nothing here answers the old query surface
//! (`REGISTRY.function`, `DefaultResolver`, …); builtin packages migrate onto this
//! shape one at a time, and the pipeline flips to it only once enough have moved.
//!
//! The shape it commits to — the whole reason it exists — is:
//!
//! ```text
//! Registry ─┬─ RegistryPackage ─┬─ RegistryFunction ── [Implementation, …]  (>= 1)
//! ```
//!
//! A function is **a name + docs + one-or-more fully-specified implementations**.
//! Each [`Implementation`] carries its *own* signature (params + return type), its
//! lowering, its realization ([`Body`]), and its errors — so overloading is just
//! "more than one implementation" and needs no `Custom`/resolver side-channel. This
//! is the endpoint the `overloads`-carries-`Implementation` and unified-
//! `implementations`-array notes in `planning/todo.md` step toward.
//!
//! Unlike `target::shared::registry` (a tree of `const`/`static` struct literals),
//! this registry is **built imperatively** — `add_package` / `add_function` — then
//! frozen once behind a `OnceLock` ([`registry`]). The imperative builder is the
//! point: a package registers itself in one readable block instead of a nested
//! struct literal plus a sidecar resolver.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// A builder-driven **inline** lowering — the single sanctioned inline shape:
/// given the caller's `CodeBuilder`, the
/// call's **pre-lowered** `ValueResult` args (each carrying its
/// source `NirValue` for NIR-structural
/// analyses like bounds-check elision), and an [`AbiCtx`], emit the call inline and
/// return where the result value lives. The body reads its `ValueResult` args and
/// never re-lowers or frees them — the dispatch owns arg acquisition/lifetime.
pub(crate) type AbiInline =
    for<'a> fn(
        &mut crate::codegen::engine::builder::CodeBuilder<'a>,
        &[crate::codegen::engine::builder::ValueResult],
        &AbiCtx<'a>,
    ) -> Result<crate::codegen::engine::builder::ValueResult, String>;

/// A builder-driven **shared-function** lowering: the *same body shape* as
/// [`AbiInline`], but the wrapper binds its
/// args to the incoming ABI param registers and emits it once as a shared
/// `_mfb_rt_*` helper (emit-once, called by `bl` from every call site).
pub(crate) type AbiFunction =
    for<'a> fn(
        &mut crate::codegen::engine::builder::CodeBuilder<'a>,
        &[crate::codegen::engine::builder::ValueResult],
        &AbiCtx<'a>,
    ) -> Result<crate::codegen::engine::builder::ValueResult, String>;

/// A [`Body::Mfb`] member's optional native **fast path** — the plan-95
/// `target::shared::registry::MfbFastPath` shape. Given the builder, the
/// `#pkg_<name>$<TypeArgs>` monomorph target, and the call args, it either lowers
/// natively (`Ok(Some(_))`) or **declines** (`Ok(None)`), in which case the caller
/// instantiates the `.mfb` `body` instead. Selected by whether the monomorph
/// instantiation qualifies (a computed axis), so it rides on the `Mfb` body rather
/// than being its own realization kind or a second overload.
pub(crate) type MfbFastPath =
    for<'a> fn(
        &mut crate::codegen::engine::builder::CodeBuilder<'a>,
        &str,
        &[crate::target::shared::nir::NirValue],
    ) -> Result<Option<crate::codegen::engine::builder::ValueResult>, String>;

// A [`Parameter`]'s type is [`crate::types::ParameterType`] — the compiler-wide type
// vocabulary (see that module for why it lives outside `codegen`). Imported for the
// registry's own use; not re-exported, so callers name it as `crate::types::ParameterType`.
use crate::codegen::builtins::{NoTypeKinds, TypeKinds};
use crate::intern::Symbol;
use crate::types::ParameterType;

mod ast_scan;
mod descriptors;
mod hir_scan;
mod late_pass;
mod lookup;
mod package;
mod qualify;
mod unify;
use ast_scan::*;
pub(crate) use descriptors::*;
use hir_scan::*;
pub(crate) use late_pass::*;
pub(crate) use lookup::*;
pub(crate) use package::*;
use qualify::*;
pub(crate) use unify::*;

pub(crate) struct ResolvedFunc<'r> {
    pub(crate) package: &'r RegistryPackage,
    pub(crate) function: &'r RegistryFunction,
}

pub(crate) enum ResolvedType<'r> {
    Record(&'r RegistryRecord),
    Union(&'r RegistryUnion),
    Enum(&'r RegistryEnum),
    Resource(&'r RegistryResource),
}

/// The registry: an ordered set of packages. Built imperatively, then
/// frozen (see [`registry`]).
#[derive(Debug, Default)]
pub(crate) struct Registry {
    packages: Vec<RegistryPackage>,
}

impl Registry {
    /// An empty registry.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add a fully-built package (built with [`RegistryPackage::new`] and filled
    /// with its records / unions / helpers / functions).
    pub(crate) fn add_package(&mut self, package: RegistryPackage) -> &mut Self {
        self.packages.push(package);
        self
    }

    /// All packages, in registration order.
    pub(crate) fn packages(&self) -> &[RegistryPackage] {
        &self.packages
    }

    // Lookup functions

    /// The package addressed by `name` — either a bare import name (`"csv"`) or the
    /// package half of a qualified call/type (`"csv.parse"` → `csv`). Membership of
    /// the function/type after the dot is not checked here; use [`resolve_func`] /
    /// [`resolve_type`] for that.
    pub(crate) fn resolve_package(&self, name: &str) -> Option<&RegistryPackage> {
        let pkg_name = name.split_once('.').map_or(name, |(pkg, _)| pkg);
        self.packages.iter().find(|p| p.import_name == pkg_name)
    }

    /// The advisory carried by the `enum_name.member` value of the package whose
    /// injected source is `builtins/<import_name>.mfb` — `None` when the package
    /// or enum is unknown, the member is not a variant, or the variant carries no
    /// advisory. Keyed by the owning package (not a bare enum-name scan) so a user
    /// enum that happens to share a builtin enum's name can never inherit its
    /// advisory. Consulted by `ir::verify` when it resolves a user-source enum
    /// member access (`check_enum_member_advisory`).
    pub(crate) fn enum_variant_advisory(
        &self,
        import_name: &str,
        enum_name: &str,
        member: &str,
    ) -> Option<EnumAdvisory> {
        let package = self
            .packages
            .iter()
            .find(|p| p.import_name == import_name)?;
        let r#enum = package.enums().iter().find(|e| e.name == enum_name)?;
        r#enum
            .variants
            .iter()
            .find(|variant| variant.name == member)?
            .advisory
    }

    pub(crate) fn resolve_func(&self, qualified: &str) -> Option<ResolvedFunc<'_>> {
        let (pkg_name, func_name) = qualified.split_once('.')?;
        let package = self.packages.iter().find(|p| p.import_name == pkg_name)?;
        let function = package.function(func_name)?;
        Some(ResolvedFunc { package, function })
    }

    pub(crate) fn resolve_type(&self, qualified: &str) -> Option<ResolvedType<'_>> {
        let (pkg_name, type_name) = qualified.split_once('.')?;
        let package = self.packages.iter().find(|p| p.import_name == pkg_name)?;

        if let Some(record) = package.records().iter().find(|r| r.name == type_name) {
            return Some(ResolvedType::Record(record));
        }
        if let Some(union) = package.unions().iter().find(|u| u.name == type_name) {
            return Some(ResolvedType::Union(union));
        }
        if let Some(r#enum) = package.enums().iter().find(|e| e.name == type_name) {
            return Some(ResolvedType::Enum(r#enum));
        }
        if let Some(resource) = package.resources().iter().find(|r| r.name == type_name) {
            return Some(ResolvedType::Resource(resource));
        }
        None
    }

    /// Inject every package's reassembled source into `ast`, for
    /// each package the program imports — the registry-driven replacement for the
    /// per-package `augmented_project` functions. A package that is not imported, or has
    /// nothing to inject (empty [`get_mfb`](RegistryPackage::get_mfb)), contributes no
    /// file. The synthetic path/doc labels match the pre-migration convention
    /// (`<builtin-csv>` / `builtins/csv.mfb`).
    ///
    /// `debug` selects each [`debug_split`](RegistryHelper::debug_split) helper's half:
    /// the build passes its `--debug` flag (plan-130-E).
    pub(crate) fn augment_project(
        &self,
        ast: &crate::ast::AstProject,
        debug: bool,
    ) -> Result<crate::ast::AstProject, ()> {
        let synthetic_files = self.synthetic_files(&ProjectView::of_ast(ast), debug)?;
        if synthetic_files.is_empty() {
            return Ok(ast.clone());
        }
        let mut augmented = ast.clone();
        augmented.files.extend(synthetic_files);
        Ok(augmented)
    }

    /// The same injection, onto the elaborated project the former source checker consumes
    /// (plan-106-D). One decision procedure, two thin adapters — the synthetic
    /// files are parsed from source and elaborated, exactly as the AST pipeline's
    /// are parsed and then elaborated downstream.
    pub(crate) fn augment_hir_project(
        &self,
        hir: &crate::hir::HirProject,
    ) -> Result<crate::hir::HirProject, ()> {
        // The HIR chain serves only in-process tests of normal builds.
        let synthetic_files = self.synthetic_files(&ProjectView::of_hir(hir), false)?;
        if synthetic_files.is_empty() {
            return Ok(hir.clone());
        }
        let mut augmented = hir.clone();
        augmented
            .files
            .extend(synthetic_files.iter().map(crate::hir::elaborate_file));
        Ok(augmented)
    }

    /// Every builtin-package source file whose injection gate `view` opens, in
    /// dependency order.
    fn synthetic_files(
        &self,
        view: &ProjectView,
        debug: bool,
    ) -> Result<Vec<crate::ast::AstFile>, ()> {
        let mut synthetic_files = Vec::new();

        for package in self.packages() {
            // `encoding` is a transitive dependency of the non-migrated `crypto` and
            // `strings` packages, whose source is injected *after* this generic pass
            // and contributes its own `IMPORT encoding` (and calls
            // `encoding::hexDecode`/`utf32Encode`). A single pass over the
            // pre-injection AST cannot see that transitive import, so `encoding` is
            // injected by its own dedicated late pass (`encoding::augmented_project`,
            // run after crypto/strings in the lowering pipeline). That late pass now
            // injects the *identical* generic `RegistryPackage::get_mfb` assembly this
            // pass would produce — only the injection position differs. Skipping it
            // here also prevents a double injection when a program imports `encoding`
            // directly.
            if package.import_name() == "encoding" {
                continue;
            }
            // `net` (and `http`) are injected by their own dedicated late passes
            // (`codegen::builtins::{net,http}::augmented_project`), for the same
            // transitivity reason as `encoding`: `http`'s injected source `IMPORT
            // net`s, and this single pass over the pre-injection AST cannot see that
            // transitive import. Skipping them here also prevents a double injection
            // when a program imports `net`/`http` directly.
            if matches!(package.import_name(), "net" | "http") {
                continue;
            }
            // `color` is injected by its own dedicated late pass
            // (`codegen::builtins::color::augmented_project`), for the same
            // transitivity reason: since plan-122-B `canvas`'s injected companion
            // carries `IMPORT color` and calls `color::toLinear`/`fromLinear` from
            // its blend and gradient helpers, and this single pass over the
            // pre-injection AST — where a canvas program has written only
            // `IMPORT canvas` — cannot see that. Skipping it here also prevents a
            // double injection when a program imports `color` directly.
            if package.import_name() == "color" {
                continue;
            }
            // `collections` is injected by its own dedicated pass at PARSE time
            // (`codegen::builtins::collections::augmented_project`, run by
            // `parse_project`): its members are source GENERICS the monomorphizer
            // must see to instantiate, long before this ir-lower-time pass runs.
            // That pass injects the identical `get_mfb` assembly; skipping it here
            // prevents a double injection for a program that imports it directly.
            if package.import_name() == "collections" {
                continue;
            }
            if !package.is_imported_by(view) {
                continue;
            }
            let source = package.get_mfb_for(debug);
            if source.is_empty() {
                continue;
            }
            let label = format!("<builtin-{}>", package.import_name());
            let doc = format!("builtins/{}.mfb", package.import_name());
            let file =
                crate::ast::parse_source_internal(std::path::Path::new(&label), &doc, &source)?;
            synthetic_files.push(file);
        }

        // plan-99: gated helper chunks — the `strings` scalar-seam Unicode table
        // (`WhenUsed`) and the `term`↔`astrings` bridge (`WhenImported`) — inject as
        // their own synthetic files, deduped by helper `name` so a chunk reached
        // through several packages is injected once. `Always` bodies are *not* here
        // (they already rendered inline in each package's `get_mfb` above);
        // `import_name` edges carry no body (ordering-only). Evaluated in package
        // registration order, then helper add order.
        let mut injected_helpers: std::collections::HashSet<&'static str> =
            std::collections::HashSet::new();
        for package in self.packages() {
            let package_imported = package.is_imported_by(view);
            for helper in package.helpers() {
                let Some(body) = helper.body else {
                    continue; // an `import_name` ordering edge — no source to inject.
                };
                let gate_open = match helper.gate {
                    // rendered inline in `get_mfb_for`.
                    HelperGate::Always
                    | HelperGate::NormalBuildOnly
                    | HelperGate::DebugBuildOnly => false,
                    // `WhenUsed` fires only when the OWNING package is imported and a
                    // gated member is referenced (the `strings` scalar-seam gate).
                    HelperGate::WhenUsed(names) => package_imported && view.references_any(names),
                    // `WhenImported` is a cross-package bridge keyed on `other` being
                    // imported — the OWNING package need NOT be imported (an
                    // `astrings`-only program does not import `strings`, yet the injected
                    // `astrings` companion calls the `strings` scalar seam, so the seam
                    // must ride in). `other` is matched by raw import name so it works for
                    // a non-registry package too (`astrings`/`term`, not yet migrated).
                    HelperGate::WhenImported(other) => view.imports(other),
                    // `WhenBothImported` is a cross-package bridge whose body references
                    // BOTH packages' surface, so it must inject only when both are
                    // imported (the `term`↔`astrings` `drawText(AttributedString)`
                    // bridge — legacy `term::bridge_uses_package`). Over-injecting on
                    // either alone would drag the other package's surface in as dead
                    // code, shifting the injected `.ir`/`build.log` of a program that
                    // imports only one.
                    HelperGate::WhenBothImported(a, b) => view.imports(a) && view.imports(b),
                    // plan-172-K: a chunk a record type opens as well as a call, and
                    // the chunk that stands in for it when nothing constructs one.
                    HelperGate::WhenUsedOrConstructed(names, types) => {
                        package_imported
                            && (view.references_any(names) || view.constructs_any(types))
                    }
                    HelperGate::WhenNotConstructed(types) => {
                        package_imported && !view.constructs_any(types)
                    }
                    // plan-164-B: the pair keyed on a type being NAMED at all.
                    HelperGate::WhenCalledOrNamed(names, types) => {
                        package_imported && (view.references_any(names) || view.names_any(types))
                    }
                    HelperGate::WhenNeitherCalledNorNamed(names, types) => {
                        package_imported && !view.references_any(names) && !view.names_any(types)
                    }
                };
                if !gate_open || !injected_helpers.insert(helper.name) {
                    continue;
                }
                let label = format!("<builtin-{}>", helper.name);
                let doc = format!("builtins/{}.mfb", helper.name);
                // The chunk is labelled by the HELPER, but it is the OWNING
                // package's source, so the package is handed over explicitly --
                // the label alone would leave its declarations unqualified
                // (bug-480 Phase 4b).
                let file = crate::ast::parse_source_builtin(
                    std::path::Path::new(&label),
                    &doc,
                    &format!("{}\n", body.trim_end()),
                    package.import_name(),
                )?;
                synthetic_files.push(file);
            }
        }

        Ok(synthetic_files)
    }

    // Generic-dispatch queries. Each answers, for a call/type, the fact the old
    // `REGISTRY`-based generic dispatch answered, so a caller can dual-path
    // `registry().X(name).or(old(name))`. `None`/`false` means "no migrated package
    // owns this", i.e. fall through to the old path.

    /// Whether a migrated package declares the call `qualified` (`"csv.parse"`).
    pub(crate) fn is_member(&self, qualified: &str) -> bool {
        self.resolve_func(qualified).is_some()
    }

    /// Whether the migrated call `qualified` is an **internal-only** member — a native
    /// primitive resolvable only from toolchain-provided (`internal`) source
    /// (`astrings.readSpans`/`writeSpans`/`scalarLen`). The registry half of
    /// `builtins::is_internal_only_call`, replacing the deleted per-package
    /// `astrings::is_astrings_internal_call`.
    pub(crate) fn is_internal_only_member(&self, qualified: &str) -> bool {
        self.resolve_func(qualified)
            .is_some_and(|resolved| resolved.function.internal_only)
    }

    /// The import name of the migrated package that owns `qualified`, or `None`.
    pub(crate) fn owning_package(&self, qualified: &str) -> Option<&'static str> {
        self.resolve_func(qualified)
            .map(|resolved| resolved.package.import_name)
    }

    /// The `(min, max)` argument arity of the migrated call `qualified`, or `None`.
    /// `min` counts the required (non-defaulted) params; `max` is the widest overload.
    pub(crate) fn arity(&self, qualified: &str) -> Option<(usize, usize)> {
        self.resolve_func(qualified)
            .and_then(|resolved| resolved.function.arity())
    }

    /// Whether the migrated call `qualified` declares `error_name` among any of its
    /// implementations' errors — the half of the `raise_error` "a builtin must declare
    /// the errors it raises" check.
    pub(crate) fn declares_error(&self, qualified: &str, error_name: &str) -> bool {
        self.resolve_func(qualified)
            .is_some_and(|resolved| resolved.function.declares_error(error_name))
    }

    /// Whether `name` is a value type (`EXPORT TYPE`/`UNION`/`ENUM`) declared by any
    /// migrated package (`json.JsonNull`/`regex.Group`), a source-declared/opaque type
    /// (`datetime.Instant`, thread's `Thread`/`ThreadWorker`), or a **parametric**
    /// spelling of an opaque type (`Thread OF Msg TO Out`) whose head token before the
    /// first ` OF ` names a source-declared type.
    pub(crate) fn is_builtin_type(&self, name: &str) -> bool {
        // The head token of a parametric spelling (`Thread OF …`): a source-declared
        // opaque type used with type arguments. `List`/`Map`/… are never source types,
        // so their `X OF …` spellings are correctly not matched here.
        //
        // plan-111-F: taken from the one grammar's own variants rather than a
        // local `split_once(" OF ")`.
        //
        // `UserOf` alone is NOT enough: `Thread OF Integer TO String` parses to
        // `ThreadHandle`, not `UserOf`, so a `UserOf`-only read silently stopped
        // recognizing the thread handles — which are precisely the
        // "source-declared opaque type used with type arguments" this exists for
        // (caught by `thread::tests::opaque_handle_types_recognized`). `List`,
        // `Set`, `Map` and `Result` are never source types, so they still yield
        // no head, exactly as the string split did.
        let head_owned = match crate::types::ParameterType::declared(name) {
            crate::types::ParameterType::UserOf(base, _) => Some(base.resolve().to_string()),
            crate::types::ParameterType::ThreadHandle { worker, .. } => Some(
                if worker {
                    crate::types::THREAD_WORKER_TYPE
                } else {
                    crate::types::THREAD_TYPE
                }
                .to_string(),
            ),
            _ => None,
        };
        let head = head_owned.as_deref();
        // bug-480 Phase 4b: a builtin value type is named `crypto.Sealed` now, while
        // a registry row still carries the bare member id it declares. Accept both --
        // callers hand this whatever the type system holds, which is the qualified
        // form, and matching only the bare id made every such probe answer `false`.
        let matches = |package: &RegistryPackage, row: &str| {
            row == name
                || name
                    .strip_prefix(package.import_name())
                    .and_then(|rest| rest.strip_prefix('.'))
                    .is_some_and(|leaf| leaf == row)
        };
        self.packages().iter().any(|package| {
            package
                .records()
                .iter()
                .any(|record| matches(package, record.name))
                || package.unions().iter().any(|union| matches(package, union.name))
                || package
                    .enums()
                    .iter()
                    .any(|r#enum| matches(package, r#enum.name))
                // `datetime`'s value records/enums live in its injected companion source.
                || package
                    .source_types()
                    .iter()
                    .any(|source| matches(package, source))
                || head.is_some_and(|head| package.source_types().contains(&head))
        })
    }

    /// Rewrite every descriptor-held TYPE REFERENCE from its bare leaf spelling to
    /// the package-qualified identity a value type now carries (bug-480 Phase 4b).
    ///
    /// The descriptors are written bare — `ParameterType::named(KEYPAIR_TYPE)`,
    /// where `KEYPAIR_TYPE` is `"KeyPair"` — because within `crypto` that IS the
    /// name, and the governing rule says a package's own members need no prefix.
    /// Once the declared identity is `crypto.KeyPair`, a signature still saying
    /// `KeyPair` denotes nothing: observed as `native record type 'KeyPair' does
    /// not resolve` out of codegen, and as `MATCH on open type Attribute` where a
    /// bare-typed scrutinee met qualified `CASE` arms.
    ///
    /// Done as one pass here rather than at ~400 construction sites because only
    /// the type FIELDS need rewriting — `name` is `&'static str` and stays the
    /// bare member id that `resolve_type` matches after splitting the qualifier.
    /// One choke point also means one place where the owner rule is stated:
    ///
    ///   1. the package that declares the leaf, when it is this package's own —
    ///      so `http`'s `Stream` is `http.Stream` and `process`'s is
    ///      `process.Stream`, which is the whole of bug-481;
    ///   2. otherwise the single package that declares it, for a genuine
    ///      cross-package reference (`http::Response` naming `net::Url`);
    ///   3. otherwise unchanged — a primitive, a generic parameter, or an
    ///      ambiguous leaf no package can claim.
    fn qualify_value_type_references(&mut self) {
        // leaf -> the packages declaring it. Built before any rewrite so a
        // cross-package reference resolves against the whole registry.
        let mut owners: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for package in &self.packages {
            let pkg = package.import_name.to_string();
            let mut add = |leaf: &str| {
                owners
                    .entry(leaf.to_string())
                    .or_default()
                    .push(pkg.clone());
            };
            for record in &package.records {
                add(record.name);
            }
            for union in &package.unions {
                add(union.name);
            }
            for r#enum in &package.enums {
                add(r#enum.name);
            }
            for source_type in &package.source_types {
                add(source_type);
            }
        }

        let packages = std::mem::take(&mut self.packages);
        self.packages = packages
            .into_iter()
            .map(|mut package| {
                let pkg = package.import_name;
                let map = |ty: &ParameterType| qualify_type_leaves(ty, pkg, &owners);
                // Record FIELD types are rewritten too (bug-484). They used to be
                // skipped, on the reasoning that a record round-trips through
                // injectable source where a qualifier is `::` and a `.` would parse
                // as field access. True, but it argues for rendering `::`, not for
                // leaving the field bare -- and `ParameterType::display` already does that
                // rewrite for every rendered type. Leaving them bare put a name that
                // is IMPORTED into the declaring package (`udp::Datagram`'s `from`
                // is net's `Address`) into the companion with no prefix, against the
                // governing rule, and left the parser to guess it back from the
                // file's imports. An ambiguous leaf it could not guess was left bare
                // and unresolved -- silently, which is the bug-483 class again.
                for record in &mut package.records {
                    for prop in &mut record.props {
                        prop.ty = qualify_type_leaves_for_source(&prop.ty, pkg, &owners);
                    }
                }
                for function in &mut package.functions {
                    for imp in &mut function.implementations {
                        for param in &mut imp.params {
                            param.ty = map(&param.ty);
                        }
                        imp.return_type = map(&imp.return_type);
                    }
                }
                package
            })
            .collect();
    }

    /// A `package.Type` reference (`"net.Url"`) resolved to the DECLARED type
    /// id when the migrated package declares it, else `None`.
    ///
    /// bug-480 Phase 4b changed what "the declared id" means. It used to be the
    /// bare member name (`net.Url` -> `Url`), because every builtin value type was
    /// declared bare in one flat top-level namespace. That namespace is why
    /// `http::Stream` (a union) and `process::Stream` (an enum) could not coexist
    /// (bug-481), and why a bare `Response` resolved from a consumer that should
    /// have had to write `http::Response`. A value type is now addressed
    /// `net.Url`, exactly as a RESOURCE has been since plan-97 — so this returns
    /// the qualified spelling unchanged, and the two `Stream`s are two names.
    pub(crate) fn qualified_builtin_type(&self, qualified: &str) -> Option<String> {
        // Match the resolved kind rather than discarding it: the member id it
        // carries is what the qualified spelling must be built from, and reading it
        // keeps the resolution honest (a row whose `name` disagreed with the
        // qualifier would otherwise pass silently).
        if let Some(resolved) = self.resolve_type(qualified) {
            let member = match resolved {
                ResolvedType::Record(record) => record.name,
                ResolvedType::Union(union) => union.name,
                ResolvedType::Enum(r#enum) => r#enum.name,
                ResolvedType::Resource(resource) => resource.name,
            };
            let (package, leaf) = qualified.split_once('.')?;
            assert_eq!(leaf, member, "registry row name disagrees with lookup");
            return Some(format!("{package}.{member}"));
        }
        // A source-declared value type (`datetime.Instant`) authored only in the
        // package's injected companion, not modeled as a record/union/enum.
        let (pkg_name, type_name) = qualified.split_once('.')?;
        self.packages()
            .iter()
            .find(|p| p.import_name == pkg_name)
            .filter(|p| p.source_types().contains(&type_name))
            .map(|_| qualified.to_string())
    }
}

/// The process-wide registry, built once on first access.
///
/// Mirrors the `OnceLock` freeze idiom in `src/unicode/runtime_tables.rs`: the
/// imperative [`build`] runs once, and every caller thereafter shares the frozen
/// `&'static Registry`.
pub(crate) fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(build)
}

/// One builtin record's layout: its package-qualified type and its fields in
/// declaration order.
pub(crate) type BuiltinRecordLayout = (ParameterType, Vec<(String, ParameterType)>);

/// Every builtin record's field layout, keyed by its package-qualified type
/// (`net.Address`), with every field type package-qualified the way the NIR spells
/// it: `net.PingResult`'s `address` is `net.Address`, not the bare local `Address` a
/// record prop carries so the companion source stays local.
///
/// plan-132 C3: the codegen `TypeModel` is built from a program's NIR types, which
/// carry a builtin record only when some file imports its package. A value still
/// reaches a program that imports nothing of the kind — `tcp::localAddress` hands a
/// `net.Address` to a file that imports only `tcp` — and a nominal the model does not
/// know is classified as a plain 8-byte scalar. `TypeModel` registers this table so a
/// builtin record always has its real layout.
pub(crate) fn builtin_record_layouts() -> &'static [BuiltinRecordLayout] {
    static LAYOUTS: OnceLock<Vec<BuiltinRecordLayout>> = OnceLock::new();
    LAYOUTS.get_or_init(|| {
        let registry = registry();
        // leaf -> the packages declaring it, built exactly as
        // `qualify_value_type_references` builds it, so a leaf qualifies here the
        // way it qualified there.
        let mut owners: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for package in registry.packages() {
            let pkg = package.import_name().to_string();
            let mut add = |leaf: &str| {
                owners
                    .entry(leaf.to_string())
                    .or_default()
                    .push(pkg.clone());
            };
            for record in package.records() {
                add(record.name);
            }
            for union in package.unions() {
                add(union.name);
            }
            for r#enum in package.enums() {
                add(r#enum.name);
            }
            for source_type in package.source_types() {
                add(source_type);
            }
        }
        registry
            .packages()
            .iter()
            .flat_map(|package| {
                let pkg = package.import_name();
                let owners = &owners;
                package.records().iter().map(move |record| {
                    (
                        ParameterType::named(&format!("{pkg}.{}", record.name)),
                        record
                            .props
                            .iter()
                            .map(|prop| {
                                (
                                    prop.name.to_string(),
                                    qualify_type_leaves_inner(&prop.ty, pkg, owners, true),
                                )
                            })
                            .collect(),
                    )
                })
            })
            .collect()
    })
}

/// Construct the registry by registering every migrated package.
///
/// The `example` package is an illustrative entry that exercises the shape (a
/// single-implementation function and a two-implementation overload); `csv` is the
/// first real package migrated off `target::shared::registry` — it registers itself
/// from its own module, `crate::codegen::builtins::csv`.
fn build() -> Registry {
    let mut r = Registry::new();
    crate::codegen::builtins::app::register(&mut r);
    crate::codegen::builtins::astrings::register(&mut r);
    crate::codegen::builtins::audio::register(&mut r);
    crate::codegen::builtins::big::register(&mut r);
    crate::codegen::builtins::bits::register(&mut r);
    crate::codegen::builtins::canvas::register(&mut r);
    crate::codegen::builtins::color::register(&mut r);
    crate::codegen::builtins::compress::register(&mut r);
    crate::codegen::builtins::csv::register(&mut r);
    crate::codegen::builtins::json::register(&mut r);
    crate::codegen::builtins::math::register(&mut r);
    crate::codegen::builtins::regex::register(&mut r);
    crate::codegen::builtins::strings::register(&mut r);
    crate::codegen::builtins::term::register(&mut r);
    crate::codegen::builtins::testing::register(&mut r);
    crate::codegen::builtins::process::register(&mut r);
    crate::codegen::builtins::datetime::register(&mut r);
    crate::codegen::builtins::encoding::register(&mut r);
    crate::codegen::builtins::errorcode::register(&mut r);
    crate::codegen::builtins::collections::register(&mut r);
    crate::codegen::builtins::money::register(&mut r);
    crate::codegen::builtins::os::register(&mut r);
    crate::codegen::builtins::fs::register(&mut r);
    crate::codegen::builtins::general::register(&mut r);
    crate::codegen::builtins::io::register(&mut r);
    crate::codegen::builtins::crypto::register(&mut r);
    crate::codegen::builtins::tls::register(&mut r);
    crate::codegen::builtins::tcp::register(&mut r);
    crate::codegen::builtins::udp::register(&mut r);
    crate::codegen::builtins::net::register(&mut r);
    crate::codegen::builtins::http::register(&mut r);
    crate::codegen::builtins::thread::register(&mut r);
    crate::codegen::builtins::vector::register(&mut r);
    r.qualify_value_type_references();
    r
}

#[cfg(test)]
mod qualification_tests {
    use super::*;
    use std::collections::HashMap;

    /// `net` and `udp` each declare a type of their own; `Socket` is declared by
    /// two packages, so a third package naming it bare declares no such type.
    fn owners() -> HashMap<String, Vec<String>> {
        HashMap::from([
            ("Address".to_string(), vec!["net".to_string()]),
            ("Datagram".to_string(), vec!["udp".to_string()]),
            (
                "Socket".to_string(),
                vec!["tcp".to_string(), "udp".to_string()],
            ),
        ])
    }

    fn for_source(ty: &str, package: &str) -> String {
        qualify_type_leaves_for_source(&ParameterType::declared(ty), package, &owners())
            .name()
            .into_owned()
    }

    fn for_signature(ty: &str, package: &str) -> String {
        qualify_type_leaves(&ParameterType::declared(ty), package, &owners())
            .name()
            .into_owned()
    }

    /// The governing rule, source side: a locally-declared name needs no prefix.
    /// `net` declares `Address`, `udp` declares `Datagram`, so each keeps the bare
    /// leaf in its own rendered companion — `regex`'s PRIVATE `#regex_Cont` is the
    /// case that matters, since prefixing it stopped the descriptor's field type
    /// matching the local declaration the parser produces (bug-484).
    #[test]
    fn a_rendered_field_leaves_a_local_name_bare() {
        assert_eq!(for_source("Address", "net"), "Address");
        assert_eq!(for_source("Datagram", "udp"), "Datagram");
        assert_eq!(for_source("List OF Datagram", "udp"), "List OF Datagram");
    }

    /// A signature is a type-system identity, never rendered back into source, so
    /// it carries the qualified id even for the declaring package's own types.
    #[test]
    fn a_signature_qualifies_its_own_package() {
        assert_eq!(for_signature("Address", "net"), "net.Address");
        assert_eq!(for_signature("Datagram", "udp"), "udp.Datagram");
    }

    /// A cross-package reference is spelled out by the descriptor author, and both
    /// passes then leave it exactly as written.
    #[test]
    fn a_spelled_out_cross_package_reference_is_left_alone() {
        assert_eq!(for_source("net.Address", "udp"), "net.Address");
        assert_eq!(for_signature("net.Address", "udp"), "net.Address");
        assert_eq!(
            for_source("List OF net.Address", "udp"),
            "List OF net.Address"
        );
    }

    /// Names that are not registry-declared types — scalar nominals, C ABI
    /// spellings, generic parameters — are left exactly as written, and must not
    /// trip the not-declared-here check.
    #[test]
    fn a_non_registry_leaf_is_untouched() {
        for leaf in [
            "Scalar",
            "Error",
            "ErrorLoc",
            "AttributedString",
            "CPtr",
            "T",
        ] {
            assert_eq!(for_source(leaf, "net"), leaf, "{leaf}");
            assert_eq!(for_signature(leaf, "net"), leaf, "{leaf}");
        }
    }

    /// bug-484: a bare leaf the referencing package does not declare is UNDEFINED,
    /// and how many other packages export it is irrelevant — one is not more
    /// resolvable than three, it is the same guess with a smaller search space.
    /// `udp` naming a bare `Address` (net's, single owner) is refused...
    #[test]
    #[should_panic(expected = "declares no such type")]
    fn a_bare_cross_package_leaf_is_refused_even_with_one_owner() {
        let _ = for_source("Address", "udp");
    }

    /// ...and so is the same shape in a signature.
    #[test]
    #[should_panic(expected = "declares no such type")]
    fn a_bare_cross_package_leaf_is_refused_in_a_signature() {
        let _ = for_signature("Address", "udp");
    }

    /// Several owners is the same error, not a different one.
    #[test]
    #[should_panic(expected = "declares no such type")]
    fn a_bare_leaf_with_several_owners_is_refused() {
        let _ = for_source("Socket", "http");
    }

    /// A shared leaf the referencing package DOES declare is local, and local
    /// always wins: `Socket` inside `tcp` is tcp's. That other packages export the
    /// same leaf never makes it ambiguous — the local declaration decides.
    #[test]
    fn a_shared_leaf_the_referencing_package_declares_is_local() {
        assert_eq!(for_source("Socket", "tcp"), "Socket");
        assert_eq!(for_signature("Socket", "tcp"), "tcp.Socket");
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "tests/raw_result_block_ownership.rs"]
mod raw_result_block_ownership;
