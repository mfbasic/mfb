# bug-772: `strings::toScalars` / `fromScalars` allocate one or two blocks per scalar through a three-layer `.mfb` seam

Last updated: 2026-09-28
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (performance: a linear op with a 10–15× constant, from per-element heap allocation)

Status: Fixed (2026-10-03, bug-staging-5)
Regression Test: `tests/runtime/rt_scalar_seam_alloc.rs`; the existing
`tests/rt-behavior/general/scalar-strings-seam-rt` (behavior guard)

`strings::toScalars(s)` and `strings::fromScalars(xs)` are correct, but they are MFBASIC source bodies stacked three deep over
`encoding::utf32Encode`/`utf32Decode`, which are themselves source bodies. Each scalar pays at least one `arena_alloc`/
`arena_free` pair plus the arena's entropy scrub on the freed block. `toScalars` pays about 1 pair per scalar, and `fromScalars` about 2. Each also pays a chain of
geometric list regrows and an extra list copy. Both cost 25–40 ns per scalar, flat in n. That is the whole cost of two D
rows:

| row (benchmark/baseline, `min` ms) | mfb -O1 | C -O0 | Python | grade |
|---|---:|---:|---:|---|
| `scalarbench transform` | 0.454 | 0.024 | 1.214 | D 18.9× |
| `scalarbench listchurn` | 5.601 | 0.297 | 9.089 | D 18.9× |
| `scalarbench roundtrip` (same seam, secondary) | — | — | — | C 9× (`./benchmark/rank.py`) |

(From `benchmark/baseline/{mfb-O1,c-O0,python}.log`, `./benchmark/rank.py`. `-O2`/`-O3`: transform 0.427 / 0.425,
listchurn 4.999 / 5.023.) The rows are in `benchmark/mfb/src/scalarbench.mfb`. `test_scalar_transform` runs
200 passes of {43 appends of `toScalar(rot13(toInt(sc)))`, one `strings::fromScalars` of 43 scalars, a 43-element sum}.
`test_scalar_listchurn` runs 2000 passes of {`strings::toScalars` of a 90-byte ASCII string, 89 `get`-pair compares}.

**The single correct behavior a fix produces:** `toScalars` and `fromScalars` each make **exactly one arena allocation
per call** (the result) at every n, and neither frees anything per scalar. Their per-scalar cost falls to **≤ 5 ns** at
n = 4,300 on the reference host. The measured values today are 28 and 39 ns. The target is justified by the C -O0 peer, which does the
whole transform pass (decode, ROT-13, encode and sum) in 24 µs / 200 / 43 = 2.8 ns per scalar, and by the
native UTF-8 walkers the compiler already emits for other `strings` builtins (`emit_utf8_decode_next`,
`emit_utf8_encode_next`). For the rows, the **expected** (guess, from the component split below) result is transform ≈ 0.15 ms and
listchurn ≈ 1.5–2 ms, both in the C band. The per-call allocation count and the per-scalar ns target are the acceptance
criteria.

References:

- `mfb spec language types` §4 (`src/docs/spec/language/04_types.md`, the `Scalar` paragraph): `toScalars`/`fromScalars`
  "iterate and rebuild". The contract is only the value produced.
- `mfb man strings toScalars` / `fromScalars` (`src/codegen/builtins/strings/func_to_scalars.rs`,
  `func_from_scalars.rs` `DESC`): `fromScalars(toScalars(s)) = s`; no error of their own.
- `mfb spec memory arenas` → *Entropy Fill*: every freed chunk larger than 16 B is scrubbed. It stays on
  (`planning/todo.md` item 2), and that file measured `scalarbench.listchurn` at ×0.78–0.89 with the fill off.
- Related: `bugs/bug-778-every-inline-trap-evaluation-heap-allocates-a-result.md` (the per-element `toScalar(cp) TRAP`
  cost, in general), `bugs/bug-769-set-map-insert-pays-the-free-scrub-of-every-grow.md` (scrub cost on growth) and
  `bugs/bug-771-groupby-and-partition-churn-full-size-transient-blocks.md` (same cluster, transient-byte root cause).
- Found by: benchmark D-grade triage, cluster D, 2026-09-28.

