# MFBASIC Performance Hot-Paths Plan

Last updated: 2026-06-27

This plan fixes the benchmark hot paths discovered in `benchmark/run.log`. A
correct implementation makes element-by-element collection mutation, sorting,
JSON parsing, and line-oriented stdin reading run in the expected asymptotic
class (O(n) or amortized O(1) per op) instead of today's O(n²)/O(n³) and
syscall-per-byte behavior — without changing any observable language semantics.

The unifying finding: MFBASIC value semantics force a full O(n) "rebuild a fresh
tight collection" on every collection mutation, and **plan-01 plugged exactly
one hole** — the `name = collections::append(name, item)` self-assignment idiom
(`try_inplace_append_assign`). The structurally identical holes for `set`,
`prepend`/`insert`, `sort`'s per-swap sets, and the functional-accumulator
append (`LET new = append(old, x)`) remain open, and every benchmark fire except
two is a direct consequence. The two exceptions (map lookup, stdin read) are
genuinely independent and get their own tracks.

It complements:

- `./mfb spec memory collections` (collection layout: 40-byte header + 40-byte
  LookupEntry, capacity-based data region; the invariants every in-place path
  must preserve; the open question on map hash metadata). Canonical source:
  `src/spec/memory/05_collections.md`.
- `./mfb spec language memory-semantics` (value/copy/move semantics — the
  contract copy-elision must not violate).
- `./mfb spec language escape-analysis` (the syntactic decision procedure;
  relevant to whether a last-use signal exists for Phase 4).
- `./mfb spec io` (`io::readLine`/`input`/`readChar`/`readByte` line/byte
  contract — buffering must stay invisible to it).

## 1. Goal

- `collections::set(list, i, x)` on a uniquely-owned MUT list is amortized O(1)
  (fixed-width element) instead of two arena allocs + two O(n) copies.
- `collections::sort` over comparable primitive element types runs as an
  in-place O(n²)-comparison / O(n)-swap native helper (currently ~O(n³)).
- `collections::set(map, k, v)` building a map is O(1) amortized per insert
  (insert phase), and map lookup is O(1) average (hash map) instead of O(n).
- The JSON parser's array/object accumulation is O(n) instead of O(n²).
- `io::readLine` (and the readChar/readByte/input family) reads stdin in blocks
  (~1 syscall per 4–8 KiB) instead of one `read()` syscall per byte.
- `collections::prepend` into a MUT list is amortized O(1) instead of O(n).

Concrete acceptance targets (median, per the new `med` column in
`benchmark/runner.sh`): list-sort 311ms→<1ms; record-update 216ms→tens of ms;
parse-json 579ms→tens of ms; io-read 287ms→20–40ms; map-set 54ms→low single
digits; list-prepend 20ms→~5ms.

### Non-goals (explicit constraints)

These are guardrails. A phase that violates one is wrong:

- **No language-surface change.** No new syntax, builtins, or operators
  (a hash map is an internal layout change, not a surface change).
- **Value/copy/move semantics unchanged.** Every in-place mutation must be
  observationally identical to today's copy-and-rebuild. In-place mutation fires
  **only** on a uniquely-owned, non-`by_ref` MUT local with no live alias —
  reusing the exact ownership/copy-insertion gating `try_inplace_append_assign`
  already relies on (`builder_control.rs:710,716,744`). `by_ref` and
  shared-snapshot cases stay on the rebuild path.
- **Layout/ABI: the 40-byte header and 40-byte LookupEntry invariants and the
  capacity-based data-region base (`emit_collection_data_pointer`) stay intact**
  for Phases 1–5. Only Phase 6 (hash map) bumps the collection layout version,
  and it must keep stable iteration order or version it explicitly.
- **Thread-transfer rules unchanged.** `copy_collection_tight` (shrink-to-fit on
  every copy/transfer) stays the boundary; headroom and any new metadata
  (front-gap, hash buckets) must never leak across a copy or thread boundary and
  must be deterministic/offset-relative across transfer.
- **`FOR EACH` snapshot contract preserved.** In-place mutation that moves
  payload bytes a live iterator already snapshotted is forbidden — restrict to
  same-length overwrites or exclude inside `FOR EACH`.
- **Golden output unchanged.** Sort order for non-NaN inputs, map iteration
  order (until/unless Phase 6 versions it), and all parse results identical.

## 2. Current State

Everything below is cited and was verified against source for this plan.

### 2.1 The one in-place hole plan-01 plugged

