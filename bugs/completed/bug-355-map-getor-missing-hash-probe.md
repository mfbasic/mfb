# bug-355: `collections::getOr` on a map is O(n) where `collections::get` is O(1) — `lower_map_get_or` never got the hash probe

Last updated: 2026-07-22
Effort: small (<1h)
Severity: MEDIUM
Class: Performance (silent asymptotic degradation)

Status: Fixed (2026-07-22). Probe adopted in `lower_map_get_or`; `getOr` measured
flat (1,383–1,687 µs across M=64…4096, within noise of `get`, was 4,850 →
77,758 µs); full acceptance green (1077 tests); man page updated.
Regression Test: tests/rt-behavior/collections/func_map_getor_hash_probe (new) — asserts the `[hash]` lowering is selected for a probe-eligible map `getOr`

`lower_map_get` (`src/target/shared/code/builder_collection_query.rs:337`) opens
with a hash-probe fast path at `:345-386`, gated on
`Self::map_key_probe_eligible(key_type)` (`:80`), which calls `emit_map_probe`
(`:140`) and returns a `ValueResult` tagged `"[hash]"`. Its sibling
`lower_map_get_or` (`:563`) has no such block: `:572` goes straight to
`reset_temporary_registers()` and the linear entry scan at `:602-647`. The two
functions are otherwise near-identical (bug-333 §C4 measures 90 shared lines of
138/112).

The consequence is measured below: on a `Map OF String TO Integer` with 4096
entries, 20,000 `getOr` lookups take **~67 ms** while the same 20,000 `get`
lookups take **~2 ms** — a 33× gap that doubles every time the map doubles, while
`get` stays flat. Nothing warns the user. `get` and `getOr` are documented as the
same read with different miss handling, sit next to each other in `SEE ALSO`, and
a reader has no reason to expect one to be constant-time and the other linear.
This is silent because the program is *correct* either way — `getOr` returns the
right value, just in time proportional to map size.

The single correct behavior a fix produces: `collections::getOr` on a
probe-eligible map performs the same O(1) hash probe as `collections::get`, with
the default substituted on the probe's not-found branch; its measured cost stops
growing with map size.

References:

- `src/docs/man/builtins/collections/get.txt` and
  `.../getOr.txt` — **neither documents any complexity guarantee**, and no `O(1)`,
  `O(n)`, or "linear scan" language appears anywhere under `src/docs/spec/`
  (searched). So this violates no written contract; it is a quality-of-
  implementation defect against a reasonable user expectation, which is why it is
  MEDIUM and not HIGH. Both man pages describe `getOr` purely as "`get` with a
  default", reinforcing that expectation.
- `src/target/shared/code/builder_collection_mutate.rs:2221-2223` — the codebase's
  own statement of intent: *"Locate the key: O(1) hash probe for eligible key
  types, else the linear scan."* Evidence the omission in `getOr` is accidental.
- **bug-333** (`bugs/bug-333-string-collection-builder-duplication.md`), item
  **C4** and Open Decision #1 — the cleanup-side write-up that first flagged this
  asymmetry and deferred the behavior question. **This document resolves that open
  decision** with measurement; bug-333 §C4 owns the `Miss`-parameterization
  collapse and must not add the probe itself.
- **bug-354** (`bugs/bug-354-static-type-name-drift-typename-failure.md`) — the
  sibling correctness finding from the same cleanup review (bug-333 §C1).
- plan-25-D (`emit_map_probe`'s inline FNV-1a first-bucket probe), plan-02 Phase 6
  (the bucket index).

## Failing Reproduction

The reviewer's claim was verified two ways: by reading both lowerings (the probe
block is present at `builder_collection_query.rs:345-386` and absent from
`lower_map_get_or`), and by measurement. The measurement is the reproduction.

