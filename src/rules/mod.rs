use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// How many located diagnostics `show_diagnostic` renders in full before the
/// rest are only counted. Every rendered diagnostic echoes up to three source
/// lines, so with no ceiling a source provoking one error per line turned
/// 240 KB of input into 10 GB of stderr (audit-3 FE-03 / bug-505). No golden
/// in the tree records more than 22; a developer reads far fewer than 100.
/// `report_suppressed_diagnostics` prints the withheld count once the stream is
/// complete.
pub const MAX_RENDERED_DIAGNOSTICS: usize = 100;

/// Located diagnostics handed to `show_diagnostic` so far in this process.
static SEEN: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
thread_local! {
    /// Set by [`collecting_diagnostics`] for the duration of one closure.
    static COLLECTED: std::cell::RefCell<Option<Vec<(String, String)>>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// bug-697: mangled instantiated-generic symbol (`Box$Integer`, as
    /// `ParameterType::name()` renders it once monomorphization has replaced the
    /// template with its concrete nominal) -> the template spelling
    /// (`Box OF Integer`) a user actually wrote, fully recursive so a nested
    /// instantiation (`Holder$Holder$Integer`) maps to a fully-expanded spelling
    /// (`Holder OF Holder OF Integer`), never a half-translated mix.
    ///
    /// Set once per build by `monomorph::monomorphize_project` from its private
    /// `type_instantiations` inverse map — the only place the mapping is known,
    /// since the mangling is lossy and cannot be parsed back (see
    /// `Monomorphizer::template_view`). Thread-local because a build's
    /// monomorphize-then-render sequence runs on one thread and the harness runs
    /// tests concurrently across threads; a thread that never monomorphizes
    /// renders with an empty table, which is a no-op (every rewrite candidate
    /// must contain a `$`, and no valid source spelling can produce one, so a
    /// diagnostic untouched by this bug's mechanism is never touched by the
    /// rewrite either).
    static TEMPLATE_SPELLINGS: std::cell::RefCell<HashMap<String, String>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Install this build's mangled-symbol -> template-spelling table, read by
/// every subsequent [`show_diagnostic`] on this thread until the next build
/// overwrites it. Called once by `monomorph::monomorphize_project` after a
/// successful run.
pub(crate) fn set_template_spellings(table: HashMap<String, String>) {
    TEMPLATE_SPELLINGS.with(|cell| *cell.borrow_mut() = table);
}

/// Whether `c` may appear inside a mangled-symbol token
/// (`mangle_name`/`sanitize_type_name` produce only ASCII alphanumerics, `_`
/// and `.`, joined by `$`).
fn is_mangled_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

/// Rewrite every whole mangled-symbol token in `text` — a maximal run of
/// [`is_mangled_token_char`] characters containing at least one `$` separator,
/// matching `[A-Za-z0-9_.]+(\$[A-Za-z0-9_.]+)+` — to its template spelling from
/// [`TEMPLATE_SPELLINGS`], longest-match first so `Holder$Holder$Integer` is
/// never partly rewritten as `Holder$Holder OF …`. An unknown `$` token (no
/// entry in the table — e.g. a mangled name from an imported `.mfp`'s own prior
/// compilation, which has no `type_instantiations` here) is left exactly as
/// is, never guessed at by parsing the mangled string (forbidden: the encoding
/// is lossy).
fn rewrite_mangled_tokens(text: &str) -> Cow<'_, str> {
    if !text.contains('$') {
        return Cow::Borrowed(text);
    }
    let rewritten = TEMPLATE_SPELLINGS.with(|cell| {
        let table = cell.borrow();
        if table.is_empty() {
            return None;
        }
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut changed = false;
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '$' || !is_mangled_token_char(chars[i]) {
                out.push(chars[i]);
                i += 1;
                continue;
            }
            let start = i;
            let mut j = i;
            while j < chars.len() && is_mangled_token_char(chars[j]) {
                j += 1;
            }
            let mut end = j;
            let mut has_group = false;
            while j < chars.len() && chars[j] == '$' {
                let mut k = j + 1;
                while k < chars.len() && is_mangled_token_char(chars[k]) {
                    k += 1;
                }
                if k == j + 1 {
                    // a bare `$` with nothing token-shaped after it: not a
                    // mangled-symbol group, so stop extending the token here.
                    break;
                }
                has_group = true;
                j = k;
                end = j;
            }
            if has_group {
                let token: String = chars[start..end].iter().collect();
                match table.get(&token) {
                    Some(spelling) => {
                        out.push_str(spelling);
                        changed = true;
                    }
                    None => out.push_str(&token),
                }
                i = end;
            } else {
                out.push(chars[start]);
                i = start + 1;
            }
        }
        changed.then_some(out)
    });
    match rewritten {
        Some(out) => Cow::Owned(out),
        None => Cow::Borrowed(text),
    }
}

