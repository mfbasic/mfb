//! The `-vv` compile profiler: a hierarchical wall-clock tracer for the build
//! pipeline.
//!
//! `-v` answers *which phase* is slow with five `phase <name> <N>ms` lines.
//! That is enough to see that a 340-second acceptance build spends 338 of those
//! seconds in `codegen+link`, and no help at all in seeing *why*. This module is
//! the next zoom level: any point in the compiler can open a named span, spans
//! nest into a tree, and the CLI renders that tree — total, self, count, and
//! share of the build — once the build finishes.
//!
//! Three kinds of record, because "where did the time go" needs three different
//! shapes of answer:
//!
//! * **Spans** ([`span`]) aggregate by *path*, so 3,000 executions of the same
//!   pass collapse into one row with a count. This is the tree.
//! * **Items** ([`item`]) keep a top-N leaderboard of individually-named units
//!   of work — a per-function codegen time, say. One row per function would
//!   drown the tree; the leaderboard answers "is it one pathological function
//!   or all of them?", which the aggregate cannot.
//! * **Counters** ([`count`]) record sizes, not times — function counts,
//!   instruction counts, instantiation counts. A pass that is slow because it is
//!   quadratic in a stream that is itself 50x too long is a different bug from a
//!   pass that is slow per instruction, and only a size number tells them apart.
//!
//! # Disabled by default, and observably inert
//!
//! Everything here is behind one relaxed [`AtomicBool`]. When `-vv` is not
//! selected, [`span`] returns a `Span` with no start time and its `Drop` does
//! nothing, [`item`]/[`count`] return before touching a lock, and no map is ever
//! allocated. The tracer only ever *reads* the compiler's state and only ever
//! writes to stderr, so — exactly like [`crate::cli::build::Verbosity`] itself —
//! the emitted artifact bytes are identical with and without it. That is a hard
//! requirement, not an aspiration: a profiler that perturbed codegen would make
//! every golden depend on the verbosity flag.
//!
//! # Cost when enabled
//!
//! Two [`Instant::now()`] calls and one mutex-guarded tree walk per span, so
//! spans belong at *function* and *pass* granularity, never per instruction. The
//! densest instrumentation in the tree today is one span per optimizer pass per
//! function (~30 passes x the module's function count); at that granularity the
//! tracer's own overhead stays in the low single-digit percent of a build that
//! is slow enough to be worth profiling.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How many individually-named rows a leaderboard bucket keeps. Enough to show
/// a distribution's shape (one outlier vs. a uniformly slow population) without
/// turning the report into a listing.
const TOP_ITEMS: usize = 20;

/// Tree rows below this are folded into one summary line. A build worth
/// profiling is seconds long, so a millisecond is under a tenth of a percent of
/// it — noise that costs a line each.
const NEGLIGIBLE: Duration = Duration::from_millis(1);

/// Whether `-vv` selected tracing. Relaxed everywhere: this is written once,
/// before any span opens, and read on a path that has no other synchronization
/// to order against.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// One node of the span tree, keyed by name within its parent.
///
/// `children` is a `Vec` rather than a map so siblings render in *first-open*
/// order, which for a pass pipeline is pipeline order — the order a reader
/// needs to follow the stream through the compiler. Lookup is a linear scan,
/// which is correct at the arity a span tree actually has (tens of siblings,
/// not thousands).
struct Node {
    name: &'static str,
    total: Duration,
    count: u64,
    children: Vec<Node>,
}

impl Node {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            total: Duration::ZERO,
            count: 0,
            children: Vec::new(),
        }
    }

    /// The node at `path` below `self`, creating any missing ancestors.
    fn descend(&mut self, path: &[&'static str]) -> &mut Node {
        let mut node = self;
        for name in path {
            let index = match node.children.iter().position(|child| child.name == *name) {
                Some(index) => index,
                None => {
                    node.children.push(Node::new(name));
                    node.children.len() - 1
                }
            };
            node = &mut node.children[index];
        }
        node
    }
}

/// A top-N leaderboard of individually-named work units.
struct Bucket {
    name: &'static str,
    /// `(duration, label)`, longest first, truncated to [`TOP_ITEMS`].
    top: Vec<(Duration, String)>,
    /// Totals over *every* item, not just the retained top — so the report can
    /// say what share of the bucket the leaderboard actually accounts for.
    total: Duration,
    count: u64,
}

