//! plan-156-A Phase 1: a thread control block's block arena.
//!
//! Every value that crosses between a parent and its worker is deep-copied into the
//! control block's own arena: the seed at `thread::start`, each message at the enqueue,
//! the worker's result at its exit. The copy runs with the pinned arena register
//! pointed at the block arena for its length and restored afterwards; the block arena
//! is destroyed — every chunk it mapped returned to the OS — when the block is freed.
//!
//! These run under `--debug`, whose report registers every block arena as a
//! `thread_block` arena and counts its maps and unmaps, and prints the thread table.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, build_debug, counter, run_ok};

/// The `thread.` lines of the (last) report block on `stderr`, as `(key, value)`.
fn thread_lines(case: &str, stderr: &str) -> Vec<(String, u64)> {
    let at = stderr
        .rfind("mfb.debug.begin ")
        .unwrap_or_else(|| panic!("{case}: no report block on stderr:\n{stderr}"));
    stderr[at..]
        .lines()
        .take_while(|line| *line != "mfb.debug.end 1")
        .filter(|line| line.starts_with("thread."))
        .map(|line| {
            let (key, value) = line.split_once(' ').expect("key value");
            (key.to_string(), value.parse().expect("u64 value"))
        })
        .collect()
}

fn thread_value(case: &str, lines: &[(String, u64)], key: &str) -> u64 {
    lines
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| panic!("{case}: no `{key}` line in {lines:?}"))
}

/// Indices of the registered `thread_block` arenas.
fn block_arenas(lines: &[String]) -> Vec<usize> {
    lines
        .iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("arena.")?;
            let (index, kind) = rest.split_once(".kind ")?;
            (kind == "thread_block").then(|| index.parse().ok())?
        })
        .collect()
}

/// Every registered block arena gave every chunk it mapped back to the OS.
fn assert_block_arenas_unmapped(case: &str, lines: &[String], stderr: &str) -> usize {
    let blocks = block_arenas(lines);
    assert!(!blocks.is_empty(), "{case}: no thread_block arena registered:\n{stderr}");
    for index in &blocks {
        let maps = counter(case, lines, *index, "maps");
        let mapped = counter(case, lines, *index, "mapped_bytes");
        assert!(maps >= 1, "{case}: block arena {index} never mapped a chunk");
        assert_eq!(
            counter(case, lines, *index, "unmaps"),
            maps,
            "{case}: block arena {index} kept a chunk mapped"
        );
        assert_eq!(
            counter(case, lines, *index, "unmapped_bytes"),
            mapped,
            "{case}: block arena {index} kept bytes mapped"
        );
    }
    blocks.len()
}

/// 10,000 threads, one after another, whose seeds and results are the three block
/// shapes — a `String`, a record holding a list, and a union — each checked by value
/// on the parent side.
const SHAPES: &str = r#"IMPORT collections
IMPORT io
IMPORT thread

TYPE Bag
  label AS String
  items AS List OF String
END TYPE

TYPE Hit
  text AS String
END TYPE

TYPE Miss
  code AS Integer
END TYPE

UNION Outcome
  Hit
  Miss
END UNION

ISOLATED FUNC echo(w AS ThreadWorker OF String TO String, seed AS String) AS String
  RETURN seed & "!"
END FUNC

ISOLATED FUNC grow(w AS ThreadWorker OF String TO Bag, seed AS Bag) AS Bag
  MUT items AS List OF String = seed.items
  items = collections::append(items, seed.label)
  RETURN Bag[seed.label & "+", items]
END FUNC

ISOLATED FUNC pick(w AS ThreadWorker OF String TO Outcome, seed AS Outcome) AS Outcome
  MATCH seed
    CASE Hit(h)
      RETURN Miss[len(h.text)]
    CASE Miss(m)
      RETURN Hit["code" & toString(m.code)]
  END MATCH
END FUNC