/// Run `body` with every located diagnostic COLLECTED instead of rendered, and
/// return them alongside its result as `(rule name, detailed message)`.
///
/// The parse-time diagnostics have no other way to be asserted in process.
/// `parse_source` returns `Result<AstFile, ()>` — the `()` means "diagnostics
/// were printed" — and `testutil::check_src` panics outright on a program that
/// does not parse, so the whole `src/ast/**` refusal surface is reachable today
/// only through `test-accept.sh`, whose coverage lands in a child process
/// (plan tests.md, Finding F1). A test can assert `is_err()` without this, but
/// "it did not parse" is not the contract; WHICH rule fired is.
///
/// Thread-local and scoped, so one test collecting cannot silence another
/// running beside it, and `#[cfg(test)]` so a release build has neither the
/// slot nor the branch.
#[cfg(test)]
pub(crate) fn collecting_diagnostics<T>(body: impl FnOnce() -> T) -> (T, Vec<(String, String)>) {
    COLLECTED.with(|slot| *slot.borrow_mut() = Some(Vec::new()));
    let result = body();
    let collected = COLLECTED
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_default();
    (result, collected)
}

/// One source file, read once and indexed by line for every diagnostic that
/// points into it. `show_diagnostic` used to re-read the whole file per
/// diagnostic, which is what made the cost O(errors × filesize) (bug-505); now
/// each render is one `stat` (to notice a rewritten file, which in-process
/// tests do) plus an indexed slice.
struct CachedSource {
    stamp: SourceStamp,
    contents: String,
    /// `(start, end)` byte range of each line, exactly as `str::lines` yields
    /// them (no terminator; a trailing newline starts no line).
    line_ranges: Vec<(usize, usize)>,
}

type SourceStamp = (u64, Option<SystemTime>);

impl CachedSource {
    fn load(path: &Path) -> Option<Self> {
        let stamp = source_stamp(path)?;
        let contents = fs::read_to_string(path).ok()?;
        let base = contents.as_ptr() as usize;
        let line_ranges = contents
            .lines()
            .map(|line| {
                let start = line.as_ptr() as usize - base;
                (start, start + line.len())
            })
            .collect();
        Some(Self {
            stamp,
            contents,
            line_ranges,
        })
    }

    fn line_count(&self) -> usize {
        self.line_ranges.len()
    }

    /// The `index`th (0-based) line, as `str::lines` would yield it.
    fn line(&self, index: usize) -> Option<&str> {
        let (start, end) = *self.line_ranges.get(index)?;
        Some(&self.contents[start..end])
    }
}

fn source_stamp(path: &Path) -> Option<SourceStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()))
}

fn source_cache() -> &'static Mutex<HashMap<PathBuf, Arc<CachedSource>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<CachedSource>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The indexed contents of `path`, read from disk only the first time (or
/// after its length/mtime changes). `None` when the file cannot be read, which
/// renders the diagnostic without source context exactly as before.
fn cached_source(path: &Path) -> Option<Arc<CachedSource>> {
    let stamp = source_stamp(path)?;
    let mut cache = source_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.get(path) {
        if cached.stamp == stamp {
            return Some(Arc::clone(cached));
        }
    }
    let loaded = Arc::new(CachedSource::load(path)?);
    cache.insert(path.to_path_buf(), Arc::clone(&loaded));
    Some(loaded)
}