`try_inplace_append_assign` (`src/target/shared/code/builder_control.rs:697`)
is the **sole** in-place mutation path. It bails unless the call's
`native_builtin_target(target) == Some("append")` (`builder_control.rs:710`)
**and** `args[0]` is the same local being assigned (`arg0 != name` bail,
`builder_control.rs:716`). On match it routes to `lower_list_append_in_place`
(`src/target/shared/code/builder_collection_updates.rs:554`): write into the
spare slot + bump count when there's room, else geometric realloc. Amortized
O(1). Soundness rests on value semantics + copy-insertion (no live alias) and
`FOR EACH` snapshotting count. After mutation it clears `local.constant`
(`builder_control.rs:744`). See [[collection-memory-mgmt]].

### 2.2 `set` is remove-then-insert: two allocs + two O(n) copies (verified)

`lower_collection_set` (`builder_collection_updates.rs:193`) lowers a **list**
set as `lower_list_remove_at` **then** `lower_list_insert_collection`
(`builder_collection_updates.rs:227,235` — verified). `remove_at`
(`:772`) arena-allocs a fresh block and copies every surviving entry + the data
region (`:831` alloc, `:865/:883` copies). `insert_collection` (`:354`)
arena-allocs **again** (`:407`) and copies both data regions verbatim
(`:436-529`). So each list `set` = 2 arena allocs + 2 full O(n) copies. There is
no `set` analogue of `try_inplace_append_assign` — `name = collections::set(...)`
falls straight through to the general reassignment path.

A **map** set (`builder_collection_updates.rs:244`) is the same shape:
`lower_map_remove_key` (`:1286`, linear scan + fresh alloc + repack) then
`lower_map_concat` (`:1100`, second fresh alloc + block copy). Two full O(n)
rebuilds per insert ⇒ the 1000-insert build loop is O(n²).

### 2.3 `sort` is a source insertion sort built on `set` (verified)

`collections::sort` is **not** native — it is rewritten in monomorph
(`src/monomorph.rs:166`) to the MFBASIC-source generic `__collections_sort`
(`src/builtins/collections_package.mfb:13-27` — verified: insertion sort whose
inner loop does **two `collections::set` calls per swap**, lines 20–21). With the
remove+insert set lowering (§2.2), each swap is ~4 O(n) reallocs and the whole
sort is ~O(n³) — hence 311ms for 50 elements, ~150s for 200 (source comment in
`benchmark/list-sort/mfb/src/main.mfb`).

A **true in-place** native selection sort already exists for strings only:
`_mfb_rt_sort_string_list` (`src/target/shared/code/mod.rs:13531-13632`) swaps
40-byte LookupEntry blocks **without touching the variable-width data region**
(swapping `(valueOffset,valueLength)` pairs leaves the data bytes correctly
addressed) and derives the data base from **CAPACITY not count** (`mod.rs:13560`).
But its only caller is the internal readdir helper (`mod.rs:9747`) — it is **not
wired to `collections::sort`**. This is the proven model for a generic helper.

### 2.4 Maps are linear-scan everywhere — no hash (verified via spec)

`lower_string_key_map_get` (`builder_collection_updates.rs:1737`) and
`lower_map_get` (`:1633`) both scan `0..count` comparing keys; there is no hash,
sort, or binary search. The spec confirms this is intentional-for-now and an
open question: `src/spec/memory/05_collections.md:95,288,307`. So map-set is O(n²)
on **both** the insert loop (§2.2) and the lookup loop.

### 2.5 `io::readLine` reads one byte per `read()` syscall (verified)

`lower_io_read_line_helper` (`src/target/shared/code/mod.rs:8007`) sets
`x2 = 1` and calls `emit_read_file` in its `read_loop` (`mod.rs:8126-8136` —
verified) and again for each UTF-8 continuation byte (`:8154,:8179,:8216,:8242,
:8278,:8297`). One `read()` per byte; 100k lines ≈ 600k+ syscalls. The line
buffer growth itself is geometric/amortized (`:8318-8395`), so the syscall storm
— not buffer realloc — is the cost. The reason it can't block-read: stdin is
non-seekable, so over-reading past `\n` would strand the next line's bytes (there
is no per-process read buffer). Contrast `fs::readLine` (`mod.rs:12847`) which
`lseek`s and block-reads the remainder (`:12961`) — only possible on a seekable
fd. `io::write` (`:4794`) already issues one `_write` per line — the asymmetry is
read-only. Precedent for runtime-owned mutable state: the per-arena
`ARENA_STATE` block pinned by `x19` (`mod.rs:121,163`), already holding RNG state
at offsets 88/96 (`ARENA_STATE_SIZE=104`). See [[math-rng-pcg64]],
[[macos-app-mode-progress]].

### 2.6 JSON parser hits the value-copy tax via a functional accumulator (verified)