/// A per-key aggregate: unlike a [`Bucket`], which ranks *individual*
/// occurrences, this sums every occurrence sharing a key. The right shape when
/// the same name recurs thousands of times and the question is which name costs
/// the most in total — a builtin's inline lowering, say, where no single call is
/// slow but one builtin accounts for half the stage.
struct Tally {
    name: &'static str,
    rows: std::collections::HashMap<String, (Duration, u64)>,
}

#[derive(Default)]
struct State {
    root: Option<Node>,
    buckets: Vec<Bucket>,
    tallies: Vec<Tally>,
    counters: Vec<(&'static str, u64)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

thread_local! {
    /// The currently-open span path on this thread. A span's aggregation key is
    /// this stack at the moment it *closes*, so the tree shape follows the call
    /// stack without anything having to thread a context parameter through the
    /// compiler.
    static STACK: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Turn tracing on. Called once by the build CLI when `-vv` is selected, before
/// the pipeline starts.
pub(crate) fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
    *STATE.lock().expect("trace state") = Some(State::default());
}

/// Whether tracing is on. Public so a caller can skip *building* an expensive
/// label (a `format!`) that only the tracer would read.
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// An open span. Closes — recording its elapsed time against the path it was
/// opened at — when dropped, so a `?` early-return out of an instrumented
/// function still accounts for the time it spent before failing.
#[must_use = "a span records nothing until it is dropped; bind it to a `_name` local"]
pub(crate) struct Span {
    /// `None` when tracing is off: the whole type degrades to an empty struct
    /// whose `Drop` returns immediately.
    start: Option<Instant>,
    /// The stack depth this span occupies, i.e. its own index plus one.
    depth: usize,
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(start) = self.start else {
            return;
        };
        let elapsed = start.elapsed();
        // Truncate to this span's own depth rather than popping blindly, so a
        // span that outlives its parent (an `?` unwinding past a stage span, say)
        // cannot leave the stack permanently mis-nested and mis-file every
        // later span in the build. A span whose depth is already gone — its
        // parent closed first — records nothing rather than guessing a path.
        let Some(path) = STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.len() < self.depth {
                return None;
            }
            let path = stack[..self.depth].to_vec();
            stack.truncate(self.depth - 1);
            Some(path)
        }) else {
            return;
        };
        let mut guard = STATE.lock().expect("trace state");
        let Some(state) = guard.as_mut() else {
            return;
        };
        let root = state.root.get_or_insert_with(|| Node::new("build"));
        let node = root.descend(&path);
        node.total += elapsed;
        node.count += 1;
    }
}

/// Open a span named `name` under whatever span is currently open on this
/// thread. Bind the result to a local; the span closes when that local drops.
///
/// ```ignore
/// let _span = trace::span("monomorphize");
/// ```
pub(crate) fn span(name: &'static str) -> Span {
    if !enabled() {
        return Span {
            start: None,
            depth: 0,
        };
    }
    let depth = STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        stack.push(name);
        stack.len()
    });
    Span {
        start: Some(Instant::now()),
        depth,
    }
}

thread_local! {
    /// The span opened by the most recent [`stage`] call, held open until the
    /// next one (or until [`end_stage`]).
    static CURRENT_STAGE: std::cell::RefCell<Option<Span>> =
        const { std::cell::RefCell::new(None) };
}

/// Every stage name seen so far, leaked to `'static`.
///
/// [`stage`] is the one entry point whose name arrives as a runtime `&str`: the
/// backends hand their sub-stage names to a `&dyn Fn(&str)` progress callback,
/// which is what lets one hook cover all five of them without editing any. The
/// set is the handful of literals those backends pass, so leaking is bounded and
/// only ever happens under `-vv`.
static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn intern(name: &str) -> &'static str {
    let mut names = NAMES.lock().expect("trace names");
    if let Some(found) = names.iter().find(|known| **known == name) {
        return found;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.push(leaked);
    leaked
}

/// Open a *stage* span — one that stays open until the next [`stage`] call
/// replaces it, rather than until a local goes out of scope.
///
/// This is the shape a streaming progress callback has: the backend announces
/// "planning + regalloc" and says nothing more until it announces "emitting
/// native code". Hooking it here means the span tree covers each backend's
/// platform-specific tail — object writing, linking, bundle sealing — which
/// lives in five separate `write_executable` functions that no shared span
/// could otherwise reach.
///
/// Call [`end_stage`] before the enclosing span closes, so the stages nest
/// inside it rather than outliving it.
pub(crate) fn stage(name: &str) {
    if !enabled() {
        return;
    }
    let name = intern(name);
    // Replace, not push: dropping the previous stage's span closes it, and the
    // new one opens at the same depth.
    let span = {
        CURRENT_STAGE.with(|current| current.borrow_mut().take());
        span(name)
    };
    CURRENT_STAGE.with(|current| *current.borrow_mut() = Some(span));
}