```
$ cat src/main.mfb
IMPORT io
IMPORT collections
IMPORT datetime

SUB bench(m AS Integer, lookups AS Integer)
  MUT map AS Map OF String TO Integer = Map OF String TO Integer {}
  MUT i AS Integer = 0
  WHILE i < m
    map = collections::set(map, "k" & toString(i), i)
    i = i + 1
  WEND

  LET t0 AS Integer = datetime::monotonicNanos()
  MUT acc AS Integer = 0
  MUT j AS Integer = 0
  WHILE j < lookups
    acc = acc + collections::get(map, "k" & toString(j MOD m))
    j = j + 1
  WEND
  LET t1 AS Integer = datetime::monotonicNanos()

  MUT acc2 AS Integer = 0
  MUT k AS Integer = 0
  WHILE k < lookups
    acc2 = acc2 + collections::getOr(map, "k" & toString(k MOD m), 0)
    k = k + 1
  WEND
  LET t2 AS Integer = datetime::monotonicNanos()

  io::print("M=" & toString(m) & " get_us=" & toString((t1 - t0) / 1000) & " getOr_us=" & toString((t2 - t1) / 1000) & " acc=" & toString(acc) & "/" & toString(acc2))
END SUB

SUB main
  bench(64, 20000)
  bench(128, 20000)
  bench(256, 20000)
  bench(512, 20000)
  bench(1024, 20000)
  bench(2048, 20000)
  bench(4096, 20000)
END SUB

$ mfb build && ./build/mapbench.out
```

Built with `target/release/mfb`, run on macOS 24.6.0 aarch64, base `b12213d2`.
20,000 lookups per row; every lookup hits (`acc == acc2` confirms both loops read
identical values). Median of three runs, microseconds:

| M (map entries) | `get` µs | `getOr` µs | ratio |
| --- | --- | --- | --- |
| 64 | 1,721 | 2,601 | 1.5× |
| 128 | 1,659 | 3,614 | 2.2× |
| 256 | 1,951 | 6,180 | 3.2× |
| 512 | 1,771 | 10,732 | 6.1× |
| 1,024 | 2,274 | 18,510 | 8.1× |
| 2,048 | 2,027 | 32,819 | 16.2× |
| 4,096 | 2,016 | 67,049 | 33.3× |

- Observed: `get` is flat — 1.7 ms to 2.0 ms across a **64× increase in map
  size**. `getOr` grows linearly — each doubling of M doubles its time
  (4096/2048 = 2.08× measured), reaching 33× slower than `get` at M=4096 and
  continuing to diverge without bound.
- Expected: both flat. `getOr` should track `get` within the cost of one extra
  default-materialization branch.

Confirmed on a second probe-eligible key type — `Map OF Integer TO Integer`,
same harness:

| M | `get` µs | `getOr` µs | ratio |
| --- | --- | --- | --- |
| 256 | 409 | 2,363 | 5.8× |
| 1,024 | 568 | 9,152 | 16.1× |
| 4,096 | 1,089 | 36,872 | 33.9× |

### Contrast case that behaves correctly today

`collections::hasKey` — the other probe-eligible map read — is flat, confirming
the probe itself works and bounding the defect to `getOr`'s lowering. Same
harness, `Map OF String TO Integer`, 20,000 calls:

| M | `hasKey` µs |
| --- | --- |
| 256 | 8,669 |
| 1,024 | 6,449 |
| 4,096 | 8,213 |

(The higher absolute cost is the `IF`/branch wrapper in the harness; flatness
across a 16× size increase is the point.)

| Environment | Config | Result |
| --- | --- | --- |
| macOS 24.6.0 aarch64 | `target/release/mfb`, base `b12213d2` | diverges ✗ |
| all targets | lowering is above the MIR seam, backend-independent | diverges ✗ (by inspection) |

## Root Cause

`src/target/shared/code/builder_collection_query.rs:563` `lower_map_get_or` is a
copy of `lower_map_get` (`:337`) taken from *below* that function's fast-path
block. `lower_map_get` begins:

```rust
// builder_collection_query.rs:345-349
if Self::map_key_probe_eligible(key_type) {
    let not_found = self.label("map_get_not_found");
    let done = self.label("map_get_done");
    let entry_slot =
        self.emit_map_probe(collection_slot, key_slot, key_type, &not_found)?;
```

…resolving the entry in O(1) and returning at `:381-385` with
`text: "get(...) [hash]"`. `lower_map_get_or`'s body starts at `:572` with
`self.reset_temporary_registers();` — which is exactly `lower_map_get:387`, the
first line *after* the fast path — and proceeds directly to the linear scan
(`:602` `label(&loop_label)` … `:604` `branch_ge(&use_default)` … `:647`
`branch(&loop_label)`), walking every entry until the key matches.

