# bug-773: regex compile costs ~20 µs per call and `|`-alternation ~1 µs per start position — `regexbench compile` F (42×), `alternation` D (20×)

Last updated: 2026-09-28
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Other (performance)

Status: Fixed (Goal (a) per-compile timing partly met — see STATUS)
Regression Test: `tests/runtime/rt_regex_cost.rs`

Two `regexbench` rows are D/F grade (`./benchmark/rank.py`; baseline logs, `min` column):

| row | mfb -O1 | mfb -O2 | mfb -O3 | C -O0 | C -O2 | Python | R_C (grade) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `regexbench compile` | 0.632 ms | 0.616 | 0.619 | 0.015 | 0.015 | 0.007 | 42× (F) |
| `regexbench alternation` | 1.239 ms | 1.236 | 1.218 | 0.062 | 0.062 | 0.013 | 20× (D) |

(Source: `grep -A5 '^regexbench' benchmark/baseline/*.log`.)

Two regex costs produce these rows, each measured below:

1. **Compiling a pattern is ~16× C's `regcomp`** — ~21 µs for `[0-9]+`, ~86 µs for
   `cat|dog|bird|fish|owl` — and every `regex::*` call compiles its pattern again. Most of
   that time goes to copying and dropping `__regex_Node` parse-tree values. Each copy or
   drop runs the graph walker, which allocates a 1 552-byte work stack.
2. **Every start position the matcher tries costs ~0.35 µs plus ~0.19 µs per alternative.**
   When the pattern does not start with one fixed literal, nothing filters start positions,
   so `cat|dog|bird|fish|owl` runs the whole engine at all 1 349 positions of the subject.
A third cost is a separate compiler regression: at -O1+ a union-element `get` that is only `MATCH`ed is copied, so the matcher copies a leaf on every node visit — **see bug-776** (not claimed here; it multiplies class-heavy scans 1.7–4.5× at -O1).

**The single correct behavior a fix produces:**

- (a) **Per-compile cost:** `[0-9]+` costs ≤ 5 µs and `cat|dog|bird|fish|owl` ≤ 20 µs,
  standalone, at -O1. That is ≤ 4× C's `regcomp` + `regfree` (1.26 / 5.35 µs) and below
  Python's uncached `sre_compile` (8.1 / 22.8 µs). Copying or dropping a parse node
  allocates no fixed-size block when it has no edges to push.
- (b) **Repeated pattern:** a program that calls `regex::*` with the same pattern string
  pays the compile once, not once per call. That brings the `compile` row within 6× of C
  -O0 (≤ 0.09 ms, grade B).
- (c) **Scan cost:** a start position whose scalar cannot begin any match costs about the
  same as the `firstCp` skip, ≤ 40 ns per position. Today the `firstCp` skip measures
  ~27 ns per position (`zzq` no-match: 37 µs over 1 349 positions). The `alternation` row
  lands within 6× of C -O0 (≤ 0.37 ms, grade B).
## STATUS: FIXED (merged from `worktree-B-773`; Goal (a) per-compile timing partly met)

`rt_regex_cost` counts `--debug` arena allocations:

| measure | before | after | bound |
|---|---:|---:|---:|
| per compiled pattern literal | 29.8 | under 10 | 10 |
| one `[0-9]` class over one literal | +813 | under 50 | 50 |
| repeated `findAll("", "[0-9]+")` per call | 914 | 14 | 25 |
| a `z` position for `cat\|dog\|bird\|fish\|owl` | 15.0 | 0.002 | 1 |

R1 at -O1 (`/tmp/b773-r1.sh`, pre-fix vs fixed, same host):