/// Close the open [`stage`] span, if any.
pub(crate) fn end_stage() {
    if !enabled() {
        return;
    }
    CURRENT_STAGE.with(|current| current.borrow_mut().take());
}

/// Run `body` inside a [`span`] named `name`. The expression form, for
/// instrumenting a long list of sibling calls — a pass pipeline — without a
/// block and a `let _span` around each one.
pub(crate) fn timed<T>(name: &'static str, body: impl FnOnce() -> T) -> T {
    let _span = span(name);
    body()
}

/// Record one individually-named work unit into the `bucket` leaderboard.
///
/// `label` is a closure so the caller pays for the `format!` only when tracing
/// is on — the call sites are inside per-function loops, where an unconditional
/// allocation would be a real slowdown of the un-traced build.
pub(crate) fn item(bucket: &'static str, label: impl FnOnce() -> String, elapsed: Duration) {
    if !enabled() {
        return;
    }
    let mut guard = STATE.lock().expect("trace state");
    let Some(state) = guard.as_mut() else {
        return;
    };
    let index = match state.buckets.iter().position(|entry| entry.name == bucket) {
        Some(index) => index,
        None => {
            state.buckets.push(Bucket {
                name: bucket,
                top: Vec::new(),
                total: Duration::ZERO,
                count: 0,
            });
            state.buckets.len() - 1
        }
    };
    let entry = &mut state.buckets[index];
    entry.total += elapsed;
    entry.count += 1;
    // Insertion sort into a list capped at TOP_ITEMS: cheaper than sorting a
    // per-function-sized vector at the end, and bounds the memory the tracer
    // holds for a module with tens of thousands of functions.
    if entry.top.len() < TOP_ITEMS || elapsed > entry.top[entry.top.len() - 1].0 {
        let at = entry
            .top
            .iter()
            .position(|(duration, _)| *duration < elapsed)
            .unwrap_or(entry.top.len());
        entry.top.insert(at, (elapsed, label()));
        entry.top.truncate(TOP_ITEMS);
    }
}

/// Run `body`, adding its elapsed time to the `bucket` tally row named `key`.
///
/// The recorded time is **inclusive**: if the body re-enters an instrumented
/// site, the inner time is counted in both rows. That is the right reading for
/// the question this answers ("what does lowering a call to `strings::upper`
/// cost me, all in"), but it means tally totals may exceed the enclosing span.
pub(crate) fn timed_tally<T>(
    bucket: &'static str,
    key: impl FnOnce() -> String,
    body: impl FnOnce() -> T,
) -> T {
    if !enabled() {
        return body();
    }
    let start = Instant::now();
    let value = body();
    let elapsed = start.elapsed();
    let mut guard = STATE.lock().expect("trace state");
    let Some(state) = guard.as_mut() else {
        return value;
    };
    let index = match state.tallies.iter().position(|entry| entry.name == bucket) {
        Some(index) => index,
        None => {
            state.tallies.push(Tally {
                name: bucket,
                rows: std::collections::HashMap::new(),
            });
            state.tallies.len() - 1
        }
    };
    let row = state.tallies[index].rows.entry(key()).or_default();
    row.0 += elapsed;
    row.1 += 1;
    value
}

/// Add `amount` to the `name` counter — a size, not a time.
pub(crate) fn count(name: &'static str, amount: u64) {
    if !enabled() {
        return;
    }
    let mut guard = STATE.lock().expect("trace state");
    let Some(state) = guard.as_mut() else {
        return;
    };
    match state.counters.iter_mut().find(|(key, _)| *key == name) {
        Some((_, value)) => *value += amount,
        None => state.counters.push((name, amount)),
    }
}

/// Time `body`, recording it as both a span under `bucket` and a leaderboard
/// item labelled `label`. The shape every per-function instrumentation site
/// wants: the aggregate row answers "how much of the build is this stage", the
/// leaderboard answers "and is it one function or all of them".
pub(crate) fn timed_item<T>(
    bucket: &'static str,
    label: impl FnOnce() -> String,
    body: impl FnOnce() -> T,
) -> T {
    if !enabled() {
        return body();
    }
    let span = span(bucket);
    let start = Instant::now();
    let value = body();
    let elapsed = start.elapsed();
    drop(span);
    item(bucket, label, elapsed);
    value
}