## Failing Reproduction

```
mkdir -p /tmp/b772/src && cd /tmp/b772
printf '{"name":"b772","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}\n' > project.json
# src/main.mfb = the program below
/Users/justinzaun/Development/mfb/target/release/mfb build -O 1 -q . && ./build/b772.out
```

```
IMPORT io
IMPORT strings
IMPORT encoding
IMPORT datetime

FUNC main AS Integer
  MUT base AS String = "The Quick Brown Fox Jumps Over The Lazy Dog"
  FOR sz = 0 TO 3
    MUT acc AS Integer = 0
    LET reps = 100000 / len(base)
    MUT t0 = datetime::monotonicNanos()
    FOR pass = 1 TO reps
      LET l AS List OF Scalar = strings::toScalars(base)
      acc = acc + len(l)
    NEXT
    io::print("toScalars n=" & toString(len(base)) & ": " & toString((datetime::monotonicNanos() - t0) / (reps * len(base))) & " ns/scalar")
    LET l2 AS List OF Scalar = strings::toScalars(base)
    t0 = datetime::monotonicNanos()
    FOR pass = 1 TO reps
      LET s AS String = strings::fromScalars(l2)
      acc = acc + len(s)
    NEXT
    io::print("fromScalars n=" & toString(len(base)) & ": " & toString((datetime::monotonicNanos() - t0) / (reps * len(base))) & " ns/scalar")
    LET cps = encoding::utf32Encode(base)
    t0 = datetime::monotonicNanos()
    FOR pass = 1 TO reps
      LET s AS String = encoding::utf32Decode(cps)
      acc = acc + len(s)
    NEXT
    io::print("utf32Decode n=" & toString(len(base)) & ": " & toString((datetime::monotonicNanos() - t0) / (reps * len(base))) & " ns/cp")
    t0 = datetime::monotonicNanos()
    FOR pass = 1 TO reps
      LET c = encoding::utf32Encode(base)
      acc = acc + len(c)
    NEXT
    io::print("utf32Encode n=" & toString(len(base)) & ": " & toString((datetime::monotonicNanos() - t0) / (reps * len(base))) & " ns/cp")
    base = base & base & base & base & base & base & base & base & base & base
  NEXT
  RETURN 0
END FUNC
```

Observed (macOS aarch64 M2 Max, `target/release/mfb` at `7e53c82b9`, 2026-09-28):

```
toScalars n=43: 29 ns/scalar      fromScalars n=43: 41     utf32Decode n=43: 29     utf32Encode n=43: 11
toScalars n=430: 24               fromScalars n=430: 37    utf32Decode n=430: 28    utf32Encode n=430: 9
toScalars n=4300: 28              fromScalars n=4300: 39   utf32Decode n=4300: 28   utf32Encode n=4300: 11
toScalars n=43000: 36             fromScalars n=43000: 41  utf32Decode n=43000: 28  utf32Encode n=43000: 15
```

- Observed: flat 24–41 ns per scalar. This is linear, not quadratic; the constant is the bug.
- Expected: ≤ 5 ns per scalar, and one allocation per call.

**Deterministic counters** (`--debug` build, a 1000-call loop of one operation, `arena.0.*` divided by 1000):

| call | `alloc_calls` / call | `alloc_bytes` / call |
|---|---:|---:|
| `toScalars`, n = 43 | 56 | 3,536 |
| `toScalars`, n = 430 | 449 | 27,344 |
| `fromScalars`, n = 43 | 99 | 4,400 |
| `fromScalars`, n = 430 | 879 | 38,603 |

So `toScalars` makes ≈ 1 allocation per scalar, and `fromScalars` ≈ 2.

**Component split of the two rows** (per pass, same binary, `/tmp/dbD-sc` probe, 2000 passes each):

| component | ns/pass |
|---|---:|
| 43 × `out = collections::append(out, sc)` into a fresh `List OF Scalar` | 310 |
| `strings::fromScalars` of 43 | 1,741 |
| `FOR EACH` sum of `toInt` over 43 | 53 |
| `strings::toScalars` of 43 | 1,257 |
| 42 `get`-pair `<` compares | 179 |

