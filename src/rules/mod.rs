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
mod tests;
