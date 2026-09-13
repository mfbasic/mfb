# tools/recursive-value-bench

The before/after instrument for plan-134 (bug-536 Shape C: recursive values copied and
freed). Every number plan-134's letters gate on comes from this one tool; the baselines are
in `planning/plan-134-A-*.md` §2.1 (or `planning/completed/` once archived).

    bash tools/recursive-value-bench/run.sh <mfb> [program...]

Builds each program with `<mfb>` and prints one row per run:

    name size maxrss_bytes real_s exit stdout

- `size` is the program's argument (`-` = none; `a:b` = two arguments).
- `maxrss_bytes` / `real_s` come from `/usr/bin/time -l` (macOS) or `-v` (Linux). Where
  `/usr/bin/time` is missing the program is built with `--debug` and RSS is read from the
  report's `process.peak_rss_bytes`; a crashed run prints no report, so its RSS is `-`.
- `exit` is the process status; a signal death is `128 + N` (139 = SIGSEGV).
- `stdout` is the program's output with spaces as `_`.
- The first run of a freshly built binary includes the OS's first-launch cost, so a single
  `real_s` is noisy. A speed comparison takes the median of 5 runs.

`node_copies` also prints a `node_copies-ncode` row counting, from its `-ncode` relocation
table, the `_mfb_thread_copy_*` and `_mfb_rt_graph_copy` calls `_mfb_fn_main` makes, its
`_mfb_arena_alloc` / `_mfb_arena_free` calls, and the copy functions emitted.

## Programs (`programs/<name>/`)

| Program | Sizes | Measures | Gates |
| --- | --- | --- | --- |
| `c_union_rss` | 400 000, 800 000 | bind `json::JsonNull[NOTHING]` n times — bug-536 Shape C's own repro | G (flat RSS) |
| `c_record_rss` | 400 000, 800 000 | bind `Node[kids := [], tag := i]` n times | G (flat RSS) |
| `json_repeat` | K = 1, 2, 4 | `json::parse` of one 480 003-byte document K times | D/E (speed budget), H (flat RSS) |
| `regex_repeat` | K = 1, 2, 4 | `regex::findAll(subject, "[a-c]+[0-9]+")` over 100 000 chars, 10 000 hits, K times | D/E (speed budget), H (flat RSS) |
| `node_copies` | — | one `Node` stored by bind, list literal in a record, `append`, `LET c = a`; copy calls from `main` | D, E |
| `tree_alias` | — | bug-601's recursive row: `MUT ys = xs` over `List OF Tree`, 5 in-place appends; correct output `ys=6 xs=1` | D |
| `deep_chain` | 50 000, 70 000, 100 000, 1 000 000 | an n-deep chain deep-copied by `collections::get`; exits 139 at ≥ 70 000 while the copy recurses natively | B |
| `deep_build_only` | 1 000 000 | the same chain, never copied (control for `deep_chain`) | B |
| `regex_chain` | `simple:1`, `simple:10000`, `group:1`, `group:10000`, `group:499999`, `group:500001` | the depth of the recursive chains `regex::findAll` builds (see below) | B (depth the walker must handle) |

### `regex_chain`

- `simple <r>` runs `[a-c]+[0-9]+` over r × `"abcab1234 "`. Build it with `--debug` and compare
  `arena.0.alloc_calls` at r = 1 and r = 10 000: the per-hit count is constant, so no
  `__regex_Cont` chain grows with the subject.
- `group <r>` runs `(a)+` over r × `"a"`. Every greedy group iteration pushes one
  `__regex_Choice` onto the `__regex_Choices` chain, so r = 499 999 finishes (exit 0) and
  r = 500 001 raises at `__REGEX_PENDING_LIMIT` (prints `raised=…`, exit 3).