The `transform` row's pass (0.454 ms / 200 = 2.27 µs) is ~77% `fromScalars`. The `listchurn` row's pass
(5.601 ms / 2000 = 2.8 µs) is ~90% `toScalars` of 90 scalars (derived: 90 × 28 ns ≈ 2.5 µs). I ran `sample` on a
10⁸-pass listchurn kernel and attributed samples to functions by disassembly (the store loop holding the PCG64 multiplier is
`arena_fill_random`; the functions with 279 and 408 `bl` callers are `arena_alloc` and `arena_free`). The shares were
**entropy scrub 39.3%, alloc 11.0%, free 4.7%**, so more than half the row is the allocator.

Contrast: `encoding::utf32Encode` (one layer down, and it does not wrap each element) runs at 9–15 ns per code point, and a
plain `FOR EACH … toInt` sum at ~1.2 ns per scalar.

## Root Cause

Both members are `Body::Rewrite` entries (`src/codegen/builtins/strings/func_to_scalars.rs:register`,
`func_from_scalars.rs:register`). They route to source FUNCs in the `WhenUsed` helper chunk
`src/codegen/builtins/strings/helper_scalar_seam.rs:SEAM`, which calls further source FUNCs in `encoding`. The `.ncode`
for `#strings_toScalars` confirms it: `bl _mfb_ifn_encoding_5Futf32Encode`, then a `for_each` loop containing
`_mfb_arena_alloc` … `_mfb_arena_free` per iteration.

**`toScalars(s)`** = `__strings_toScalars`:

1. `encoding::utf32Encode(s)` → `__encoding_codepoints` (`src/codegen/builtins/encoding/helper_codepoints.rs:BODY`):
   `strings::toBytes(value)` copies the string into a `List OF Byte` (1 alloc). A byte-at-a-time `WHILE` loop then
   `collections::append`s each code point to a `List OF Integer` that starts empty, so it regrows geometrically (log n allocs,
   each old block freed and scrubbed).
2. The seam's `FOR EACH cp IN cps` does `LET sc AS Scalar = toScalar(cp) TRAP(err) … END TRAP` per element. An inline
   `TRAP` on a call lowers through a heap `Result` value.
   `src/codegen/memory/arena/builder_arena_transfer.rs:CodeBuilder::emit_build_result_inline` allocates a flat
   `{tag, size, payload}` block (24 B, a 32 B chunk) for **every** evaluation, including the successful ones. The
   raw-result path in `src/codegen/engine/value/builder_values.rs` unpacks it, and it is then freed and scrubbed
   (16 B past the node). The `.ncode` shows `result_inline_alloc_ok` → `_mfb_arena_free` inside `for_each_loop`. This is
   the ≈ 1 allocation per scalar. Measured in isolation (probe `/tmp/dbD-trap`), 50 evaluations of
   `LET sc AS Scalar = toScalar(cp) TRAP(err) … END TRAP` cost 560 ns against 291 ns without the TRAP, and `--debug` counts
   exactly one 32 B allocation per evaluation (500,000 over 10,000 × 50).
3. `out = collections::append(out, sc)` into a `List OF Scalar` that starts empty, so it regrows geometrically again.
4. `cps` is dropped (freed and scrubbed).

The TRAP is dead code by construction. The seam comment says `utf32Encode` yields only valid code points, "so
`toScalar` never actually fails".

**`fromScalars(xs)`** = `__strings_fromScalars`:

1. `cps = collections::append(cps, toInt(sc))` per scalar into an empty `List OF Integer` (log n regrows).
2. `encoding::utf32Decode(cps)` (`src/codegen/builtins/encoding/func_utf32_decode.rs:BODY`): the `.ncode` opens with a
   `flat_copy` of the parameter list (1 alloc of the whole list). Per code point it runs two range checks, then
   `out = out & __encoding_fromCodepoint(cp)`. `__encoding_fromCodepoint`
   (`src/codegen/builtins/encoding/helper_from_codepoint.rs`) builds a `List OF Byte` literal (`[toByte(value)]`) and
   `toString`s it. That is **2 allocations per code point**, and the one-scalar `String` is then appended and dropped.
   This is the ≈ 2 allocations per scalar.