`json::parse` is the MFBASIC-source package `src/builtins/json_package.mfb`. It
parses over a **pre-split grapheme list** with an **index cursor**, so there is
**no** substring/`mid$` O(n²) — `collections::get` on a list is O(1). The O(n²)
is the array accumulator: `__json_parseArrayItems`
(`json_package.mfb:347`) does `LET nextItems = collections::append(items, ...)`
(`:349`) and tail-recurses passing `nextItems` (`:357` — verified). Because the
result binds a **new** name (`nextItems`, not `items = append(items,…)`), it
**never matches** the `arg0 == name` gate (§2.1) and takes the full-copy path
every element. The object path (`:400` `collections::set(fields,…)`, `:408`
recurse) is the same shape on a Map.

### 2.7 The pervasive copy tax and where it's required

Passing a List to a FUNC is **zero-copy** (args lowered with `lower_value`, not
`lower_value_owned`; params are borrows the callee never frees —
`builder_misc.rs:28`, `mod.rs:3446`). `LET c = f(...)` is also copy-free (a
`NirValue::Call` is not in `value_is_aliasing_source`,
`builder_values.rs:74-85`). The single deep copy in list-copy is at `RETURN xs`
(`lower_returned_value` → `copy_collection_tight`, `builder_misc.rs:3301`). For
**list-copy that copy is semantically required** — `strs` stays live across all
1000 calls — so only a constant-factor (single memcpy vs two-pass tight copy) is
available there. The generic win is elsewhere: copy-elision when the source is
**provably dead** (`x = f(x)`, single-use `RETURN x`, the json `LET new =
append(old, x)` dead-source case).

### 2.8 String `&` is O(n²) — the same hole, for strings (verified by scan)

A read-only scan of the areas no benchmark touches (the other `.mfb` source
packages, `strings::` builtins, the un-benchmarked `collections::` ops, the rest
of I/O) surfaced one large cross-cutting miss and several smaller ones:

- **String `&` concatenation has no in-place path (verified).**
  `lower_string_concat` (`src/target/shared/code/builder_misc.rs:357-461`)
  allocates a fresh block of exactly `left+right+9` bytes (`:404`) and copies both
  operands every time — no headroom, no amortization. So `s = s & x` in a loop is
  **O(n²)**, the exact string analogue of the pre-plan-01 list-append problem. The
  string-concat benchmark (1000 iters → 1000-byte result) only hides it because
  the result is small. **Note:** copy-elision/move semantics does *not* fix this —
  the cost is `&` always allocating a new tight buffer, not a redundant copy of a
  live source. The fix is an in-place string **self-append** (`s = s & x` on a
  uniquely-owned dead local with geometric headroom), a direct sibling of plan-01's
  list append.
- **This is the hidden root cause of a class of source-package O(n²)s** — all
  build strings via `out = out & …` in a loop and are fixed transitively by the
  string self-append (no package rewrites): `regex::replace`/`__regex_expand`
  (`regex_package.mfb:1792,1650`), `csv::stringify`/`quoteField`
  (`csv_package.mfb:136,150,184`), `__http_dechunk` (`http_package.mfb:203`),
  `encoding::htmlUnescape`/punycode (`encoding_package.mfb:878,1077,1202`),
  `datetime::format` (`datetime_package.mfb:625`).
- **`io::readChar`/`readByte`/`input` share readLine's per-byte syscall**
  (`mod.rs:7647,7456`) — confirms Phase 5 must cover the whole stdin family, not
  just `readLine`.
- **`io::write`/`print` is unbuffered: one `write()` syscall per call**
  (`lower_io_write_helper`, `mod.rs:4794`). A loop of 100k prints = 100k syscalls
  (the io-write benchmark's 74ms vs C's 11ms). An output buffer is a candidate —
  but semantically delicate (flush ordering, atexit, read/write interleaving).
- **`collections::distinct`** does `contains()` in a loop
  (`collections_package.mfb:259`) = O(n²); needs a hash **set** — rides on the
  Phase 6 hash map, NOT fixed by in-place mutation.
- **`collections::groupBy`/`mapValues`/`merge`** (`collections_package.mfb:163,
  185,269`) are O(n²) via map `set`+`hasKey` in a loop — fixed by Phase 3
  (in-place map set) + Phase 6 (hash).
- **`regex::toScalars`** splits input with `strings::mid(s,i,1)` per character
  (`regex_package.mfb:217`) = O(n²), and it runs on **every** regex call — a small
  package fix (use `strings::graphemes` once, as the json/csv parsers already do).