/// Render the collected report to stderr. Called once by the build CLI after
/// the pipeline finishes (including after a *failed* build, so a compile that
/// dies slowly is still profilable).
pub(crate) fn render() {
    if !enabled() {
        return;
    }
    let mut guard = STATE.lock().expect("trace state");
    let Some(state) = guard.as_mut() else {
        return;
    };
    if let Some(root) = &state.root {
        let total = root.children.iter().map(|child| child.total).sum();
        eprintln!("--- trace: span tree ---");
        eprintln!(
            "{:<44} {:>10} {:>10} {:>8} {:>7}",
            "span", "total", "self", "count", "share"
        );
        for child in &root.children {
            render_node(child, 0, total);
        }
    }
    for bucket in &state.buckets {
        if bucket.count == 0 {
            continue;
        }
        eprintln!(
            "--- trace: slowest {} ({} total, {} over {} items) ---",
            bucket.name,
            bucket.top.len().min(TOP_ITEMS),
            millis(bucket.total),
            bucket.count
        );
        for (elapsed, label) in &bucket.top {
            eprintln!("{:>10}  {label}", millis(*elapsed));
        }
    }
    for tally in &state.tallies {
        let mut rows: Vec<(&String, &(Duration, u64))> = tally.rows.iter().collect();
        rows.sort_by(|left, right| right.1 .0.cmp(&left.1 .0));
        let total: Duration = rows.iter().map(|(_, (elapsed, _))| *elapsed).sum();
        eprintln!(
            "--- trace: costliest {} ({} of {} keys, {} total, inclusive) ---",
            tally.name,
            rows.len().min(TOP_ITEMS),
            rows.len(),
            millis(total)
        );
        for (key, (elapsed, count)) in rows.iter().take(TOP_ITEMS) {
            eprintln!("{:>10} {count:>8}x  {key}", millis(*elapsed));
        }
    }
    if !state.counters.is_empty() {
        eprintln!("--- trace: counters ---");
        let width = state
            .counters
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        for (name, value) in &state.counters {
            eprintln!("{name:<width$} {value:>12}");
        }
    }
}

/// One tree row plus its subtree. `share` is against the whole build so a deep
/// row's number is directly comparable with a top-level one.
fn render_node(node: &Node, depth: usize, build_total: Duration) {
    let child_total: Duration = node.children.iter().map(|child| child.total).sum();
    // Saturating: a parent's own span can close before a child's on a panic
    // unwind, and a negative "self" would be a confusing artifact of the report
    // rather than a fact about the compiler.
    let self_time = node.total.saturating_sub(child_total);
    let share = if build_total.is_zero() {
        0.0
    } else {
        100.0 * node.total.as_secs_f64() / build_total.as_secs_f64()
    };
    let indent = "  ".repeat(depth);
    let name = format!("{indent}{}", node.name);
    eprintln!(
        "{name:<44} {:>10} {:>10} {:>8} {share:>6.1}%",
        millis(node.total),
        millis(self_time),
        node.count,
    );
    // Slowest first: the reason to read a profile is to find the top row, and
    // for a fan-out node (per-function work, a pass pipeline) the interesting
    // ordering is by cost, not by name.
    let mut children: Vec<&Node> = node.children.iter().collect();
    children.sort_by(|left, right| right.total.cmp(&left.total));
    // A 30-row pass pipeline is mostly rows that did nothing, and thirty
    // `0.0ms` lines per parent buries the two that matter. Fold the tail into
    // one summary line — which still *names* what was folded and totals it, so
    // the report never silently drops coverage.
    let shown = children
        .iter()
        .position(|child| child.total < NEGLIGIBLE)
        .unwrap_or(children.len());
    for child in &children[..shown] {
        render_node(child, depth + 1, build_total);
    }
    let folded = &children[shown..];
    if !folded.is_empty() {
        let total: Duration = folded.iter().map(|child| child.total).sum();
        let indent = "  ".repeat(depth + 1);
        let name = format!(
            "{indent}({} rows under {})",
            folded.len(),
            millis(NEGLIGIBLE)
        );
        eprintln!("{name:<44} {:>10}", millis(total));
    }
}