3. The TRAP around `utf32Decode` is again dead: every `Scalar` is a valid non-surrogate code point.

Nothing here is quadratic, since the in-place append and concat keep it linear (the counters grow ∝ n). The cost is
the constant: per scalar, 1–2 heap round-trips with a scrub, a callback-shaped TRAP unpack, and several list passes
where one UTF-8 walk suffices.

## Goal

- `toScalars(s)`: one pass over the UTF-8 bytes into an exactly-sized `List OF Scalar` (kind 2, 4-byte payload). Exactly
  one `arena_alloc` per call.
- `fromScalars(xs)`: one pass to sum the UTF-8 widths, one allocation of the exact `String`, and one encode pass. Exactly one
  `arena_alloc` per call.
- ≤ 5 ns per scalar at n = 4,300 for each, on the reference host (Failing Reproduction program).
- Byte-identical results for every input, including non-ASCII, 4-byte scalars and the empty string/list.

### Non-goals (must NOT change)

Owner constraint: no language-spec or memory-model change, only internal lowering.

- The `strings::toScalars` / `fromScalars` signatures, results, and error behavior are unchanged. They raise nothing of their own,
  and `ErrOutOfMemory` stays the only failure. `fromScalars(toScalars(s)) = s` must still hold. The man pages must not change.
- `toScalar(Integer)`'s own semantics are unchanged: it is still fallible for surrogates and out-of-range values when a program calls it.
  The fix removes the *seam's* call to it, whose argument is valid by construction. It does not change the conversion.
- `encoding::utf32Encode` / `utf32Decode` behavior is unchanged: their range/surrogate errors and error codes 77050003.
  Rewriting them natively is allowed but not required here (see Blast Radius).
- The memory model is unchanged. The result is an ordinary owned `List OF Scalar` / `String` with tight capacity
  (`mfb spec memory collections` → *Capacity Headroom and Growth*: "known-size builders … allocate exactly"). The entropy
  scrub stays.
- The five classification predicates in the same `SEAM` chunk and their `WhenUsed` gate keep working. The chunk must still
  build when only `isLetter` etc. are referenced, and when `astrings` injects it (`HelperGate::WhenImported("astrings")`).
- **Tempting wrong fixes, forbidden:** shrinking the benchmark's pass counts, since the `TODO(plan-39-A)` comments in
  `scalarbench.mfb` are about arena churn and the fix is to remove the churn, not to hide it. Deleting the TRAP alone
  and keeping the source seam is a partial fix and does not meet the Goal. Dropping the scrub is also forbidden.

## Blast Radius

Found with `grep -rn "toScalars\|fromScalars\|__encoding_fromCodepoint\|__encoding_codepoints" src/codegen`:

- `strings::toScalars` / `fromScalars` (`func_to_scalars.rs`, `func_from_scalars.rs`, `helper_scalar_seam.rs`).
  **Fixed by this bug.**
- Callers that benefit with no change: `astrings/helper_scalar_count_str.rs` (`len(strings::toScalars(s))`),
  `term/helper_astrings_bridge.rs:157`, `astrings/helper_leading_in_set.rs`, `helper_md_escape*.rs`, and the
  `scalarbench roundtrip` row (C, 9×).
- `encoding::utf32Decode`, `utf16Decode`, `codepageDecode`, `htmlUnescape`: the same `out = out & __encoding_fromCodepoint(cp)`
  per-code-point pattern (2 allocs per code point; utf32Decode measured at 28 ns/cp). **Latent, same hazard, out of scope:** no
  D/F row exercises them (`./benchmark/rank.py` lists encoding at hex 11×, base64 9.9×, percent 8×). They would share the
  native encode helper this fix adds, so it is a natural follow-up.
- `__encoding_codepoints` (behind `utf32Encode`, `utf16Encode`, `codepageEncode`, `punycodeEncode`): a `toBytes` copy plus
  byte-at-a-time append, 9–15 ns/cp. **Latent, out of scope**, for the same reason.