So for any key type where `map_key_probe_eligible` (`:80-85`) returns true —
`String`, `Integer`, `Float`, `Fixed`, `Byte`, `Boolean` — `get` is O(1) and
`getOr` is O(n) on the identical map. Non-eligible key types keep the linear scan
in both and are unaffected.

`hasKey` is immune because it has its own probe block
(`builder_collection_queries.rs:299-314`), added at the same time as `get`'s. The
probe helper `emit_map_probe` (`:140-146`) is generic over the caller: it stores
the found entry address into a fresh stack slot and branches to a caller-supplied
`not_found` label. `getOr` already has a `use_default` label (`:585`) that is
exactly the right branch target, so nothing about `getOr` blocks adoption — the
block was simply never written.

## Goal

- `lower_map_get_or` selects the hash probe for every key type where
  `map_key_probe_eligible` returns true, branching to its existing
  `use_default` label on a probe miss.
- Re-running the reproduction shows `getOr` flat across M = 64…4096, within ~2× of
  `get`, replacing the current 33× gap at M=4096.
- `getOr`'s observable results are unchanged for every key type, present and
  absent, including the `String` default-copy path.

### Non-goals (must NOT change)

- **`getOr`'s value semantics.** The `value_type == "String"` branch at `:650-662`
  copies the borrowed default into a fresh owned string, because returning the
  borrow double-frees it and corrupts the arena. The probe path must reach that
  same `use_default` block. Dropping or bypassing the copy is the realistic
  failure mode here and is forbidden.
- **The linear-scan fallback** for non-probe-eligible key types must remain, byte
  identical.
- **`emit_map_probe`** (`:140`) and `map_key_probe_eligible` (`:80`) — adopt them,
  do not modify them. The probe's inline arithmetic mirrors
  `lower_map_probe_helper` exactly so iteration order stays byte-identical
  (documented `:132-139`); changing it would shift every map's observable order.
- **Do NOT land this inside bug-333 §C4's `Miss`-parameterization collapse.**
  bug-333 requires §C4 to be byte-identical and explicitly says "Do **not** add the
  hash probe to `getOr` here." This is the separate behavior change it defers.
- Do not "fix" this by documenting the asymmetry as intentional in the man pages —
  that was bug-333 Open Decision #1's alternative, and the measurement above
  rejects it.

## Blast Radius

Searched, not recalled:
`grep -rn "map_key_probe_eligible\|emit_map_probe" src/`. All five map-key
lookup paths classified.

**Fixed by this bug:**

- `builder_collection_query.rs:563` `lower_map_get_or` — the only probe-eligible
  key lookup that skips the probe.

**Already correct — the contrast cases:**

- `builder_collection_query.rs:345` `lower_map_get` — has the probe.
- `builder_collection_queries.rs:299` `lower_collection_has_key` — has the probe
  (`:303`); measured flat above.
- `builder_collection_mutate.rs:2226` `lower_map_set_in_place` — has the probe
  (`:2227`), with the intent comment at `:2221-2223`.

**Unaffected — the probe does not apply:**

- `builder_collection_queries.rs:78` `lower_collection_contains` — despite the
  name-level similarity, this is a **list** operation: it derives
  `list_element_type(&collection.type_)` (`:95`) and scans `VALUE_OFFSET`, not
  map keys. The bucket index is keyed on `KEY_OFFSET` payloads, so no probe
  exists for it to skip. Not a sibling of this bug.
- `builder_collection_mutate.rs:4119` `lower_map_remove_key` — scans linearly, but
  it rebuilds the whole map into a fresh allocation (`map_remove_scan_*` /
  `map_remove_copy_*` loops), so it is inherently O(n) and a probe would not
  change its class. Latent at most; out of scope.
- Non-probe-eligible key types (anything outside `String`/`Integer`/`Float`/
  `Fixed`/`Byte`/`Boolean`, per `:80-85`) — linear in both `get` and `getOr`
  today and after the fix. Symmetric, so not a defect.
