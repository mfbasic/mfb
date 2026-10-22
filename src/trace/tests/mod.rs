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
        size_item("ignored", || "ignored".to_string(), 5);
        count_tally("ignored", || "ignored".to_string(), 5);
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

/// The size leaderboard keeps the largest items, in order, and its totals
/// count every item — including the ones that did not make the cut.
#[test]
fn size_items_keep_the_largest() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    disable_for_test();
    enable();
    for index in 0..(TOP_ITEMS + 10) {
        size_item("test-sizes", || format!("f{index}"), index as u64);
    }
    let guard = STATE.lock().expect("trace state");
    let state = guard.as_ref().expect("enabled");
    let bucket = state
        .size_buckets
        .iter()
        .find(|bucket| bucket.name == "test-sizes")
        .expect("bucket");
    assert_eq!(bucket.count as usize, TOP_ITEMS + 10);
    assert_eq!(bucket.top.len(), TOP_ITEMS);
    // Largest first, and the largest overall item is the last one recorded.
    assert_eq!(bucket.top[0].1, format!("f{}", TOP_ITEMS + 9));
    assert!(bucket.top[0].0 >= bucket.top[1].0);
    // The total covers the ten items the leaderboard dropped, too.
    let every: u64 = (0..(TOP_ITEMS + 10) as u64).sum();
    assert_eq!(bucket.total, every);
    drop(guard);
    disable_for_test();
}

/// A size tally sums amounts and occurrences per key.
#[test]
fn count_tallies_sum_by_key() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    disable_for_test();
    enable();
    count_tally("test-expansion", || "binop:&".to_string(), 100);
    count_tally("test-expansion", || "binop:&".to_string(), 60);
    count_tally("test-expansion", || "op:Return".to_string(), 5);
    let guard = STATE.lock().expect("trace state");
    let state = guard.as_ref().expect("enabled");
    let tally = state
        .count_tallies
        .iter()
        .find(|tally| tally.name == "test-expansion")
        .expect("tally");
    assert_eq!(tally.rows.len(), 2);
    assert_eq!(tally.rows["binop:&"], (160, 2));
    assert_eq!(tally.rows["op:Return"], (5, 1));
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

/// A span that outlives its parent records nothing rather than guessing a
/// path, and the parent's own row is still right.
#[test]
fn a_span_outliving_its_parent_records_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    disable_for_test();
    enable();
    let outer = span("test-parent-first");
    let inner = span("test-orphaned-child");
    drop(outer);
    drop(inner);
    let guard = STATE.lock().expect("trace state");
    let root = guard.as_ref().expect("enabled").root.as_ref().expect("root");
    let parent = child(root, "test-parent-first");
    assert_eq!(parent.count, 1);
    assert!(parent.children.iter().all(|c| c.name != "test-orphaned-child"));
    assert!(root.children.iter().all(|c| c.name != "test-orphaned-child"));
    drop(guard);
    disable_for_test();
}

/// A stage name announced twice is interned once and its span accumulates;
/// with tracing off neither stage call records anything.
#[test]
fn a_repeated_stage_name_accumulates_into_one_row() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    disable_for_test();
    stage("test-stage-off");
    end_stage();
    assert!(STATE.lock().expect("trace state").is_none());
    enable();
    {
        let _outer = span("test-stage-parent");
        stage("test-stage-repeat");
        stage("test-stage-repeat");
        end_stage();
    }
    let guard = STATE.lock().expect("trace state");
    let root = guard.as_ref().expect("enabled").root.as_ref().expect("root");
    let parent = child(root, "test-stage-parent");
    assert_eq!(child(parent, "test-stage-repeat").count, 2);
    drop(guard);
    disable_for_test();
}