/// Milliseconds with one decimal — sub-millisecond rows are common at pass
/// granularity and would all render as a useless `0ms` as integers.
fn millis(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tracer is a process global, so the tests that need it enabled must
    /// not run concurrently with each other. `cargo test` gives each test its
    /// own thread in one process, so they share `STATE`; this lock serializes
    /// them (and `disable_for_test` restores the default-off state so the rest
    /// of the suite is unaffected).
    ///
    /// It does **not** stop the rest of the suite: an unrelated compiler test
    /// running on another thread while tracing is briefly on records its own
    /// spans and counters into the same `STATE`. So every assertion below looks
    /// its subject up **by name** — under names no production call site uses —
    /// rather than by position or by asserting the whole collection. Asserting
    /// `root.children.len() == 1` is what made these tests pass alone and fail
    /// in a full `cargo test`.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// The child of `node` named `name`, or a panic naming what was there.
    fn child<'a>(node: &'a Node, name: &str) -> &'a Node {
        node.children
            .iter()
            .find(|child| child.name == name)
            .unwrap_or_else(|| {
                let names: Vec<&str> = node.children.iter().map(|child| child.name).collect();
                panic!("no `{name}` under `{}`; children: {names:?}", node.name)
            })
    }

    fn disable_for_test() {
        ENABLED.store(false, Ordering::Relaxed);
        *STATE.lock().expect("trace state") = None;
    }

    /// Every entry point is a no-op when tracing is off — the default for every
    /// build that did not pass `-vv`. This is the property that keeps the
    /// profiler out of the emitted bytes.
    #[test]
    fn disabled_records_nothing() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        disable_for_test();
        {
            let _span = span("ignored");
            count("ignored", 5);
            item("ignored", || "ignored".to_string(), Duration::from_secs(1));
        }
        assert!(STATE.lock().expect("trace state").is_none());
        assert!(!enabled());
    }

    /// Nested spans aggregate by path, repeats accumulate into one row with a
    /// count, and a parent's total covers its children's.
    #[test]
    fn spans_nest_and_aggregate() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        disable_for_test();
        enable();
        {
            let _outer = span("test-outer");
            for _ in 0..3 {
                let _inner = span("test-inner");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let guard = STATE.lock().expect("trace state");
        let state = guard.as_ref().expect("enabled");
        let root = state.root.as_ref().expect("root");
        let outer = child(root, "test-outer");
        assert_eq!(outer.count, 1);
        let inner = child(outer, "test-inner");
        assert_eq!(inner.count, 3);
        // The inner span ran three 1ms sleeps inside the one outer span, so the
        // outer total must cover them.
        assert!(outer.total >= inner.total);
        drop(guard);
        disable_for_test();
    }

    /// The leaderboard keeps the longest items, in order, and its totals count
    /// every item — including the ones that did not make the cut.
    #[test]
    fn items_keep_the_slowest() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        disable_for_test();
        enable();
        for index in 0..(TOP_ITEMS + 10) {
            item(
                "test-functions",
                || format!("f{index}"),
                Duration::from_millis(index as u64),
            );
        }
        let guard = STATE.lock().expect("trace state");
        let state = guard.as_ref().expect("enabled");
        let bucket = state
            .buckets
            .iter()
            .find(|bucket| bucket.name == "test-functions")
            .expect("bucket");
        assert_eq!(bucket.count as usize, TOP_ITEMS + 10);
        assert_eq!(bucket.top.len(), TOP_ITEMS);
        // Longest first, and the slowest overall item is the last one recorded.
        assert_eq!(bucket.top[0].1, format!("f{}", TOP_ITEMS + 9));
        assert!(bucket.top[0].0 >= bucket.top[1].0);
        drop(guard);
        disable_for_test();
    }

    /// A tally sums every occurrence sharing a key, rather than ranking single
    /// occurrences the way a leaderboard does.
    #[test]
    fn tallies_sum_by_key() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        disable_for_test();
        enable();
        for _ in 0..3 {
            timed_tally("test-builtin", || "strings.upper".to_string(), || ());
        }
        timed_tally("test-builtin", || "math.rand".to_string(), || ());
        let guard = STATE.lock().expect("trace state");
        let state = guard.as_ref().expect("enabled");
        let tally = state
            .tallies
            .iter()
            .find(|tally| tally.name == "test-builtin")
            .expect("tally");
        assert_eq!(tally.rows.len(), 2);
        assert_eq!(tally.rows["strings.upper"].1, 3);
        assert_eq!(tally.rows["math.rand"].1, 1);
        drop(guard);
        disable_for_test();
    }

    /// Counters sum by name rather than replacing.
    #[test]
    fn counters_accumulate() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        disable_for_test();
        enable();
        count("test-functions", 3);
        count("test-functions", 4);
        count("test-blocks", 1);
        let guard = STATE.lock().expect("trace state");
        let state = guard.as_ref().expect("enabled");
        let counter = |name: &str| {
            state
                .counters
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| *value)
        };
        assert_eq!(counter("test-functions"), Some(7));
        assert_eq!(counter("test-blocks"), Some(1));
        drop(guard);
        disable_for_test();
    }
}