- All backends — the lowering is above the MIR seam; one fix covers
  aarch64/x86_64/riscv64.

## Fix Design

Prepend to `lower_map_get_or` the probe block from `lower_map_get:345-386`,
retargeted: pass the existing `use_default` label (`:585`) as `emit_map_probe`'s
`not_found_label`, load the value offset/length from the probed entry, call
`emit_load_collection_payload`, then branch to `done`. The `use_default` block at
`:649-665` — including the `String` owned-copy at `:650-662` — is reached
unchanged on a miss.

The correctness risk is concentrated in one place: the probe path and the default
path must converge on the **same result register**. `lower_map_get` allocates its
result inside `emit_load_collection_payload` and returns immediately, but `getOr`
must merge two producers into one location, which is why `:662` already does
`move_register(&result, &copied)`. The new probe path must write the same
`result` register the fallback writes, or the `String` case returns a stale
pointer. Test both hit and miss for `String` and for a fixed-width value type.

Expected output shift: the lowering for every probe-eligible map `getOr` changes,
so goldens covering map `getOr` will move. That is the intended delta and must be
regenerated deliberately — this cannot land inside bug-333's byte-identical phase.

**Rejected alternatives:**

- *Rewrite `getOr` as `IF hasKey(m, k) THEN get(m, k) ELSE default`.* Rejected:
  two probes instead of one, and it changes the `String` ownership path.
- *Parameterize `lower_map_get` on a `Miss` enum and delete `lower_map_get_or`,
  in this change.* Rejected: that is bug-333 §C4, whose acceptance gate is
  byte-identical output. Doing both at once makes it impossible to tell the
  intended probe-shift from a broken extraction. Land the probe first; §C4
  collapses the two afterward, when they genuinely differ only in miss handling
  — which is exactly what makes the collapse safe.
- *Document the asymmetry and leave it.* Rejected by the measurement: 33× at
  M=4096, unbounded, on a function whose own man page frames it as `get` with a
  default.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add the benchmark reproduction above as a checked-in perf case; record the
      current divergence. → `bugs/repro/bug-355-map-getor-hash-probe.mfb`.
      Re-measured 2026-07-22 at `169db18b3` (release build): get flat
      1,967–2,655 µs across M=64…4096 while getOr grows 4,850 → 77,758 µs —
      the divergence reproduces. (Doc drift: the original repro used `WEND`,
      which bug-357 removed from the grammar; the checked-in repro uses
      `END WHILE`.)
- [x] Add a `tests/rt-behavior/collections/` fixture asserting `getOr` returns
      correct values for hit and miss, for `String` and `Integer` value types, on
      probe-eligible and non-eligible key types. It passes today (results are
      correct) and guards the fix.
      → `tests/rt-behavior/collections/func_map_getor_hash_probe/`, with a
      `.macos-aarch64.ncode` golden pinning the lowering: the fixture's maps are
      built with nested functional `set` calls (not `name = set(name, …)`, which
      would take the in-place probe path), so `getOr` is its only probe-eligible
      map read and any `map_probe_*` label in the ncode proves the `[hash]`
      probe was selected for `getOr`. Pre-fix ncode has zero `map_probe` labels.
- [x] Blast-radius audit complete above, verdict per site. Re-verified at
      `169db18b3`: `grep -rn "map_key_probe_eligible\|emit_map_probe" src/` hits
      exactly `lower_map_get` (`builder_collection_query.rs:328`),
      `lower_collection_has_key` (`builder_collection_queries.rs:321`),
      `lower_map_set_in_place` (`builder_collection_mutate.rs:2704`), and the
      definitions — `lower_map_get_or` is the only probe-eligible map lookup
      without the probe. (Line numbers in this doc drifted: `lower_map_get_or`
      is now at `builder_collection_query.rs:522`.)

Acceptance: divergence recorded; the behavior fixture is green pre-fix. ✓
(`scripts/test-accept.sh target/debug/mfb target/accept-actual
func_map_getor_hash_probe` passes.)
Commit: `ffd1325c3`

### Phase 2 — the fix