FUNC main AS Integer
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 10000
    LET a AS Thread OF String TO String = thread::start(echo, "n" & toString(i))
    LET s AS String = thread::waitFor(a)
    total = total + len(s)
    LET b AS Thread OF String TO Bag = thread::start(grow, Bag["x", ["p", "q"]])
    LET bag AS Bag = thread::waitFor(b)
    total = total + len(bag.items) + len(bag.label)
    LET seedHit AS Outcome = Hit["abc"]
    LET c AS Thread OF String TO Outcome = thread::start(pick, seedHit)
    LET o AS Outcome = thread::waitFor(c)
    MATCH o
      CASE Miss(m)
        total = total + m.code
      CASE Hit(h)
        total = total + 1000000
    END MATCH
    i = i + 1
  END WHILE
  io::print(toString(total))
  RETURN 0
END FUNC
"#;

#[test]
fn seeds_and_results_round_trip_through_the_block_arena() {
    let exe = build_debug("thread_block_arena_shapes", SHAPES);
    let (stdout, stderr) = run_ok("shapes", &exe);
    // Per iteration: len("n<i>!") + (3 items + len("x+")=2) + len("abc")=3.
    let expected: u64 = (0..10_000u64)
        .map(|i| (i.to_string().len() as u64 + 2) + 5 + 3)
        .sum();
    assert_eq!(stdout, format!("{expected}\n"), "the values crossed intact");
    let lines = arena_lines("shapes", &stderr);
    let blocks = assert_block_arenas_unmapped("shapes", &lines, &stderr);
    assert!(blocks >= 100, "only {blocks} block arenas were registered:\n{stderr}");
    let table = thread_lines("shapes", &stderr);
    assert_eq!(
        thread_value("shapes", &table, "thread.table_live"),
        0,
        "every control block was freed:\n{stderr}"
    );
    // Each iteration's three handles live until its scope ends, so three blocks are
    // alive at once and the freed slots are reused by the next iteration.
    assert_eq!(
        thread_value("shapes", &table, "thread.table_high"),
        3,
        "the freed slots were reused:\n{stderr}"
    );
}

/// One worker exchanging 20,000 messages each way: the block arena holds each message
/// only until the reader's next read drops it, so its peak stays a handful of messages
/// no matter how many cross.
const MESSAGES: &str = r#"IMPORT io
IMPORT thread

ISOLATED FUNC relay(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  MUT n AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 20000
    LET m AS String = thread::receive(w)
    n = n + len(m)
    thread::send(w, m & seed)
    i = i + 1
  END WHILE
  RETURN n
END FUNC

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(relay, "?", 4, 4)
  MUT back AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 20000
    thread::send(t, "message-" & toString(i MOD 10))
    LET r AS String = thread::receive(t)
    back = back + len(r)
    i = i + 1
  END WHILE
  io::print(toString(thread::waitFor(t)) & " " & toString(back))
  RETURN 0
END FUNC
"#;

#[test]
fn a_long_message_exchange_keeps_the_block_arena_small() {
    let exe = build_debug("thread_block_arena_messages", MESSAGES);
    let (stdout, stderr) = run_ok("messages", &exe);
    assert_eq!(stdout, "180000 200000\n", "every message crossed intact");
    let lines = arena_lines("messages", &stderr);
    let blocks = block_arenas(&lines);
    assert_eq!(blocks.len(), 1, "one thread, one block arena:\n{stderr}");
    assert_block_arenas_unmapped("messages", &lines, &stderr);
    // 40,000 messages of 18-19 bytes crossed; unreclaimed they would be ~1 MB. Two
    // 4-slot queues plus the seed, the result and one retired message per queue is a
    // few hundred bytes.
    let peak = counter("messages", &lines, blocks[0], "peak_live_bytes");
    assert!(
        peak < 4096,
        "the block arena's peak was {peak} bytes: messages are not dropped:\n{stderr}"
    );
}