/// An echoed source line, made safe for the developer's terminal. The line is
/// untrusted input: an ESC/CSI sequence, BEL, `\r`, or a bidi override in it
/// would otherwise recolor, erase, or visually reorder the very diagnostic that
/// reports it (audit-3 FE-04). Every terminal-unsafe code point is escaped as
/// `\u{XXXX}` by `terminal_safe::safe`; a tab — legitimate indentation, and
/// harmless — is kept verbatim so tab-indented code echoes as written.
fn safe_source_line(line: &str) -> Cow<'_, str> {
    if !line
        .chars()
        .any(|ch| ch != '\t' && crate::terminal_safe::is_terminal_unsafe(ch))
    {
        return Cow::Borrowed(line);
    }
    let mut escaped = String::with_capacity(line.len());
    for (index, segment) in line.split('\t').enumerate() {
        if index > 0 {
            escaped.push('\t');
        }
        escaped.push_str(&crate::terminal_safe::safe(segment));
    }
    Cow::Owned(escaped)
}

/// Print, once, how many located diagnostics were withheld past
/// `MAX_RENDERED_DIAGNOSTICS`. Called by the CLI when a command's diagnostic
/// stream is complete (`cli::dispatch`), so the count is exact.
pub fn report_suppressed_diagnostics() {
    let seen = SEEN.load(Ordering::Relaxed);
    let suppressed = seen.saturating_sub(MAX_RENDERED_DIAGNOSTICS);
    if suppressed == 0 {
        return;
    }
    let noun = if suppressed == 1 {
        "diagnostic"
    } else {
        "diagnostics"
    };
    eprintln!(
        "... and {suppressed} more {noun} not shown (only the first \
         {MAX_RENDERED_DIAGNOSTICS} are rendered)"
    );
}

/// A rejection collected but not yet rendered. The source-path passes
/// (`ir::shape` over the concrete HIR, `ir::verify` over the lowered IR) each
/// return these so the caller can merge both streams and render them in a
/// single pass in stream order — the sequence the goldens record (plan-20-Z,
/// plan-107).
pub struct PendingDiagnostic {
    pub rule: String,
    pub detail: String,
    pub path: PathBuf,
    pub line: usize,
}

/// Render `diagnostics` in the order given. The caller concatenates
/// `ir::shape`'s stream (its HIR walk order) with `ir::verify`'s (its IR
/// traversal order), matching the sequence the goldens record — not a line
/// sort, which neither pass produces (plan-20-Z, plan-107).
pub fn render_pending(diagnostics: Vec<PendingDiagnostic>) {
    for d in &diagnostics {
        show_diagnostic(&d.rule, &d.detail, &d.path, d.line, 1, 1);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warn,
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warn => write!(f, "warn"),
            Severity::Info => write!(f, "info"),
        }
    }
}

pub struct Rule {
    pub code: &'static str,
    pub name: &'static str,
    pub severity: Severity,
    pub message: &'static str,
}

mod table;
use table::RULES;