| line | before | after | goal |
|---|---:|---:|---:|
| `alt x30 (bench)` | 963 µs | 200 µs | ≤ 370 µs |
| `no-match alt zzq\|…` x30 | 1 277 µs | 22 µs | ≤ 40 ns/position |
| `no-match class [#]` x30 | 262 µs | 20.5 µs | — |
| repeated-pattern calls (R1's compile-only lines, now cache hits) | 4.2–51 µs | 0.4–2.3 µs | — |

**Benchmark** (`python3 /tmp/b773-bench.py 10`: the benchmark with `main` reduced to the
regex rows, pre-fix vs fixed, same host; medians in ms; checksums identical):

| row | before | after | goal |
|---|---:|---:|---:|
| `regexbench compile` | 0.505 | 0.044 | ≤ 0.09 |
| `regexbench alternation` | 0.982 | 0.206 | ≤ 0.37 |
| `regexbench capture` | 0.288 | 0.166 | — |
| `regexbench replace` | 0.431 | 0.231 | — |
| `parse regex` | 0.147 | 0.093 | — |

Gate on the tree merged with main: full suite 280 ok + 1 failure, artifact-gate 0 diffs,
acceptance 1582 at -O1 and -O3. The one failure was
`rt_map_fifo_removal_flat::removing_the_oldest_key_is_flat_in_the_window_size`, which
read ×4.2 (set, W = 4000) once under full-suite load. Run alone it passes 4 of 4 at ×1.6
(map and set). It is a known load-sensitive timing test (it also flaked during bug-770),
and this bug touches no map code.

**Goal (a) per-compile timing is partly met.** A single UNCACHED compile was measured
with distinct patterns (`/tmp/b773-u.sh`, a 6-digit suffix per call, minus a
one-literal baseline). `[0-9]+` is about 7–8 µs, down from about 17 (goal ≤ 5);
`cat|dog|bird|fish|owl` is about 23 µs, down from about 51 (goal ≤ 20). The
allocation half of Goal (a) holds: ≤ 10 per literal, and no 1,552-byte block per
node copy or drop. Goals (b) and (c) are met.

**Fixes**

- **Fix 1 (agent A; recovered from its worktree after it stopped on an API limit,
  then reviewed).**
  - The graph walkers allocate their work stack on the first push, starting at 8
    entries instead of 64, and still double.
  - `parseConcat` builds a plain literal itself and calls `parseQuantSuffix` only
    when a quantifier follows.
  - `flatten` consumes its work list without a deep copy.
- **Fix 2 (agent A).** The ASCII class bitset compares code points, not a String
  per code point.
- **`toScalars`.** Uses `toString(toScalar(cp))`, with no one-element list per
  scalar.
- **Fix 4 (agent B).**
  - `__regex_run` owns the start-position loop. It builds the capture, frame,
    choice and snapshot tables once per search and resets them at each start.
  - A first-scalar set (`helper_first_set.rs`) is computed in `flatten` and
    stored after `kids` (`firstCp` = −2 − offset).
- **Deviation: budget charging.** The design said a skip costs no budget, but a
  set filter skips positions that used to spend steps. Each skip therefore
  charges both budgets exactly the visits a failed attempt would have made, and
  a skip that would cross a budget runs the real attempt. Agent B's probe confirms
  every budget threshold is identical to the pre-fix compiler.
- **Fix 3.** `__regex_compile` keeps the last 8 successfully compiled programs per
  thread (module-level globals are per worker, `mfb spec threading` §2),
  replaced round-robin. An invalid pattern re-parses and fails on every call.
  - A hit returns a copy of the stored program.
  - The program is not borrowed in place: a borrow window would have to exclude
    any store to the cache, and a regex callback could compile a new pattern
    inside it.
  - `/tmp/b773-thread.sh` gives identical output to the pre-fix compiler for main
    plus three workers sharing `\d` and `\s+`, 12 distinct patterns (evicting
    entries), and an invalid pattern called twice.
  - The `--debug` accounting tests (`rt_scope_drop_leaks`,
    `rt_recursive_value_drop_symmetry`) pass with the cache live at exit.

**Tests corrected with proof**

- `rt_regex_cost`'s repeated-pattern bound is 25, not 10. The 10 was written in
  this bug without a measurement; a call that skips the compile still allocates
  14.
- `last_use::collect_last_use_moves_finds_the_five_decoder_append_sites` now checks
  `parseConcat`'s six appends, each against the value it appends (all six are last
  reads), instead of a count of 2. It is stricter than before.

**Latent, not fixed (out of scope, per the Blast Radius):** the non-ASCII class
path and the `(?i)` fold path still build Strings per scalar.

References:

- bug-776 (the -O1 union-element `get` copy — the matcher's per-visit leaf copy)
- `mfb spec memory arenas` §*Scope-Drop Frees* (graph drop walker), §*Entropy Fill* (freed bytes are PRNG-filled, so freed-byte volume is a cost)
- `mfb spec language memory-semantics` §14.5 / §14.6 (owned reads; iterative drop allowed)
- `mfb spec stdlib regex` (dialect, leftmost/greedy semantics, errors — unchanged by this bug)
- `benchmark/RANKING.md` §2 (grades); `benchmark/mfb/src/regexbench.mfb`, `benchmark/c/regexbench.c`, `benchmark/python/regexbench.py`
- planning/completed/plan-134-I-regex-matcher-by-node-index.md (claims "a node visit … allocates nothing beyond the growth of three tables" — false at -O1 since plan-149)
- Found during the D/F benchmark bug sweep (owner constraint `/tmp/dbugs-constraint.txt`)

### A fair reading of the `compile` row

The three peers do different work inside the timer:

- **C** (`benchmark/c/regexbench.c:test_regex_compile`) compiles once per run inside the
  timer, then runs 25 `regexec`.
- **Python** (`benchmark/python/regexbench.py:test_regex_compile`) compiles *outside* the
  timer (`pat = re.compile(...)` precedes `t0`). Its row is 25 `findall` calls with no
  compile at all. `re`'s own pattern cache would give the same result anyway.
- **mfb** has no compiled-pattern handle, and adding one is forbidden (Non-goals). It runs
  25 `findAll` calls, and each one compiles.

Per-operation comparison, measured standalone:

| operation | mfb -O1 | C -O0 | Python |
| --- | --- | --- | --- |
| compile `[0-9]+` (mfb: empty-subject `findAll`; C: `regcomp` + `regfree`; Py: uncached `sre_compile.compile`) | 20.9 µs | 1.26 µs | 8.1 µs |
| compile `cat\|dog\|bird\|fish\|owl` (mfb: `match("cat", …)`) | 85.7 µs | 5.35 µs | 22.8 µs |
| one short-line match (`row12 val 84`) excluding compile | ≈4.6 µs (25.5 − 20.9) | ≈0.55 µs ((15 − 1.3)/25, derived from row) | 0.43 µs (cached `re.findall`) |

So the row is ~82% compile (20.9 of 25.5 µs per line). Even with compile-once, the match
side alone would still be ~8× C. Fixes (a) and (b) are both needed, and the match-side
overhead of (c) is needed as well.

## Failing Reproduction

All probes were built with the prebuilt `target/release/mfb` (current main), running
`mfb build -O 1 -q <dir>` and then `<dir>/build/dbe.out`. Times are the minimum over 50
runs. `project.json` is the benchmark's (`"kind": "executable"`, `"entry": "main"`).

### R1 — cost per call and per position (`/tmp/dbE-regex/src/main.mfb`)

```
IMPORT io
IMPORT strings
IMPORT regex
IMPORT collections
IMPORT datetime

FUNC rep(frag AS String, n AS Integer) AS String
  MUT frags AS List OF String = []
  FOR i = 0 TO n - 1
    frags = collections::append(frags, frag)
  NEXT
  RETURN strings::join(frags, " ")
END FUNC

FUNC timeFindAll(label AS String, text AS String, pat AS String, iters AS Integer) AS Nothing
  MUT best AS Integer = 999999999999
  MUT cnt AS Integer = 0
  FOR r = 0 TO 49
    LET t0 AS Integer = datetime::monotonicNanos()
    FOR k = 1 TO iters
      cnt = len(regex::findAll(text, pat))
    NEXT
    LET dt AS Integer = datetime::monotonicNanos() - t0
    IF dt < best THEN
      best = dt
    END IF
  NEXT
  io::print(label & " len=" & toString(len(text)) & " matches=" & toString(cnt) & " ns/call=" & toString(best / iters))
END FUNC

FUNC timeMatch(label AS String, text AS String, pat AS String, iters AS Integer) AS Nothing
  MUT best AS Integer = 999999999999
  MUT ok AS Boolean = FALSE
  FOR r = 0 TO 49
    LET t0 AS Integer = datetime::monotonicNanos()
    FOR k = 1 TO iters
      ok = regex::match(text, pat)
    NEXT
    LET dt AS Integer = datetime::monotonicNanos() - t0
    IF dt < best THEN
      best = dt
    END IF
  NEXT
  io::print(label & " ok=" & toString(ok) & " ns/call=" & toString(best / iters))
END FUNC

FUNC main() AS Nothing
  timeMatch("compile-only [0-9]+ on '1'", "1", "[0-9]+", 200)
  timeMatch("compile-only a on 'a'", "a", "a", 200)
  timeMatch("compile-only abcdefgh on 'abcdefgh'", "abcdefgh", "abcdefgh", 200)
  timeMatch("compile-only cat|dog|bird|fish|owl on 'cat'", "cat", "cat|dog|bird|fish|owl", 200)
  timeFindAll("row-line findAll [0-9]+", "row12 val 84", "[0-9]+", 200)
  timeFindAll("row-line findAll [0-9]+ (empty line)", "", "[0-9]+", 200)
  LET frag AS String = "the cat and dog saw a bird near fish and owl"
  timeFindAll("alt x1", frag, "cat|dog|bird|fish|owl", 20)
  timeFindAll("alt x10", rep(frag, 10), "cat|dog|bird|fish|owl", 5)
  timeFindAll("alt x30 (bench)", rep(frag, 30), "cat|dog|bird|fish|owl", 2)
  timeFindAll("alt x120", rep(frag, 120), "cat|dog|bird|fish|owl", 1)
  LET t30 AS String = rep(frag, 30)
  timeFindAll("lit cat x30", t30, "cat", 2)
  timeFindAll("no-match alt zzq|zzr|zzs|zzt|zzu x30", t30, "zzq|zzr|zzs|zzt|zzu", 2)
  timeFindAll("no-match lit zzq x30", t30, "zzq", 2)
  timeFindAll("no-match class [#] x30", t30, "[#]", 2)
END FUNC
```

Observed at -O1:

```
compile-only [0-9]+ on '1' ok=TRUE ns/call=21125
compile-only a on 'a' ok=TRUE ns/call=6810
compile-only abcdefgh on 'abcdefgh' ok=TRUE ns/call=39065
compile-only cat|dog|bird|fish|owl on 'cat' ok=TRUE ns/call=85715
row-line findAll [0-9]+ len=12 matches=2 ns/call=25455
row-line findAll [0-9]+ (empty line) len=0 matches=0 ns/call=20850
alt x1 len=44 matches=5 ns/call=126050
alt x10 len=449 matches=50 ns/call=474400
alt x30 (bench) len=1349 matches=150 ns/call=1225500
alt x120 len=5399 matches=600 ns/call=4711000
lit cat x30 len=1349 matches=30 ns/call=58500
no-match alt zzq|zzr|zzs|zzt|zzu x30 len=1349 matches=0 ns/call=1603000
no-match lit zzq x30 len=1349 matches=0 ns/call=37000
no-match class [#] x30 len=1349 matches=0 ns/call=430500
```

- Observed: the `compile` row shape costs 25.5 µs per line, 20.9 µs of it with no subject
  at all. Alternation over the benchmark text costs 1 225 µs. Cost is linear in subject
  length (x10 → x30 → x120: 474 → 1 226 → 4 711 µs), so there is **no quadratic**. It is
  a large constant per position.
- Expected: Goal (a)–(c).
- Contrast (fast today): a pattern that starts with a fixed literal (`cat`, `zzq`) uses the
  `firstCp` skip and costs 37–59 µs over the same text, 20–40× cheaper than the
  alternation. That shows the scan itself is not the problem; running the engine at every
  position is.

### R2 — C and Python peers standalone (`/tmp/dbE-cpeer.c`, `cc -O0`)

```
C regcomp+regfree /[0-9]+/ ns=1256
C regcomp+regfree /a/ ns=996
C regcomp+regfree /abcdefgh/ ns=2630
C regcomp+regfree /cat|dog|bird|fish|owl/ ns=5347
C alt match-only x30 len=1349 matches=150 ns/call=57200
py uncached sre_compile /[0-9]+/ ns=8063      py cached re.findall on 'row12 val 84' ns=434
py uncached sre_compile /cat|dog|bird|fish|owl/ ns=22786
py alt x30 findall ns=13139
```

Excluding compile, mfb's alternation matching is ≈1 140 µs (1 225 − 86) against C's
57 µs: 20×.

### R3 — allocations per compile (`mfb build -O 1 --debug`; 1 000 × `regex::match(subject, pattern)`; `arena.0.alloc_calls` / `alloc_bytes` ÷ 1 000)

| subject / pattern | allocs/call | bytes/call |
| --- | --- | --- |
| `''` / `a` | 74 | 26 256 |
| `''` / `ab` | 126 | 45 472 |
| `''` / `abcd` | 193 | 81 856 |
| `''` / `abcdefgh` | 316 | 146 976 |
| `''` / 16 literals | 551 | 276 640 |
| `''` / 32 literals | 1 010 | 535 424 |
| `1` / `[0-9]` | 888 | 59 760 |
| `cat` / `cat\|dog\|bird\|fish\|owl` | 705 | 325 520 |

Each additional pattern literal costs **~29 allocations and ~16.2 KB**. The cost is linear
in pattern length, which is ≈10.4 × 1 552 bytes: the graph walker's work stack (Root
Cause 1). One character class adds ~815 allocations (the ASCII bitset, Root Cause 2).