- [x] Add the probe block to `lower_map_get_or`
      (`builder_collection_query.rs:531` post-fix), branching to the existing
      `use_default` label on a miss and writing the shared `result` register.
      Implementation note: rather than threading the probe into the linear
      scan's labels, the probe path is a self-contained block mirroring
      `lower_map_get`'s (probe → load value offset/length →
      `emit_load_map_payload` → done), with its own `use_default` block that is
      a verbatim copy of the fallback's — including the `String` owned-copy —
      converging on the one `result` register `emit_load_map_payload`
      allocated. The linear scan below is byte-untouched for non-eligible keys.
      Tagged `text: "getOr(…) [hash]"`.

Acceptance: the reproduction shows `getOr` flat across M = 64…4096 — measured
1,383–1,687 µs, within noise of `get` (was 4,850 → 77,758 µs); the Integer-key
variant equally flat (298 → 769 µs, tracking `get` exactly); the Phase 1
behavior fixture still green with identical stdout, including the
String-default owned-copy line; the non-eligible Money-key path unchanged. ✓
Commit: `5b55b27d3`

### Phase 3 — regenerate expected outputs + full validation

- [x] Regenerate goldens; confirm the delta is confined to map-`getOr` lowerings.
      `scripts/artifact-gate.sh` post-fix: 1061 tests, 1314 goldens, **exactly
      1 diff** — the Phase 1 fixture's `.macos-aarch64.ncode`, which now
      carries 160 `map_probe_*` label references (zero pre-fix). No `.ast`,
      `.ir`, `.log`, or other `.ncode` golden moved. Synced.
- [x] `scripts/test-accept.sh` green on macOS: **acceptance tests passed
      (1077 test(s) ran)** — 1076 pre-existing plus the Phase 1 fixture.
- [x] Re-run the reproduction on both `String` and `Integer` key types — both
      flat (see Phase 2 acceptance numbers).
- [x] Open Decision resolved: documented the eligible-key-type hash probe in
      `src/docs/man/builtins/collections/getOr.md`, mirroring the paragraph
      `get.md` already carries.

Acceptance: full suite green; golden delta exactly the map-`getOr` lowering.
Commit: `5b55b27d3`

## Validation Plan

- Regression test: the `tests/rt-behavior/collections/` correctness fixture (hit
  and miss × `String` and fixed-width value types × eligible and non-eligible key
  types), plus the recorded benchmark.
- Runtime proof: the reproduction's timing table — `getOr` at M=4096 drops from
  ~67,000 µs to the ~2,000 µs range `get` occupies.
- Doc sync: none required, since no complexity guarantee is documented anywhere.
  **Optional and recommended:** state the O(1)-for-eligible-key-types property in
  `get.txt` and `getOr.txt` once they agree, so the invariant this bug violated
  becomes writing rather than folklore.
- Full suite: `scripts/artifact-gate.sh` (expect the intended map-`getOr` delta,
  and nothing else), then `scripts/test-accept.sh`.

## Open Decisions

- **Document the complexity in the man pages?** Recommended yes, after the fix —
  `get`, `getOr`, `hasKey`, and `set` all share the eligible-key-type probe, and
  writing it down is what stops the next `getOr` from being written without it.
  Alternative: leave undocumented and rely on the regression test.

## Summary

`lower_map_get_or` was copied from `lower_map_get` starting one line below the
hash-probe fast path, so `collections::getOr` on a probe-eligible map linearly
scans a structure that already has a bucket index built. Measured: flat ~2 ms for
`get` versus 2.6 ms → 67 ms for `getOr` as the map grows 64 → 4096 entries, a 33×
gap that doubles with the map and does not stop. `hasKey` and `set` both have the
probe, so the omission is isolated and clearly accidental — the codebase's own
comment at `builder_collection_mutate.rs:2221` states the O(1) intent.

Filed MEDIUM: results are always correct and no documented contract is broken, but
a core collection read degrades without bound and without warning, on the one
function a user would reach for precisely to *avoid* a `hasKey`-then-`get` double
lookup.

The engineering risk is small and localized: the probe path and the default path
must converge on one result register, or the `String` owned-copy contract at
`:650-662` breaks. The larger risk is sequencing — this must land *outside*
bug-333 §C4's byte-identical collapse, and before it.