- Noise floor (note only, not in scope): string search `contains`/`find`/
  `replace`/`split`/`count` are naive O(n·m) (no KMP); `strings::normalizeNfc` has
  an O(k²) combining-mark sort (`builder_strings_package.rs:706`); a possible
  redundant `collections::get` element copy via `materialize_owned_element`
  (`builder_collection_queries.rs:15`) — low confidence, fold into Phase 4's audit.

## 3. Design Overview

Three layers, landable independently:

1. **Extend the in-place mutation framework** (the plan-01 precedent) to `set`
   (list + map), `prepend`, and a native `sort` helper. Phases 1–3. Low risk —
   each is a mechanical mirror of `lower_list_append_in_place` + the
   `try_inplace_*_assign` gate. Fixes list-sort, record-update, list-prepend, and
   the map-set **insert** phase.

2. **Copy-elision / move on provably-dead sources** (Phase 4). The one genuinely
   generic, soundness-critical change. Unblocks parse-json (and any
   functional-accumulator source package) and removes a deep copy from every
   `x = f(x)` / single-use return. Highest risk → gated behind a liveness proof,
   with a low-risk package-level json fallback if liveness proves infeasible.

3. **Two independent UNIQUE tracks** that the framework does not touch:
   buffered stdin (Phase 5) and a real hash map (Phase 6, riskiest — the only
   layout-version bump).

Correctness risk concentrates in two places: (a) the unique-ownership /
`FOR EACH`-snapshot / variable-width-payload gating shared by every in-place
phase, and (b) the dead-source liveness proof in Phase 4. Both are mitigated by
reusing the exact preconditions plan-01 already proved, and by falling back to
the rebuild path whenever the precondition can't be established.

## 4. Detailed Design

### 4.1 Phase 1 — in-place LIST set (`try_inplace_set_assign`)

Add a sibling to `try_inplace_append_assign` (`builder_control.rs:697`), called
from the `NirOp::Assign` arm (`builder_control.rs:216`) before the general path.
Match `name = collections::set(name, i, x)` on a uniquely-owned non-`by_ref` MUT
list local. Two sub-cases by element width:

- **Fixed-width element** (Integer/Float/Fixed/Boolean/Byte, records with no
  inlined String): new payload `valueLength == old`, so overwrite the value bytes
  in place at the entry's `valueOffset`; patch nothing else. True O(1). Skip both
  arena allocs.
- **Variable-width payload** (String, record with a String field — record-update's
  `Rec`): if `newLen <= oldSlotLen`, overwrite in place + patch `valueLength`;
  else fall back to today's remove+insert. (A tail-shift relayout — one O(n) data
  move, zero allocs — is an Open Decision, §Open; gated by the `FOR EACH`
  snapshot question.)

Must derive the data base from CAPACITY not count and clear `local.constant`
after mutation, exactly as append does.

**Same phase, string sibling — in-place string self-append (§2.8).** Add a
`try_inplace_concat_assign` matching `name = name & x` (and the
left-associated `name = name & a & b …`) on a uniquely-owned MUT String local.
Give String values geometric capacity headroom (mirror the list append growth
shape) so the self-append writes `x`'s bytes into the spare tail and bumps the
length — amortized O(1) — instead of `lower_string_concat` allocating a fresh
tight buffer each time (`builder_misc.rs:357`). `copy_collection_tight`'s
String analogue must shrink-to-fit on copy/return/transfer so headroom never
leaks. This single lever turns the whole class of `out = out & …` source-package
builders (§2.8) from O(n²) to O(n) with **no package edits**.

### 4.2 Phase 2 — native generic in-place SORT helper