pub fn show_diagnostic(
    rule_name: &str,
    detailed_message: &str,
    filename: &Path,
    line: usize,
    start_pos: usize,
    end_pos: usize,
) {
    // bug-697: rewrite any mangled instantiated-generic token (`Box$Integer`)
    // to its template spelling (`Box OF Integer`) before the message reaches
    // either the collecting-test path or the terminal — a single render
    // boundary that covers every emit site (`ir::shape`, `ir::verify`,
    // `resolve_augmented`, `validate_entry_point`, …) without touching any of
    // them. A no-op when the table is empty or the text carries no `$`.
    let detailed_message = rewrite_mangled_tokens(detailed_message);
    let detailed_message = detailed_message.as_ref();

    // A test inside `collecting_diagnostics` takes the diagnostic instead of the
    // terminal. Before the cap, deliberately: a collecting test wants all of
    // them, and the cap is about not flooding a human's screen.
    #[cfg(test)]
    {
        let taken = COLLECTED.with(|slot| {
            slot.borrow_mut().as_mut().map(|collected| {
                collected.push((rule_name.to_string(), detailed_message.to_string()));
            })
        });
        if taken.is_some() {
            return;
        }
    }

    let rule = rule_for(rule_name);

    // bug-505: past the cap, count instead of render. The `SEEN` total is what
    // `report_suppressed_diagnostics` reports at the end of the stream.
    if SEEN.fetch_add(1, Ordering::Relaxed) >= MAX_RENDERED_DIAGNOSTICS {
        return;
    }

    if let Some(source) = cached_source(filename) {
        let display_line = line.min(source.line_count()).max(1);
        if source.line_count() > 0 {
            let first_context_line = display_line.saturating_sub(2).max(1);
            for context_line in first_context_line..=display_line {
                if let Some(source_line) = source.line(context_line - 1) {
                    eprintln!("{:>4} | {}", context_line, safe_source_line(source_line));
                }
            }

            if start_pos > 0 && display_line == line {
                let underline_width = end_pos.saturating_sub(start_pos).max(1);
                eprintln!(
                    "     | {}{}",
                    " ".repeat(start_pos.saturating_sub(1)),
                    "^".repeat(underline_width)
                );
            }
        }
    }

    eprintln!(
        "{}:{} {}[{} {}]: {}",
        filename.display(),
        line.max(1),
        rule.severity,
        rule.code,
        rule.name,
        rule.message
    );
    eprintln!("               {}", detailed_message);
}

pub fn show_general_diagnostic(rule_name: &str, detailed_message: &str) {
    let rule = rule_for(rule_name);
    eprintln!(
        "{}[{} {}]: {}",
        rule.severity, rule.code, rule.name, rule.message
    );
    eprintln!("               {}", detailed_message);
}

/// Whether a diagnostic rule is `Error` severity (as opposed to `Warn`/`Info`).
/// Lets a collected diagnostic stream fail the build only on real errors while
/// still rendering warnings.
pub fn is_error(rule_name: &str) -> bool {
    matches!(rule_for(rule_name).severity, Severity::Error)
}

/// Resolve a rule name to its `(code, name)` identity as rendered in a
/// diagnostic header. Returns the `0-000-0000 UNKNOWN_RULE` sentinel when the
/// name is not defined in `RULES` (and, in debug builds, asserts). Used by tests
/// to prove an emit site references a defined rule.
#[cfg(test)]
pub(crate) fn code_and_name(rule_name: &str) -> (&'static str, &'static str) {
    let rule = rule_for(rule_name);
    (rule.code, rule.name)
}