- The general cost of an inline `TRAP`: `emit_build_result_inline` allocates a heap `Result` on every evaluation, success
  included (+5.4 ns and one 32 B alloc per evaluation here, and one per fallible node in general). **Tracked in
  `bugs/bug-778-every-inline-trap-evaluation-heap-allocates-a-result.md`.** This bug avoids it on the seam path by
  removing the dead TRAP. bug-778 removes the cost everywhere else.
- `regex/helper_to_scalars.rs` (`__regex_toScalars`, returns `List OF String`): a different function, so it is unaffected.

## Fix Design

Give `toScalars` and `fromScalars` native lowerings: a `Body::AbiInline` (the "single sanctioned inline shape", `registry/mod.rs:Body`)
or a shared `AbiFunction` runtime helper if the inline body is too large. Build them from the UTF-8 primitives that already exist in
`src/codegen/string/unicode_props.rs`: `emit_utf8_decode_next`, `emit_utf8_encoded_width` and `emit_utf8_encode_next`
(used today by `gen_graphemes.rs`).

- `toScalars`: pass 1 counts scalars. That is the count of bytes with `(b & 0xC0) != 0x80`, since a `String` is always valid UTF-8, so this
  is exact. Allocate `HEADER + count × 4` once, write a tight kind-2 header, then decode into it.
- `fromScalars`: pass 1 sums `emit_utf8_encoded_width` over the 4-byte payloads. Allocate the string block once, then encode.
- Remove `__strings_toScalars` / `__strings_fromScalars` from `SEAM`, and keep the five predicates and their gate. Before
  deleting the FUNCs, confirm that no other chunk calls the `__strings_*Scalars` symbols (the grep above found none outside
  the seam). Update `SEAM_MEMBERS` so the gate still opens for the predicates.

Where the risk sits: the kind-2 `List OF Scalar` header and payload width must match what `collections::*` reads for
`Scalar` (4-byte payload, `list_entry_stride` 0), and the String layout must match `mfb.string.v1`. Both are exercised by
`scalar-strings-seam-rt` and by the benchmark checksums. Expected golden shift: **20** `.ir` goldens carry the seam
bodies as functions named `#strings_toScalars` / `#strings_fromScalars`
(`grep -rlE '"name": "#strings_(to|from)Scalars"' tests | wc -l` → 20). Examples:
`tests/rt-behavior/general/scalar-strings-seam-rt/golden/scalar_strings_seam_rt.ir`, 11 `astrings` fixtures,
`tests/byte-identity/strings/golden/strings_codegen_cover_rt.ir`, and `regex-surface-parity-rt`. In each, the function body
disappears and the call site stays. That is the intended diff. Any other change in those files is a bug-hunt trigger, and
each fixture's program output must be unchanged.

Rejected: (1) Only dropping the dead `TRAP` in the seam. It removes ~1 alloc per scalar from `toScalars` but leaves
`fromScalars`'s 2 per scalar and all the list passes, so it cannot reach the Goal. (2) Rewriting `utf32Encode`/`Decode` natively
and keeping the seam on top. That still leaves the intermediate `List OF Integer` and its copy.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_scalar_seam_alloc.rs`, modeled on
      `tests/runtime/rt_debug_arena.rs::fill_counters_match_known_allocations`: `--debug` builds of 1000 and 2000 calls
      of `toScalars` (n = 430, mixed ASCII/2/3/4-byte scalars) and of `fromScalars`. Assert `delta(alloc_calls) == 1000`
      (one per call) and assert the printed checksum. It fails today (≈ 449 and ≈ 879 per call).
- [ ] Extend `scalar-strings-seam-rt` (or add a sibling fixture) with a round-trip over every UTF-8 width, the empty
      string, and a string whose scalar count differs from its byte count. It must pass today and keep passing.
- [ ] Confirm the Blast Radius verdicts by grep and record them.

Acceptance: the allocation test fails for the documented reason; the behavior fixture passes.
Commit: —

### Phase 2 — the fix

- [ ] Native `toScalars` / `fromScalars` lowering (Fix Design). Switch the two `register` bodies off `Body::Rewrite`.
- [ ] Trim `SEAM` / `SEAM_MEMBERS` in `helper_scalar_seam.rs`, keeping the predicates.

Acceptance: the Phase 1 tests pass; `scalar-strings-seam-rt` output is unchanged; the man examples for both members
(`scripts/man-run-examples.sh strings --run`) still print the same output.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate only the goldens that inlined the seam. Diff each one and confirm that the only change is the removed seam FUNCs
      and the new call lowering. Localize any other diff to a single fixture first (AGENTS.md).
- [ ] Re-run the Failing Reproduction (≤ 5 ns/scalar) and `./benchmark/run.sh 10`. The `scalarbench` rows improve, the
      `scalar_*` checksums still cross-validate against C and Python, and no other row regresses beyond noise.
- [ ] Update any `mfb spec` text that describes the scalar seam as a source body (`scripts/spec-census.sh --citations`).
- [ ] Full suite.

Acceptance: full suite green; golden deltas are exactly the removed seam; the benchmark checksums are unchanged.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_scalar_seam_alloc.rs` (one allocation per call) and the extended
  `scalar-strings-seam-rt` (byte-identical round trips).