### R4 — class item count (see bug-776)

At -O1 the per-position cost of a one-class pattern grows with its item count (`zzz…` × 1 350: `[o]` 435 µs, `[cdbfo]` 1 007 µs, `[abcdefghijklmno]` 2 822 µs; at -O0 491 / 575 / 629 µs). That is bug-776's per-visit leaf copy, not a regex cost; it is excluded from this doc's goals.

**After bug-776's fix** (`/tmp/dbE-items`, min of 30, same host): -O1 `[o]` 275 µs, `[cdbfo]` 283 µs, `[abcdefghijklmno]` 309 µs; -O0 478 / 498 / 551 µs. The item-count growth at -O1 is gone; the remaining per-position cost is this doc's.

| Environment | config | Result |
| --- | --- | --- |
| macos-aarch64 (this machine) | -O0 / -O1 / -O2 / -O3 | compile/scan costs ✗ |
| other targets | — | not measured. Root causes 1, 2 and 4 are target-neutral NIR/runtime shape, so they are expected to reproduce (guess) |

## Root Cause

Four mechanisms. Each is cited and measured.

### 1. Parse-tree copies each allocate a 1 552-byte walker stack (compile, per pattern node)

The parser is recursive descent over the recursive union `__regex_Node` (Concat/Alt hold
`List OF __regex_Node`, Repeat/Group hold a child). It passes nodes by value everywhere:

- `__regex_parseAtom` builds a node into a `__regex_Parse` record.
- `__regex_parseQuantSuffix` receives `atom.node` and re-wraps it in a new `__regex_Parse`.
- `__regex_parseConcat` appends `q.node` to `parts`, then builds `__regex_Concat[parts]`.
- `__regex_parseAlt` does the same with `opts`.
- `__regex_flatten` copies again: `collections::get(work, last)` deep-copies the popped
  subtree, which is the whole tree for the root. It also appends a copy of each child.

(Files: `src/codegen/builtins/regex/helper_parse_atom.rs:__regex_parseAtom`,
`helper_parse_quant_suffix.rs:__regex_parseQuantSuffix`,
`helper_parse_concat.rs:__regex_parseConcat`, `helper_parse_alt.rs:__regex_parseAlt`,
`helper_flatten.rs:__regex_flatten`.)