fn rule_for(rule_name: &str) -> &'static Rule {
    match RULES.iter().find(|rule| rule.name == rule_name) {
        Some(rule) => rule,
        None => {
            // An emit site referenced a rule name absent from `RULES`: the emit
            // site and the table have drifted (see bug-40). Fail loudly in debug
            // builds so the mismatch is caught by tests rather than silently
            // degraded to the `0-000-0000 UNKNOWN_RULE` sentinel at runtime.
            debug_assert!(
                false,
                "diagnostic rule `{rule_name}` is not defined in RULES (src/rules/table.rs)"
            );
            &Rule {
                code: "0-000-0000",
                name: "UNKNOWN_RULE",
                severity: Severity::Error,
                message: "unknown diagnostic rule",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_package_invalid_is_a_defined_rule() {
        // bug-40: the corrupt-`.mfp` import path emits `IMPORT_PACKAGE_INVALID`;
        // it must resolve to its defined identity, not the `UNKNOWN_RULE` sentinel.
        assert_eq!(
            code_and_name("IMPORT_PACKAGE_INVALID"),
            ("2-201-0001", "IMPORT_PACKAGE_INVALID")
        );
    }

    #[test]
    fn dead_import_missing_package_name_is_gone() {
        // The old dead rule name was renamed onto slot 2-201-0001; nothing should
        // reference it any longer.
        assert!(
            !RULES
                .iter()
                .any(|rule| rule.name == "IMPORT_MISSING_PACKAGE"),
            "IMPORT_MISSING_PACKAGE was renamed to IMPORT_PACKAGE_INVALID (bug-40)"
        );
    }

    #[test]
    fn severity_displays_all_three_levels() {
        assert_eq!(Severity::Error.to_string(), "error");
        assert_eq!(Severity::Warn.to_string(), "warn");
        // The `Info` arm is otherwise only hit by the (rare) info-severity rules.
        assert_eq!(Severity::Info.to_string(), "info");
    }

    #[test]
    fn is_error_reflects_rule_severity() {
        // An Error-severity rule fails the build; a Warn-severity one does not.
        assert!(is_error("IMPORT_PACKAGE_INVALID"));
        assert!(!is_error("PRIVATE_SHADOWS_PUBLIC"));
        assert!(!is_error("PROJECT_JSON_VALID")); // Info
    }

    #[test]
    fn show_diagnostic_renders_source_context_and_underline() {
        // Drives the on-disk source read, the context-line loop, and the caret
        // underline (start_pos > 0 and display_line == line). Output goes to
        // stderr; we only assert it does not panic on a real, multi-line file.
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("src.mfb");
        std::fs::write(&file, "line one\nline two\nline three\n").expect("write source");
        show_diagnostic("IMPORT_PACKAGE_INVALID", "detail here", &file, 2, 3, 6);
        // A line past the end clamps to the last line, still exercising the reader.
        show_diagnostic("IMPORT_PACKAGE_INVALID", "clamped", &file, 99, 0, 0);
    }

    // The guard is a `debug_assert!`, so the panic this asserts exists only when
    // debug assertions are compiled in — under `cargo test --release` the call
    // degrades to the UNKNOWN_RULE sentinel and the test can never pass.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "not defined in RULES")]
    fn unknown_rule_name_trips_the_debug_assert() {
        // An emit site referencing a rule name absent from `RULES` fails loudly in
        // debug builds (the drift guard, bug-40) rather than silently degrading to
        // the UNKNOWN_RULE sentinel.
        let _ = code_and_name("NO_SUCH_RULE_NAME");
    }

    #[test]
    fn rule_names_are_unique() {
        // `rule_for` resolves by name, so a duplicate name would shadow a rule.
        let mut names: Vec<&str> = RULES.iter().map(|rule| rule.name).collect();
        names.sort_unstable();
        assert!(
            names.windows(2).all(|w| w[0] != w[1]),
            "duplicate rule name in RULES"
        );
    }

    /// Every rule in `RULES` must appear in the embedded `mfb spec diagnostics
    /// rule-codes` table.
    ///
    /// `.ai/compiler.md` requires the embedded spec to stay current with every
    /// diagnostic change, but nothing enforced it: the `errorCode::` registry has
    /// a build-time drift guard while the *rule* table had none, so a new rule
    /// could ship documented only in the source. Caught exactly that during
    /// plan-46 (`NATIVE_LIBRARY_VENDOR_COLLISION` was in `RULES` but not the
    /// spec) — by hand, which is the reason this test exists.
    #[test]
    fn every_rule_is_documented_in_the_spec() {
        let spec = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/docs/spec/diagnostics/01_rule-codes.md"
        ));
        let missing: Vec<&str> = RULES
            .iter()
            .filter(|rule| {
                // The table renders one row per rule as `| `<code>` | `<NAME>` | ...`.
                !spec.contains(&format!("`{}`", rule.code)) || !spec.contains(rule.name)
            })
            .map(|rule| rule.code)
            .collect();
        assert!(
            missing.is_empty(),
            "rules missing from src/docs/spec/diagnostics/01_rule-codes.md: {missing:?}"
        );
    }

    #[test]
    fn show_diagnostic_handles_empty_and_missing_source_files() {
        // An empty source file yields no lines, so the context/underline block is
        // skipped and only the header + detail are rendered (the `!lines.is_empty()`
        // false branch).
        let dir = tempfile::tempdir().expect("temp dir");
        let empty = dir.path().join("empty.mfb");
        std::fs::write(&empty, "").expect("write empty");
        show_diagnostic("IMPORT_PACKAGE_INVALID", "empty file", &empty, 1, 1, 2);

        // A file that does not exist: `fs::read_to_string` fails, so the whole
        // context block is skipped — the diagnostic header still renders.
        let missing = dir.path().join("does-not-exist.mfb");
        show_diagnostic("IMPORT_PACKAGE_INVALID", "missing file", &missing, 3, 1, 4);
    }

    #[test]
    fn show_diagnostic_skips_underline_when_position_precedes_the_reported_line() {
        // start_pos > 0 but the clamped display line differs from the reported
        // line (line past EOF): the caret underline is suppressed even though a
        // start position was given (the `display_line == line` guard is false).
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("src.mfb");
        std::fs::write(&file, "only one line\n").expect("write source");
        show_diagnostic(
            "IMPORT_PACKAGE_INVALID",
            "clamped-with-pos",
            &file,
            42,
            3,
            7,
        );
    }

    #[test]
    fn show_general_diagnostic_renders_header_and_detail() {
        // The context-free renderer (used when there is no source location) emits
        // the rule header and the detail line for each severity.
        show_general_diagnostic("IMPORT_PACKAGE_INVALID", "an error detail");
        show_general_diagnostic("PRIVATE_SHADOWS_PUBLIC", "a warning detail");
        show_general_diagnostic("PROJECT_JSON_VALID", "an info detail");
    }

    #[test]
    fn show_diagnostic_rewrites_mangled_generic_names_to_template_spellings() {
        // bug-697: `show_diagnostic` is the render boundary that turns a
        // post-monomorph mangled nominal (`Box$Integer`) back into what the
        // user wrote (`Box OF Integer`). Drives the table directly (rather
        // than through a full build) so the boundary itself is pinned
        // independent of `monomorph::monomorphize_project`'s table-building.
        let mut table = HashMap::new();
        table.insert("Box$Integer".to_string(), "Box OF Integer".to_string());
        table.insert(
            "Holder$Holder$Integer".to_string(),
            "Holder OF Holder OF Integer".to_string(),
        );
        set_template_spellings(table);

        let (_, collected) = collecting_diagnostics(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let file = dir.path().join("src.mfb");
            std::fs::write(&file, "line one\n").expect("write source");
            show_diagnostic(
                "IMPORT_PACKAGE_INVALID",
                "Binding `n` has initializer type Box$Integer, expected Integer.",
                &file,
                1,
                1,
                1,
            );
            // A nested instantiation renders fully, never half-mangled —
            // the whole `Holder$Holder$Integer` token is one match, not
            // `Holder$Holder` + a partial leftover.
            show_diagnostic(
                "IMPORT_PACKAGE_INVALID",
                "Binding `h` has initializer type Holder$Holder$Integer, expected Integer.",
                &file,
                1,
                1,
                1,
            );
            // An unknown `$` token (e.g. a mangled name from an imported
            // `.mfp`, which has no entry in this build's table) is left
            // exactly as is, never guessed at.
            show_diagnostic(
                "IMPORT_PACKAGE_INVALID",
                "Binding `u` has initializer type Unknown$Stranger, expected Integer.",
                &file,
                1,
                1,
                1,
            );
            // Text with no `$` at all is untouched (the common case).
            show_diagnostic(
                "IMPORT_PACKAGE_INVALID",
                "Binding `p` has initializer type Integer, expected String.",
                &file,
                1,
                1,
                1,
            );
        });

        // Reset so a later test on this thread starts from an empty table.
        set_template_spellings(HashMap::new());

        assert_eq!(
            collected[0].1,
            "Binding `n` has initializer type Box OF Integer, expected Integer."
        );
        assert_eq!(
            collected[1].1,
            "Binding `h` has initializer type Holder OF Holder OF Integer, expected Integer."
        );
        assert_eq!(
            collected[2].1,
            "Binding `u` has initializer type Unknown$Stranger, expected Integer."
        );
        assert_eq!(
            collected[3].1,
            "Binding `p` has initializer type Integer, expected String."
        );
    }

    #[test]
    fn code_and_name_resolves_representative_rules() {
        // Every entry in the table resolves to its own identity (not the sentinel).
        for rule in RULES {
            let (code, name) = code_and_name(rule.name);
            assert_eq!(name, rule.name, "name round-trip for {}", rule.name);
            assert_eq!(code, rule.code, "code round-trip for {}", rule.name);
        }
    }

    #[test]
    fn is_error_partitions_the_whole_table_by_severity() {
        // Exercise `is_error` across every defined rule so both the Error and the
        // non-Error (Warn/Info) arms are hit for real table entries.
        for rule in RULES {
            let expected = matches!(rule.severity, Severity::Error);
            assert_eq!(is_error(rule.name), expected, "is_error for {}", rule.name);
        }
    }

    // ---- bug-505: read-once source cache; FE-04: terminal-safe echo ----
    // These drive the helpers directly rather than `show_diagnostic`: the
    // rendering cap is process-wide, and the thousands of parses this test
    // binary runs have long since crossed it.

    #[test]
    fn cached_source_reads_a_file_once_and_indexes_its_lines() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("src.mfb");
        std::fs::write(&file, "line one\r\nline two\n\nline four").expect("write");
        let first = cached_source(&file).expect("readable");
        let again = cached_source(&file).expect("readable");
        assert!(
            Arc::ptr_eq(&first, &again),
            "a second diagnostic must reuse the indexed file, not re-read it"
        );
        // Exactly `str::lines` semantics: `\r\n` stripped, empty line kept, no
        // phantom line after a missing trailing newline.
        assert_eq!(first.line_count(), 4);
        assert_eq!(first.line(0), Some("line one"));
        assert_eq!(first.line(1), Some("line two"));
        assert_eq!(first.line(2), Some(""));
        assert_eq!(first.line(3), Some("line four"));
        assert_eq!(first.line(4), None);
    }

    #[test]
    fn cached_source_notices_a_rewritten_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("src.mfb");
        std::fs::write(&file, "before\n").expect("write");
        let before = cached_source(&file).expect("readable");
        // A different length is a different stamp even within mtime granularity.
        std::fs::write(&file, "after, longer\n").expect("rewrite");
        let after = cached_source(&file).expect("readable");
        assert_eq!(before.line(0), Some("before"));
        assert_eq!(after.line(0), Some("after, longer"));
    }

    #[test]
    fn cached_source_is_none_for_a_missing_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(cached_source(&dir.path().join("nope.mfb")).is_none());
    }

    #[test]
    fn safe_source_line_escapes_controls_and_bidi_but_keeps_tabs() {
        // Plain text (including a tab) passes through unallocated.
        assert!(matches!(
            safe_source_line("\tLET x = 1"),
            Cow::Borrowed("\tLET x = 1")
        ));
        // ESC/CSI, BEL, CR and RLO are escaped; the tab survives verbatim.
        assert_eq!(
            safe_source_line("\tLET s = \"\u{1b}[31mred\u{7}\r\u{202e}\""),
            "\tLET s = \"\\u{001b}[31mred\\u{0007}\\u{000d}\\u{202e}\""
        );
    }

    #[test]
    fn suppressed_count_is_seen_minus_the_cap() {
        // Only the arithmetic is asserted here (the printer writes to stderr);
        // the end-to-end shape is pinned by tests/cli/cli_diagnostic_stream.rs.
        assert_eq!(0usize.saturating_sub(MAX_RENDERED_DIAGNOSTICS), 0);
        assert_eq!(
            (MAX_RENDERED_DIAGNOSTICS + 51).saturating_sub(MAX_RENDERED_DIAGNOSTICS),
            51
        );
        report_suppressed_diagnostics();
    }
}