- Runtime proof: the Failing Reproduction per-scalar numbers, plus the `scalarbench transform`/`listchurn`/`roundtrip` rows
  and their `./benchmark/rank.py` grades.
- Doc sync: no `mfb man` change. Update `mfb spec` only where it names the seam's implementation.
- Full suite: `cargo test` per `.ai/compiler.md` *Hard Completion Gate*, plus `artifact-gate`/`test-accept` per
  `.ai/testing-gates.md`.

## Open Decisions

- `AbiInline` or a shared `AbiFunction` helper? Recommended: `AbiFunction`. Two UTF-8 loops at every call site would
  bloat the code, and the call overhead is amortized over n. The alternative is `AbiInline` for code-size parity with the other
  `strings` inline members.
- Fold the native encode helper into `encoding::utf32Decode` etc. in the same change? Recommended: no, file it as a follow-up,
  since no D/F row needs it.

## Summary

The engineering is small and well precedented, because the UTF-8 walkers already exist. The risk is confined to getting the
kind-2 `Scalar` list and string layouts exactly right, and to regenerating the handful of goldens that inline the seam.
Everything observable stays the same: signatures, errors, round-trip identity and the predicates.

## Resolution (2026-10-03)

Both members are `Body::abi_inline` lowerings in
`src/codegen/builtins/strings/gen_scalar_seam.rs`, in the `gen_case_map` shape:

- `toScalars`: one `emit_utf8_decode_next` walk counts the scalars, one allocation of
  an exactly-sized kind-2 `List OF Scalar` (header + 4 bytes each), and a second walk
  stores each code point.
- `fromScalars`: one pass sums `emit_utf8_encoded_width` over the payloads, one
  allocation of the exact `String` (`len + 9`), and a second pass encodes with
  `emit_utf8_encode_next`.

Both now declare `ErrOutOfMemory`, which they can raise. The seam chunk's
`__strings_toScalars`/`__strings_fromScalars` (called by name from the `astrings`
companion) delegate to the members, the two members leave the seam's `WhenUsed`
gate, and the chunk no longer imports `collections` or `encoding`. So an `astrings`
program stops pulling in all 61 `#encoding_*` functions; its `.ir` changes in exactly
that set, the call targets (`#strings_toScalars` → `strings.toScalars`), and the
predicates' line numbers.

Measured with the Failing Reproduction program (`-O 1`, macOS M2 Max):

| n | toScalars ns/scalar | fromScalars ns/scalar |
| --- | ---: | ---: |
| 43 | 29 → 2 | 41 → 2 |
| 430 | 24 → 1 | 37 → 2 |
| 4,300 | 28 → 1 | 39 → 2 |
| 43,000 | 36 → 2 | 41 → 2 |

`rt_scalar_seam_alloc`: exactly 1 allocation per call for ASCII, mixed 1–4-byte, and
empty input (main: 56 and 99 per call at n = 43), with `fromScalars(toScalars(s)) = s`
and one `Scalar` per Unicode scalar. The strings/astrings fixtures run and print
unchanged; the `strings` `toScalars`/`fromScalars` and all 17 `astrings` man examples run.
Spec: `architecture/02_frontend.md`.