A **stable** in-place helper (D3) that swaps 40-byte LookupEntry blocks (data base
from CAPACITY, like `_mfb_rt_sort_string_list` at `mod.rs:13531`) but, unlike that
helper, must NOT be a selection sort — selection is unstable and `collections::sort`
is documented stable, observable on `Float` (`±0.0`, `NaN`). Minimal form: in-place
insertion sort over entries (O(n²) compares, O(1) swaps — already kills today's
O(n³)); scale target: stable merge sort over the entry array with a scratch entry
buffer. It switches on the header `valueType` (`05_collections.md:68-79`) for the
comparison, using the *same* operation the `<` operator lowers to: signed 8-byte
compare for Integer/Fixed (**not** the string helper's unsigned byte compare), the
operator's `fcmp` for Float, byte-compare for String/Byte. **The helper sorts a
fresh owned copy and returns it** (D2): lowering = `copy_collection_tight(arg)` →
sort that buffer in place → return; it must never sort the borrowed argument
(`value is not modified`). Dispatch in monomorph per concrete `T` (D2): comparable
scalar `T` → native helper; everything else (records, non-comparable, `sortBy`) →
the existing source `__collections_sort`.

### 4.3 Phase 3 — in-place MAP set (insert phase only)

`lower_map_set_in_place`: linear-scan for the key (lookup stays O(n) until
Phase 6); if found and the new value fits the slot, overwrite in place; if the
key is new and there's capacity/dataCapacity headroom, write key+value into the
spare slot + bump count/dataLength (mirror `lower_list_append_in_place`'s
headroom contract); else one geometric grow. Removes the remove_key+concat double
rebuild and the tight copy on the assignment. Gated by `try_inplace_set_assign`'s
map arm. Lookup phase deferred to Phase 6.

### 4.4 Phase 4 — copy-elision / move on provably-dead sources

When a NIR source value is provably dead after a copy (no later read, no live
aliasing binding, not Global/Capture/`by_ref`, not `FOR EACH`-snapshotted, not
`value_is_runtime_managed`), **move** instead of `copy_flat_block`: transfer
ownership and deactivate the source's `OwnedValue` cleanup. Reuse the
`deactivate_moved_*` machinery already used for thread/resource args
(`builder_misc.rs:824/839`). The elision predicate gates `lower_value_owned` /
`lower_returned_value`. This also relaxes the `arg0 == name` gate in
`try_inplace_append_assign` to fire when the source list is dead — which is the
json array case (`LET nextItems = append(items, …)` with `items` dead). Prereq:
confirm a last-use/liveness signal exists (the escape pass `ResOwner`,
`src/escape.rs`) or build one — §Open. **Fallback (B):** if liveness is too
risky, rewrite the json array/object accumulators to the iterative MUT
`items = append(items, x)` idiom, which hits the existing in-place append. Small,
low-risk, captures nearly all the parse-json win.

### 4.5 Phase 5 — buffered stdin reader (UNIQUE track A) — SUPERSEDED

**Superseded by `planning/plan-15-stdin-broadcast.md`.** The transparent
per-arena buffered reader below is subsumed by the broadcast-stdin design (each
subscribed thread sees its own cursor over one runtime-owned log), which keeps the
single-threaded program byte-identical while making multi-thread stdin an explicit
`thread::openStdIn`/`closeStdIn` subscription. Build plan-15, not this section.
The original sketch is retained for context only:

Reserve three words in the per-arena `ARENA_STATE` block (`mod.rs:121,163`):
`BUF_PTR`, `BUF_FILLED`, `BUF_POS`; bump `ARENA_STATE_SIZE` and zero-init them
where the state block is already zeroed (so `BUF_PTR` starts NULL → lazy-alloc
fires). Add `_mfb_rt_stdin_next_byte`: if `BUF_POS < BUF_FILLED` return
`buf[BUF_POS++]`; else lazily arena-alloc a 4 KiB buffer (D5), issue **one**
`read(0, buf, CAP)`, set `BUF_FILLED`, `BUF_POS = 0`, return the byte (or EOF on
read==0). Reroute the 7 per-byte `read` sites in `lower_io_read_line_helper`
**and** the readChar/readByte/input lowerings to drain this one shared buffer
(critical: a shared buffer means switching consumers never loses bytes).
Per-arena state ⇒ thread-safe without a lock. Gate buffering on `!isatty` (D5) so
interactive line-at-a-time latency is preserved, and make `pollInput` count
buffered bytes. The
`ARENA_STATE_SIZE` change is layout-sensitive — move `ENTRY_ARGC/ARGV` offsets
with it ([[macos-codegen-latent-bugs]], [[shutdown-and-signal-handlers]]).

### 4.6 Phase 6 — real hash map (UNIQUE track B, riskiest)

Bump the collection layout version in `src/spec/memory/05_collections.md`. Keep the
lookup-entry array in **insertion order** so observable iteration order is unchanged
(D7); add a separate capacity-sized bucket-index array for O(1) probe and an FNV-1a
string-key hash (none exists today; cache the hash in the bucket array if the
40-byte entry can't carry it — D7). Route get / set-key-check / contains / removeKey
through the probe. Buckets are derived metadata: `copy_collection_tight` and
thread-transfer **recompute** them from the offset-relative keys rather than copying
verbatim (D7), guaranteeing determinism with no stale offsets. `keys`/`values`/
iteration keep walking the insertion-ordered entries unchanged.

## Layout / ABI Impact

- **Phases 1–4: none.** Header/entry sizes, capacity-based data base, copy/
  transfer behavior, and golden output all unchanged. In-place mutation is
  confined to function-local owned buffers and is observationally identical to
  copy-and-rebuild.
- **Phase 5:** `ARENA_STATE_SIZE` grows by 3 words (internal runtime state, not a
  user-visible collection/value layout). Update `mfb spec` only if the arena-state
  layout is documented there; the change is invisible to programs.
- **Phase 6:** collection layout version bump in `mfb spec memory collections` —
  the one true ABI change. Iteration order is **preserved** (entries stay
  insertion-ordered; buckets are derived metadata — D7), so golden output is
  unchanged; document the bucket/hash metadata. If the 40-byte entry can't carry a
  cached hash, store it in the bucket array (not a `flagsVersion` split).

## Phases

1. **Native in-place LIST set** (`try_inplace_set_assign`, fixed-width first,
   variable-width fallback) **+ in-place String self-append**
   (`try_inplace_concat_assign` for `name = name & x`, §4.1). Acceptance:
   record-update's set loop and a `List OF Integer` set loop show no `arena_alloc`
   in the set path under codegen inspection; record-update median drops to tens of
   ms; a `s = s & x` loop building a long string is O(n) (no per-iter alloc under
   codegen inspection) and the regex/csv/datetime string-builder paths (§2.8) drop
   accordingly; all collections + strings tests + acceptance pass.
2. **Native generic in-place SORT helper** (stable; sorts a copy; monomorph
   dispatch — D2/D3) wired to `collections::sort` for comparable scalars.
   Acceptance: list-sort 50-elem median <1ms; 200/1000-elem sorts feasible; output
   **byte-identical** to the source sort for *all* inputs including `NaN`/`±0.0`
   (stability preserved); the source list is provably unmodified; sort function
   tests pass for Integer/Float/Fixed/String/Byte.
3. **In-place MAP set** (insert phase). Acceptance: map-set insert phase ~halves;
   build loop shows a single scan + slot write, no double alloc.
4. **Copy-elision / move on dead sources** (+ json `arg0!=name` relaxation), or
   the package-level json fallback if liveness is infeasible. Acceptance:
   parse-json median drops ~1–2 orders of magnitude; the trap-cleanup and
   thread-transfer test suites show no double-free/UAF ([[trap-cleanup-double-free]]).
5. **Buffered stdin reader** — **SUPERSEDED by
   `planning/plan-15-stdin-broadcast.md`** (broadcast stdin: per-thread cursor over
   one runtime-owned log, explicit `thread::openStdIn`/`closeStdIn`, single-threaded
   byte-identical). Same io-read 287ms→20–40ms target; build plan-15 instead. Output
   buffering for `io::write`/`print` stays out of scope (D6 → plan-14).
6. **Real hash map** (layout-version bump). Also unblocks `collections::distinct`
   (O(n²) `contains`-in-loop → hash set) and `groupBy`/`mapValues`/`merge` (§2.8).
   Acceptance: map-set get phase O(n²)→O(n), whole benchmark to low single digits
   and scaling; distinct on 5k elements O(n); map iteration order unchanged or
   explicitly versioned; copy/thread-transfer of maps verified.
6b. **Small package fix:** rewrite `regex::toScalars` (`regex_package.mfb:217`) to
   call `strings::graphemes` once instead of `strings::mid(s,i,1)` per char — kills
   an O(n²) that runs on every regex call. Independent, low-risk, landable anytime.
7. *(optional, low value)* list-copy: single memcpy when the source is already
   tight. Constant-factor only.

Rationale: 1–3 are mechanical mirrors of the landed append precedent (lowest
risk, fix four benchmarks). 4 is the generic but soundness-critical elision. 5–6
are independent UNIQUE tracks runnable in parallel with the rest. 7 is cleanup.

## Validation Plan

- **Function tests:** every changed builtin gets `tests/func_collections_set_*`,
  `tests/func_collections_sort_*`, `tests/func_collections_prepend_*`,
  `tests/func_io_readLine_*` (and the readChar/readByte/input family) `_valid/**`
  and `_invalid/**`, **full overload coverage** (each element/key/value type for
  set/sort; list and map for set).
- **Runtime proof** (not just golden output): for each in-place phase, a program
  that mutates a long-enough collection/string that the O(n²)→O(n)/O(1) change is
  observable, plus a codegen-inspection assertion that the in-place path emits no
  `arena_alloc` (mirror `tests/collection-memory-grow-rt`). For the string
  self-append (Phase 1), a `s = s & x` loop building a large string that would be
  visibly quadratic without the fix, plus a regex/csv string-builder proof. For
  Phase 5, a piped 100k-line stdin run proving syscall count collapse. For Phase 4, an aliasing
  stress test (source read after the would-be elision) proving the copy still
  fires when the source is live.
- **Soundness regression:** the existing trap-cleanup and thread-transfer suites
  must stay green for Phases 1, 3, 4 (in-place + move are where double-free/UAF
  hides — [[trap-cleanup-double-free]], [[scope-drop-frees]]).
- **Doc sync:** Phase 6 updates `mfb spec memory collections` (layout version,
  hash/bucket metadata, iteration-order contract) and resolves its open question
  at `05_collections.md:307`. Phases 1–3 update the `collections::set/sort/prepend`
  complexity notes in the same spec if they state a complexity. No diagnostics
  changes expected (no new error codes).
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- **Benchmarks:** re-run `benchmark/benchmark.sh`; compare the new `med` column
  against the targets in §1.

## Decisions (resolved)

Resolution rule applied to every fork below: pick the option that (1) cannot
violate the spec's copy/move/freeze mechanics (`mfb spec language memory-semantics`
§14) and (2) is most correct — i.e. observationally identical to today's
copy-and-rebuild and to the documented builtin contracts. Where two options were
both spec-safe, the one with the smaller observable-behavior surface wins, even at
some performance cost; the fast-but-riskier variant is recorded as a deferred
follow-up, never the default.

- **D1 — Phase 1 variable-width set: overwrite-when-`newLen <= oldSlotLen`, else
  fall back to the existing remove+insert rebuild.** The tail-shift relayout is
  rejected as the default: it moves other elements' payload bytes, which a live
  `FOR EACH` snapshot may re-derive offsets into, risking a read that is no longer
  "an owned value, not an alias into the buffer" (§14.6). Overwrite-in-place
  touches only the target slot, so it is value-identical to a rebuild and trivially
  safe under the snapshot + unique-`MUT`-owner rule (§14.6). The rebuild fallback is
  always correct, so the gate only chooses speed, never correctness. *(Deferred:
  tail-shift behind a proven not-in-`FOR EACH` guard.)* (§4.1)

- **D2 — Phase 2 dispatch in monomorph (`monomorph.rs:166`), per concrete `T`.**
  That is the single point that already specializes `collections::sort` per element
  type and where comparability is decidable: route comparable scalar `T` to the
  native helper symbol, keep the source-`__collections_sort` rewrite for everything
  else. Correctness-neutral to copy/move (pure dispatch). **Binding constraint for
  the helper:** the native sort MUST sort a *fresh owned copy* of the argument and
  return it — `collections::sort … value is not modified` (man) and value semantics
  forbid mutating the borrowed argument. So lowering = `copy_collection_tight(arg)`
  → sort that buffer in place → return it. Sorting the borrowed buffer directly
  would corrupt the caller's list (the benchmark sorts `base` repeatedly) and is
  forbidden. (§4.2)

- **D3 — Phase 2 ordering: stable, using the same `<` the operator lowers to.**
  The man page documents `collections::sort` as **stable** ("items that compare
  equal keep their original relative order"), and the current source sort is
  insertion sort (stable). A *selection* sort (as in `_mfb_rt_sort_string_list`) is
  **unstable** and is therefore rejected — it would change observable output for
  the one type where equal-but-distinguishable elements exist: `Float` (`+0.0`
  vs `-0.0`, and `NaN`, which `<` leaves unordered). The helper must use a stable
  algorithm (minimal: in-place insertion sort over 40-byte entries — O(n²) compares
  but O(1) entry-swaps, already collapsing today's O(n³); scale target: a stable
  merge sort over the entry array with a scratch buffer) and the *identical* `<`
  comparison (same `fcmp`/signed-compare the operator emits) so output is
  byte-identical to the source sort for every input including `NaN`/`±0.0`. For
  Integer/Fixed/Byte/String, equal elements are value-identical so stability is
  unobservable — but using one stable algorithm uniformly is simplest and can't
  regress. (§4.2)

- **D4 — Phase 4 elision driven by the existing flow-sensitive move analysis
  (`typecheck.rs` `OwnershipState`, §14.9), not a fresh pass and not fallback-only.**
  §14.1 explicitly sanctions replacing a copy with a move "when it proves the
  source is not used afterward… must not change diagnostics or observable behavior
  except performance," and §14.2 already moves copyable last-use bindings. The
  `OwnershipState` lattice is exactly that proof. Reusing it makes elision a
  spec-blessed optimization with the soundness obligations already enumerated
  (no `MaybeMoved` source, not a `Global`/`Capture`/`by_ref` borrow, not
  `FOR EACH`-snapshotted, not runtime-managed). The json package fallback (B) stays
  as a risk hedge only — it is not the design. (§4.4)

- **D5 — Phase 5 buffering gated on `!isatty`, 4 KiB, single shared drain.**
  Buffering must not change *what* bytes a program observes or *when*. A blocking
  `read(0, buf, CAP)` on a terminal would change interactive timing (block until
  `CAP`/EOF), so terminals keep byte/line-at-a-time; only non-tty stdin buffers.
  4 KiB already cuts syscalls ~4000×; a larger buffer buys little and costs
  per-thread arena memory. All four consumers (readLine/input/readChar/readByte)
  drain the one shared buffer, and `pollInput` MUST count user-space buffered bytes
  (else it reports "no input" while a byte sits buffered). Audit term::/app-mode/GTK
  for direct fd-0 reads and route them through the same drain. (§4.5)

- **D6 — Phase 5 output buffering for `io::write`/`print`: NOT in scope here.**
  Buffering output can change observable behavior — partial output on crash/abort,
  reordering against `stderr`, and flush timing relative to interactive reads — none
  of which is "performance only." Rejected as a default per the resolution rule.
  Pursued instead as an explicit, opt-in feature in **`planning/plan-14-io-buffering.md`**
  (`io::setBuffered`/`isBuffered` + a load-bearing `io::flush`, default off), so it
  never changes behavior for programs that don't opt in. (§Phase 5)

- **D7 — Phase 6 hash map: keep the lookup-entry array in insertion order, add a
  separate capacity-sized bucket-index array; recompute buckets on copy.** Map
  iteration order is "implementation-defined stable" (`05_collections.md:95`) and
  drop order must not be depended on (§14.7) — but the *most correct* choice is to
  not perturb observable iteration order at all, so the entry array stays
  insertion-ordered and only a derived bucket array is added for O(1) probe. Buckets
  are pure derived metadata: `copy_collection_tight` and thread-transfer
  **recompute** them from the (offset-relative) keys rather than copying them
  verbatim, guaranteeing determinism across copy/transfer (§14.3.1) with no stale
  offsets. Hash = FNV-1a over key bytes (deterministic). This is a runtime-layout
  change (spec-legal, §14.3.1) with a memory-spec version bump; golden output is
  unchanged. *(If the 40-byte entry can't carry a cached hash, store the hash in the
  bucket array, not a `flagsVersion` split.)* (§4.6)

- **D8 — `index-store` desugaring: N/A.** Verified there is no subscript-assignment
  lvalue in the language — `list[i]` is read-only indexing and `collections::set`
  ("return a collection with one value changed", `set.txt`) is the sole, purely
  functional mutation surface. So Phase 1 matches only `name = collections::set(
  name, …)`. If subscript assignment is ever added it must desugar to
  `collections::set` and thereby inherit the optimization automatically. (§4.1)

- **D9 — String headroom: a `MUT`-string grown form carries a capacity word; it
  freezes to the canonical tight `[len][bytes][NUL]` on any copy/return/transfer.**
  This mirrors the proven `MUT`-collection design exactly: §14.3 already says
  "returning a `MUT` collection freezes the mutable buffer into an immutable owned
  collection value," and the same freeze (shrink-to-fit) applies to a `MUT` string
  so headroom never escapes — copies stay independent and byte-identical, transfer
  stays deterministic. Runtime layout is the memory spec's domain (§14.3.1), so
  adding the capacity word is spec-legal; the immutable/observable string form is
  unchanged. A side-channel headroom tracker is rejected — it can desync from the
  buffer across copy/transfer, risking exactly the aliasing/independence violations
  §14.6 forbids. (§4.1)

## Non-Goals

- Optimizing the float-compute benchmarks (mandelbrot/nbody, 2–3× over `c -O0`).
  That gap is codegen quality (no optimizer, no inlining, the no-spill register
  allocator), **not** allocation/GC — out of scope here and overlapping with
  plan-01-simd. [[datetime-package-impl]] (register allocator has no spilling).
- A general optimizer / inliner / register spiller.
- list-copy beyond the optional Phase 7 constant-factor — its copy is
  semantically required.

## Summary

Almost every benchmark fire is one mechanism: value semantics rebuild the whole
collection on every mutation, and plan-01 made only `append` in-place. Extending
that proven in-place framework to `set`, `sort`, `prepend`, and the
dead-source/functional-accumulator append (Phases 1–4) fixes list-sort,
record-update, list-prepend, parse-json, and the map-set insert phase. Two fires
are genuinely independent and get their own tracks: buffered stdin (Phase 5) and
a real hash map (Phase 6). The real engineering risk is the unique-ownership /
`FOR EACH`-snapshot gating shared by every in-place phase and the dead-source
liveness proof in Phase 4 — both contained by reusing plan-01's exact
preconditions and falling back to the rebuild path whenever a precondition can't
be proved. What stays untouched: language surface, value/copy/move semantics, the
collection header/entry layout (until Phase 6's versioned hash-map bump), and
thread-transfer behavior.