Because `__regex_Node` takes part in a type cycle, each of those copies calls
`_mfb_thread_copy__regex_Node_*` → `_mfb_rt_graph_copy`, and each scope-drop calls
`_mfb_rt_graph_drop`. In the -O1 `.ncode` (`/tmp/dbE-f-nomatchalt`, `bl` targets counted
per function):

| function | `_mfb_rt_graph_drop` | `thread_copy__regex_Node` |
| --- | --- | --- |
| `parseConcat` | 24 | 1 |
| `parseAlt` | 12 | 1 |
| `parseAtom` | 8 | 0 |
| `parseEscapeAtom` | 14 | 0 |
| `parseParen` | 15 | 0 |
| `parseQuantSuffix` | 2 | 3 |
| `flatten` | 4 (+1 `graph_drop_edges`) | 6 |
| `compile` | 4 | 0 |

Both walkers allocate their work stack unconditionally on entry: `STACK_HEADER_SIZE +
STACK_INITIAL_CAPACITY * STACK_ENTRY_SIZE` = 16 + 64 × 24 = **1 552 bytes**. They do this
even when the value has no cyclic edge to push, such as a `__regex_Lit` leaf. See
`src/codegen/memory/arena/graph_copy.rs:lower_graph_copy_walker` and
`src/codegen/memory/arena/graph_drop.rs:lower_graph_drop_walker` ("The work stack:
header + 64 entries").

`arena_free` then PRNG-fills the freed payload (`mfb spec memory arenas` §*Entropy
Fill*), so every walk also pays a ~1.5 KB fill.

The cost of one walk was measured on a user-level union, 1 M iterations of construct +
`MATCH` + drop, timed with `/usr/bin/time`:

- recursive `UNION Node {Lit, Cls, Concat}` (`/tmp/dbE-un-node`): **600 ns, 3 allocs, 1 664 B**
- the same union without the recursive variant (`/tmp/dbE-un-flat`): **200 ns, 2 allocs, 112 B**

So one walk costs about 400 ns. At ~10 walks per pattern literal (derived: 16.2 KB ÷
1 552 B, not directly counted), that is ~4 µs per literal. This matches the measured
~4.6 µs per literal (R1: `abcdefgh` 39.1 µs vs `a` 6.8 µs) and the linear byte growth in
R3.

### 2. The ASCII class bitset builds a String per code point (compile, per class)

`src/codegen/builtins/regex/helper_ascii_class_bitset.rs:__regex_asciiClassBitset` loops
over cp 0..127. On every iteration it:

- builds `encoding::utf32Decode([cp])` (a list literal plus a String);
- calls `__regex_classMatchOne(items, ch, cp)`. That function compares Strings for
  `Range`/`Single` items (`helper_class_match_one.rs`: `ch >= rng.lo AND ch <= rng.hi`,
  `ch = sng.ch`);
- appends to `bits`.

Measured cost: [0-9] adds ~815 allocations (R3: 888 vs 74), and `[a]` adds 11.6 µs
(`/tmp/dbE-cc`: 18.45 µs vs `a` 6.81 µs). That is ≈6.4 allocations and ≈90 ns per bitset
entry. The `compile` row's pattern `[0-9]+` pays this on all 25 calls.

### 3. Every `regex::*` call recompiles; nothing caches the program

All eight public bodies begin with `LET prog AS __regex_Program = __regex_compile(pattern)`:
`func_count.rs`, `func_find.rs`, `func_find_all.rs`, `func_find_all_matches.rs`,
`func_find_match.rs`, `func_match.rs`, `func_replace.rs` and `func_split.rs`
(`grep -ln "__regex_compile(" src/codegen/builtins/regex/*.rs`). The `compile` row
therefore pays items 1 and 2 twenty-five times per run, while C pays them once and
Python pays them zero times.

### 4. Every start position runs the full engine setup; no first-set filter (alternation)

`src/codegen/builtins/regex/helper_search_from.rs:__regex_searchFrom` skips start
offsets only when `prog.firstCp >= 0`. `helper_flatten.rs:__regex_flatten` computes
`firstCp` only for a non-folding literal reached through Concat/Group. For Alt, Class,
Any and Repeat it gives up. So `cat|dog|bird|fish|owl` calls `__regex_tryAt` at all 1 349
positions, while only 270 positions hold a scalar in {c, d, b, f, o}
(`python3 -c "…sum(ch in 'cdbfo')"` → 270).

Each `__regex_tryAt` (`helper_try_at.rs`) builds fresh state:

- `__regex_initCaps` allocates a new caps list;
- `__regex_setCap` updates it;
- `__regex_run` (`helper_run.rs`) allocates a fresh `frames` literal and grows fresh
  `choices`/`snaps` tables;
- a failure returns `__regex_fail()`, a new `Result` with an empty list.

For the alternation, the Alt op pushes a choice point: 8 integers plus the caps snapshot.
Each option pushes a 5-integer Concat frame, visits one Lit leaf and fails, and then the
choice is popped, the caps restored and the next option pushed.

Measured per failed position (R3 method with 100 calls, x30 − x1 subjects ÷ 1 305 extra
positions):

- `[#]`: **9.0 allocations**, ≈0.32 µs
- `zzq|zzr|zzs|zzt|zzu`: **20.0 allocations**, ≈1.12 µs (R1: 1 603 µs ÷ 1 349)

`a|c` over `bbbb…` costs 501 µs vs `[a]` 488 µs (`/tmp/dbE-pm`). So each extra
alternative costs ≈0.19 µs per position.

### 5. (Moved) the -O1 per-visit leaf copy

See bug-776: `__regex_simpleMatchAt`'s `collections::get` + `MATCH` of a leaf copies at -O1+ since plan-149-A2. It is a general compiler regression and is fixed there.

## Goal

- (a) Standalone compile at -O1: `regex::match("", "[0-9]+")` ≤ 5 µs;
  `regex::match("cat", "cat|dog|bird|fish|owl")` ≤ 20 µs. Allocations per pattern
  literal ≤ 10, and no 1 552-byte block per node copy or drop.
- (b) Calling any `regex::*` member N times with the same pattern string compiles once
  (a compile counter or allocation count is flat in N after the first call).
  `regexbench compile` ≤ 0.09 ms at -O1.
- (c) Every non-matching start position that a first-scalar set can reject costs ≤ 40 ns;
  `regexbench alternation` ≤ 0.37 ms at -O1.

### Non-goals (must NOT change)

- **Owner constraint (language spec and memory model).** Nothing observable changes:
  - value semantics — every `get`/`LET`/argument/return still behaves as a copy;
  - `collections::*` results, ordering, errors and error codes;
  - no new syntax, keywords, builtins, signatures or members. In particular, **no public
    compiled-pattern type or `regex::compile` function**, even though that would make the
    `compile` row trivially fast;
  - `mfb man` / `mfb spec language` behavior;
  - the memory model in `mfb spec memory`: copy/mutate/alias rules, the arena allocator,
    the ownership/drop contract, the in-place self-update cost contract, and RES handles.

  Internal lowering, fast paths, runtime-helper algorithms and invisible data-layout
  internals are the only things that may change.
- **Regex semantics.** The following stay byte-identical, and the pinned corpus in
  `tests/runtime/rt_regex_bounds.rs` is the proof:
  - the dialect (`mfb spec stdlib regex`);
  - leftmost-first alternation order, greedy/lazy exploration order, capture spans;
  - zero-width-match iteration, `start` rules, the empty-pattern refusals;
  - the step budgets (`__REGEX_STEP_BUDGET`, the call-wide budget, `__REGEX_PENDING_LIMIT`)
    and when they fire;
  - `ErrInvalidFormat` for every invalid pattern on **every** call, including a repeated
    invalid pattern.
- **No invisible-cache leaks into behavior.** A compiled-program cache must be invisible to
  programs:
  - the same results and errors whether or not the pattern was cached;
  - bounded memory;
  - safe under threads and workers;
  - it must not change `--debug` end-of-run accounting that existing tests assert
    (`live_bytes 0` programs). See Open Decisions.
- **The PRNG fill on free stays.** It is spec'd (§*Entropy Fill*). The fix reduces how
  many bytes are freed; it does not skip the fill.
- **The graph walker stays iterative.** It stays arena-backed with its doubling growth
  (§*Scope-Drop Frees*). Shrinking or lazily allocating the *initial* stack is internal;
  replacing it with native recursion is forbidden (it reintroduces the plan-134-A stack
  overflow).
- **Forbidden wrong fixes:**
  - editing `benchmark/mfb/src/regexbench.mfb`, `benchmark/c/regexbench.c` or
    `benchmark/python/regexbench.py` (for example, hoisting the pattern, shrinking inputs,
    or making C/Python recompile);
  - re-baselining the logs to hide the gap;
  - replacing the regex engine with a host/libc regex (forbidden by the package's
    portability contract in `mfb man regex`).

## Blast Radius

Searches: `grep -rn -A1 "= collections::get(" src/codegen/builtins | grep -B1 "MATCH "`,
`grep -ln "__regex_compile(" src/codegen/builtins/regex/*.rs`,
`grep -rln "add_union" src/codegen/builtins`, plus the `.ncode` `bl` census above.

- `get` + `MATCH` leaf copies in `__regex_simpleMatchAt`, `__regex_run` (`anchorLeaf`), `__regex_flatten` (`firstLeaf`) — **fixed by bug-776**, not here.
- The graph walker's fixed 1 552-byte stack (`graph_copy.rs:lower_graph_copy_walker`, `graph_drop.rs:lower_graph_drop_walker`) — **fixed by this bug** (Root Cause 1). It also affects every copy and drop of `json::Json`, user recursive types, and the other `thread_copy_*` shims. Regex benefits most because its parser copies leaves.
- The seven other public regex members (`count`, `find`, `findAllMatches`, `findMatch`, `match`, `replace`, `split`) — **fixed by this bug** (Root Cause 3/4): they share `__regex_compile` / `__regex_searchFrom` / `__regex_tryAt`. The `capture` (0.428 ms, 9.5×) and `replace` (0.715 ms, 9×) rows are C-grade and will move too; they are not claimed here.
- `parse regex` row (`benchmark/mfb/src/main.mfb:test_parse_regex`, 0.277 ms vs C 0.041, 6.8×) — **shares the pattern** (`[0-9]+`, per-call compile); expected to improve and not claimed.
- `regex/helper_class_match.rs:__regex_classMatch` non-ASCII path (`__regex_chr` + String compares per scalar) — **latent, same String-per-scalar shape as Root Cause 2**, and out of scope: the rows are ASCII-only. Note it in the fix PR.
- `regex/helper_char_eq.rs:__regex_charEq` `(?i)` path (two `caseFold` Strings per visit) — **latent, out of scope**: folding semantics require String case folding (U+00DF → "ss"), and no benchmark row uses `(?i)`.

## Fix Design

**Order:** 1 (runtime walker) → 2 → 4 → 3. (The per-visit leaf copy is bug-776.)
Each is independently landable and measurable with R1/R3/R4.

**Fix 1 — no fixed-size stack for a walk with nothing to push**
(`graph_copy.rs:lower_graph_copy_walker`, `graph_drop.rs:lower_graph_drop_walker`).
Allocate the work stack lazily on the first push; a leaf-only value such as a `__regex_Lit`
never pushes. Alternatively, start at a small capacity (e.g. 4) and keep
`_mfb_rt_graph_stack_grow` doubling. Either way the stack remains an arena work stack
with the same growth, so §*Scope-Drop Frees* and §14.5 ("iterative drop") hold unchanged.

Then cut the parser's copies at the source, without changing semantics:

- `__regex_parseQuantSuffix` should not re-wrap an unquantified atom in a second
  `__regex_Parse`.
- `__regex_flatten` should consume `work` by moving the popped element (`removeAt` first,
  then use it, instead of `get` + `removeAt`).
- Parse leaves should be emitted directly into the flat tables (the parser already knows
  the op kind), so a Lit/Class never becomes a `__regex_Node` at all. This is the long-term
  shape: the recursive tree exists only for Concat/Alt/Repeat/Group, or not at all.

**Fix 2 — build the bitset from code points** (`helper_ascii_class_bitset.rs`). Compare
`cp` against each item's code points: `__regex_Range`/`__regex_Single` already carry
single-scalar Strings; add or derive their code points once per class, not per cp. Build
the String only for fold and `Short`/`Prop` items, which need it. The output is identical,
and `rt_regex_bounds` pins it.

**Fix 4 — first-scalar set + reusable engine state**
(`helper_flatten.rs`, `helper_search_from.rs`, `helper_try_at.rs`, `helper_run.rs`).

- Generalise `firstCp` into a first-set:
  - an ASCII bitset plus an "any non-ASCII may start" flag;
  - a Lit adds its cp (a folded Lit adds both cases, else "unknown");
  - a Class adds its bitset;
  - an Alt takes the union of its options;
  - a Concat takes the first part's set, continuing to the next part only while the part
    is nullable (conservative: treat Repeat lo=0, anchors and groups of nullables as
    "unknown");
  - Any gives "unknown".

  `__regex_searchFrom` then skips positions whose scalar is not in the set, exactly as the
  `firstCp` loop does today. This is a pure prefilter. It never changes which position
  matches first, because a skipped position provably cannot begin a match.
- Hoist `caps`/`frames`/`choices`/`snaps` into per-search state that is reset (length and
  `pending` set back to 0), not rebuilt, at each start position. `helper_run.rs` already
  reuses `choices`/`snaps` slots within a search, so this extends the same idea across
  start offsets.

Budget semantics are untouched: skipped positions ran zero node visits before as well
(`firstCp`), and the step accounting per visit is unchanged.

**Fix 3 — invisible compile cache** (`helper_compile.rs`). Add a small module-level
cache mapping pattern String → `__regex_Program` (e.g. the last 8 patterns, LRU or
round-robin). The regex package already keeps module-level state (`__regex_steps`,
`helper_steps.rs`), and canvas keeps module-level lists (`helper_damage.rs`).

- Cache only successful compiles. An invalid pattern re-parses and FAILs on every call, so
  the error contract is unchanged.
- The cached `__regex_Program` must be borrowed, not copied, on a hit. Use a windowed
  borrow of a module-level container (`mfb spec memory collections` §*`get`*, *windowed*):
  no call inside the window may reach a store to the cache. The matcher does not compile,
  so the compile-then-match window must be structured to hold that.

This stays within the memory model: a module-level global is ordinary program state (§8
program-startup), and nothing a program can observe changes. Measure (a) first: if Fixes
1 and 2 bring `[0-9]+` to ≤ 5 µs, the `compile` row is 25 × (≤5 + ≤1) ≈ 0.15 ms. That is
grade C, not B, so Fix 3 is still needed for Goal (b).

**Expected artifact shifts:**

- `.ncode`/NIR goldens that include the regex package bodies, or any `get`+`MATCH`
  function (for example `get-borrow-match-rt`'s `.ir` if it is post-optimization), will
  change.
- Byte-identity/acceptance goldens that embed regex helpers will change.
- `rt_recursive_value_drop_symmetry::the_regex_matcher_graphs_drop_exactly_their_copies`
  counts must still balance, although the absolute counts change.

Every such diff is a target expected to change; inspect one fixture per class before
accepting it.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [x] 1a. New `tests/runtime/rt_regex_cost.rs`, in the `--debug` counter style of
      `rt_list_element_record_borrow.rs::bounded`. The asserts count allocations, never
      time:
  - (i) compile allocations per pattern literal ≤ 10: 8- vs 16-literal pattern, empty
    subject;
  - (ii) `regex::findAll` called K and 2K times with the same pattern: the allocation delta
    per call excludes a compile. This is RED until Fix 3;
  - (iii) a no-match `cat|dog|bird|fish|owl` over `strings::repeat("z", N)` and `2N`:
    allocations per extra position ≤ 1. This is RED until Fix 4;
- [x] 1b. Blast-radius audit: confirm each regex member's verdict above.

Acceptance: 1a(i)–(iii) fail for the documented reasons; the audit list is complete.
Commit: —

### Phase 2 — the fixes (one commit each, in order 1 → 2 → 4 → 3)

- [x] Fix 1: lazy or small initial work stack in both graph walkers, plus the parser copy cuts in `helper_parse_*`/`helper_flatten.rs`.
- [x] Fix 2: code-point bitset in `helper_ascii_class_bitset.rs`.
- [x] Fix 4: first-scalar set in `__regex_flatten`/`__regex_searchFrom`, plus per-search reusable engine state.
- [x] Fix 3: bounded, success-only compile cache in `__regex_compile`, borrowed on hit.

Acceptance: each fix's Phase 1 test passes; `tests/runtime/rt_regex_bounds.rs`,
`rt_regex_span.rs`, `rt_native_regex_parser_depth.rs`, `rt_recursive_value_*`, and
`get-borrow-match-rt` pass unchanged; R1 meets Goal (a)–(c).
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] Run the full suite first. Then regenerate only the goldens whose diff is the intended
      shape (regex helper bodies); inspect one fixture per
      class before accepting.
- [x] Update the spec: `mfb spec memory arenas` §*Scope-Drop Frees* only if the walker's
      stack allocation wording changes, and `mfb spec stdlib regex` cost notes, if any,
      per `.ai/specifications.md`.
- [x] Re-run the benchmark group (`regexbench`, `parse regex`) and re-baseline
      `benchmark/baseline/*.log` as its own commit.

Acceptance: full suite green; `regexbench compile` ≤ 0.09 ms and `alternation` ≤ 0.37 ms
at -O1; golden deltas limited to the intended change.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_regex_cost.rs` (1a i–iii).
- Semantics guard: `tests/runtime/rt_regex_bounds.rs` (pinned corpus: leftmost order,
  captures, budgets, errors), `rt_regex_span.rs`, `tests/rt-behavior/**/regex*` goldens.
  All must pass without edits.
- Runtime proof: R1 re-run at -O1/-O2/-O3, with numbers recorded in this doc; `./benchmark/rank.py` grades for the two rows.
- Threads/workers: a program that calls `regex::findAll` from a worker and from main with
  the same and with different patterns, with results identical to a cache-less build.
  This is required before Fix 3 lands.
- Doc sync: update `mfb spec memory arenas` only if the walker wording changes.
  `cargo build` + `cargo test --bin mfb spec`.
- Full suite: `cargo test` (the project's full gate), then the benchmark re-baseline.

## Open Decisions

- **Compile cache vs. debug accounting.** Cache blocks are live at exit. Existing
  `--debug`-counter tests that run regex programs could see that live memory
  (`rt_recursive_value_drop_symmetry.rs::the_regex_matcher_graphs_drop_exactly_their_copies`
  compares counters; `rt_scope_drop_leaks.rs::a_repeated_regex_find_all_runs_at_constant_rss`
  checks flat RSS). Which of their assertions a cache would trip is not yet checked. The options are to (i) free the cache at program
  shutdown before the report, or (ii) exclude it the way module-level globals are already
  excluded (`rt_list_element_record_borrow.rs` notes a global "lives until the process
  ends"). Recommended: (i). Whether a module-level global in a builtin package is
  per-worker or shared is **unverified (guess: per package instance, per
  `mfb spec memory arenas` "each worker package instance owns a separate arena")**.
  Resolve it before Fix 3.

## Summary

The regex engine is linear, not quadratic. It is slow because of constant factors that
repeat everywhere:

- ~400 ns graph-walker overhead on every parse-node copy or drop;
- ~90 ns per ASCII code point to build each class bitset;
- a full compile on every call;
- ~0.35 µs of fresh state per start position, with no first-set filter for alternations;
- (separately, bug-776: since plan-149, a leaf copy on every node visit at -O1+).

Most of the engineering risk is in Fix 3 (an invisible cache that must stay
invisible across threads, errors and `--debug` accounting). Fixes 1, 2 and 4 are local
and semantics-free.
