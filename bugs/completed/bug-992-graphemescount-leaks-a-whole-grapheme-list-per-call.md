# bug-992: `strings::graphemesCount` and `strings::graphemeAt` leak a whole `List OF String` per call

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory)

Status: Fixed (2026-10-10)
Regression Test: `tests/runtime/rt_strings_unowned_blocks.rs` (`--debug` `alloc_calls`/`free_calls`/`live_bytes` at N = 100 and 200)

Every call to `strings::graphemesCount(s)` builds the complete `List OF String` of `s`'s
grapheme clusters, reads its `count` word, and drops the block with no owner. Nothing ever
frees it, so a loop that counts graphemes grows the main arena by
`round16(40 + 40 × clusters + byteLen(s))` bytes per call for the life of the process. On
the 148-byte ASCII string from the repro that is 6,112 B of live arena per call and
**~16 KB of RSS per call**: 200,000 calls peak at 3.28 GB. `strings::graphemeAt(s, i)` (the
copying lowering, used by every call except `s = strings::graphemeAt(s, i)`) builds the
same list and leaks it the same way. It was found while building `examples/canvas-demo`: the
bestiary scene, which calls `graphemesCount` on each creature's lore text every frame, grew
about 170 MB/min (reported by the task that found it; this doc did not re-measure it).

A third, unrelated leak turned up in the same package audit: `strings::fromScalars` leaks
its own `String` result when nothing binds it (`round16(8 + byteLen)` per call). Its
producer never sets the bug-536 freshness mark, so the statement-scope free fails closed.
It is in scope here (Sub-issue B) because the audit found it and it has the same shape: a
fresh block with no owner.

**The single correct behavior a fix produces:** a loop of N calls to `graphemesCount`,
`graphemeAt`, or an unbound `fromScalars` reports the same main-arena `live_bytes` at N and
2N under `mfb build --debug`, and its peak RSS does not grow with N.

References:

- `mfb spec unicode` (01_tables-and-algorithms, "Runtime algorithm: grapheme segmentation")
  defines the three grapheme members as one segmentation; it says nothing about the count
  allocating.
- `.ai/codegen-invariants.md`, "Bare `String` temps: fail-closed freshness provenance": why
  an unmarked `String` producer leaks (Sub-issue B).
- bug-536 (`bugs/completed/bug-536-scope-drop-leaks-recursive-types-return-constructor-string-temps.md`),
  shape B: the interior-block and fresh-mark mechanisms this fix reuses.
- bug-772 (`bugs/completed/bug-772-toscalars-fromscalars-allocate-per-scalar.md`): made
  `fromScalars` native. `tests/runtime/rt_scalar_seam_alloc.rs` counts its *allocations*
  (one per call) but never its frees, which is why Sub-issue B went unnoticed.
- Found during `examples/canvas-demo` (bestiary scene).

## Failing Reproduction

`/tmp/cdleak/src/main.mfb` (a console project; `os::args()` index 0 selects the mode)
loops 200,000 times over a record whose `lore` field is a 148-byte ASCII sentence:

```
FUNC countOnly(b AS Beast) AS Integer
  RETURN strings::graphemesCount(b.lore)
END FUNC
FUNC midOnly(b AS Beast) AS Integer
  RETURN len(strings::mid(b.lore, 1, 100))
END FUNC
```

```
target/release/mfb build /tmp/cdleak
/usr/bin/time -l /tmp/cdleak/build/cdleak.out count
```

- Observed (re-measured 2026-10-06 with the command above): `count` → `3277897728  maximum resident set size`.
- Expected: about the same as `none`, `1081344  maximum resident set size`.

The same run's contrast modes: `none` 1,081,344 B; `mid` (`strings::mid` alone) 1,081,344 B;
`literal`, `local` and `field` (`graphemesCount` + `mid`, with the argument from a literal, a
local copy, or a record field) all 3,277,897,728 B. So the leak follows `graphemesCount`
whatever the argument's source is, and `mid` is not involved.

### Exact size per call

Measured with `/tmp/gleak/dbg.sh`, which builds a one-function probe
(`RETURN <expr>` over `LET s AS String = b.s`) at N = 100 and N = 200 with
`mfb build --debug` and diffs the main arena's `live_bytes`:

| `s` | clusters | bytes | live B / call | `round16(40 + 40·g + bytes)` |
| --- | --- | --- | --- | --- |
| `""` | 0 | 0 | 48 | 48 |
| `"a"` | 1 | 1 | 96 | 96 |
| 8 × `a` | 8 | 8 | 368 | 368 |
| 26 × `a` | 26 | 26 | 1,120 | 1,120 |
| 64 × `a` | 64 | 64 | 2,672 | 2,672 |
| 148 × `a` | 148 | 148 | 6,112 | 6,112 |
| 512 × `a` | 512 | 512 | 21,040 | 21,040 |
| `a😀b👍🏽c` | 5 | 15 | 256 | 256 |

So the leak is **exactly one `List OF String` block per call**: a 40 B header, a 40 B entry
per cluster, and a copy of the string's bytes. It is proportional to the cluster count, not
fixed. The emoji row shows it counts clusters, not scalars or bytes (`👍🏽` is two scalars,
one cluster). `baseline` (`RETURN 0`) and `len(s)` measure 0 B / call.

Peak RSS is amplified over the live bytes. Measured with `/tmp/gleak/probe.sh` (peak RSS
at N = 20,000 and 40,000): 64 to 256 bytes all cost 16,384 B of RSS per call, and 512 bytes
costs 32,769 B. For the 148-byte repro string that is 16,384 RSS B against 6,112 live B per
call, about 2.7×. Guess, not measured: the arena gets a fresh chunk for each large request
because nothing on the free list fits, and macOS charges RSS in 16 KB pages. The fix makes
this moot.

### Blast radius probe (all 2026-10-06, `dbg.sh`, 47-byte ASCII `s`, live B / call)

Leaking:

| expression | live B / call |
| --- | --- |
| `strings::graphemesCount(s)` | 1,968 |
| `len(strings::graphemeAt(s, 3))` | 1,968 |
| `strings::byteLen(strings::graphemeAt(s, 3))` | 1,968 |
| `len(strings::fromScalars(strings::toScalars(s)))` | 64 (= `round16(8 + 47)`); 16 for `s = "a"` |

Flat, 0 B / call: `len(strings::graphemes(s))`, `strings::displayWidth`,
`padLeftToWidth`, `padRightToWidth`, `padLeft` (2- and 3-arg), `padRight`, `toScalars`,
`len(strings::toScalars(strings::upper(s)))`, `left`, `right`, `mid`, `split`, `upper`,
`lower`, `upper(lower(s))`, `normalizeNfc`, `caseFold`, `byteLen`, `toBytes`, `count`,
`find`, `replace`, `trim`, `trimStart`, `trimEnd`, `trimChars`, `stripPrefix`,
`stripSuffix`, `repeat`, `join(split(...))`.

`graphemeAt` leaks the list, not its result: the 1,968 B is exactly the 47-cluster list
(`round16(40 + 1880 + 47)`), and the result `String` it materializes is freed.

## Root Cause

### Sub-issue A: the grapheme list built for `graphemesCount` and `graphemeAt` has no owner

All three grapheme members share
`src/codegen/builtins/strings/gen_graphemes.rs:lower_strings_graphemes`. It makes two passes
over `value`:

1. a **count pass** that walks scalars through the UAX #29 break machine
   (`emit_grapheme_break_branch` / `emit_grapheme_state_update`) and stores the cluster count
   in its `strings_graphemes_count` slot;
2. a `_mfb_arena_alloc` of `COLLECTION_HEADER_SIZE + count × COLLECTION_ENTRY_SIZE + byteLen`
   and a **write pass** that fills one entry per cluster.

It returns the list block as a `ValueResult`. For `strings::graphemes` that is the member's
own result, so `lower_value` → `register_pending_temp`
(`src/codegen/engine/value/builder_values.rs`) frees it at statement end, or a binding
claims it. That is why `graphemes` is flat.

`src/codegen/builtins/strings/func_graphemes_count.rs:lower` calls
`lower_strings_graphemes`, spills the list to `strings_graphemes_count_list`, loads
`COLLECTION_OFFSET_COUNT`, and returns that **Integer**. The list block is an *interior*
allocation. `lower_value` sees only the Integer result, so `register_pending_temp` never
sees the block, and nothing else frees it.

`src/codegen/builtins/strings/func_grapheme_at.rs:lower` (the copying lowering) does the
same: it builds the list, reads one entry's `(offset, length)`, and copies those bytes out
through `emit_materialize_string_from_bytes`. The returned `String` is marked fresh and
freed. The list it was copied from is never freed.

This is bug-536 shape B (`strings::padLeft`'s interior one-byte pad `String`) for a
`List OF String` instead of a `String`.

The in-place arm `gen_graphemes.rs:grapheme_at_window` (used only for
`s = strings::graphemeAt(s, i)`, via `string_self_update.rs`'s `WindowKind::GraphemeAt`)
already walks the clusters with no allocation. Its doc comment says the copying lowering
"allocates", but nothing ever frees that allocation.

### Sub-issue B: `strings::fromScalars` never marks its result fresh

`src/codegen/builtins/strings/gen_scalar_seam.rs:lower_from_scalars` allocates exactly one
block (`length + 9`) and returns it. It never calls `builder.mark_fresh_string(...)`, unlike
every other native `String` producer in the package (`func_repeat.rs`,
`func_normalize_nfc.rs`, `gen_pad.rs`, `func_join.rs`, `gen_case_map.rs`). By the bug-536
fail-closed rule, `pending_temp_is_freeable` refuses an unmarked bare `String`, so an
*unbound* `fromScalars` result (a `len(...)` operand, a comparison, a concatenation input)
leaks. A bound result is claimed by its binding and is fine. bug-772's
`rt_scalar_seam_alloc` asserts one allocation per call but not one free, so it passes.

## Goal

- `graphemesCount` allocates nothing: `alloc_calls` and `live_bytes` are the same at N and
  2N.
- The copying `graphemeAt` leaves only its result `String` live, and frees that at statement
  end when nothing binds it: `live_bytes` is flat at N and 2N.
- An unbound `fromScalars` result is freed at statement end: `live_bytes` is flat at N and
  2N.
- `/tmp/cdleak` mode `count` peaks at about the same RSS as mode `none` (about 1 MB).

### Non-goals (must NOT change)

- Segmentation results. `graphemesCount`, `graphemeAt` and `graphemes` must still agree
  cluster for cluster, including the Indic-conjunct limitation in `mfb spec unicode`.
- `graphemeAt`'s `ErrIndexOutOfRange` contract (negative or ≥ count) and its error code.
- `strings::graphemes`' result layout, and the in-place `grapheme_at_window` arm.
- Do not "fix" `graphemesCount` by registering its list as a pending temp and keeping the
  allocation. That makes the arena flat but still allocates and entropy-scrubs
  `O(clusters × 41)` bytes per call to return one integer (bug-771's cost model). It is
  acceptable only as a fallback for `graphemeAt` if the window walk cannot be reused there
  (see Fix Design).
- Do not mark `fromScalars`' result fresh on any path other than the one that has just
  allocated it. An empty-list fast path that returns a shared constant (if one is ever
  added) must not be marked.

## Blast Radius

Found by `grep -rn lower_strings_graphemes src` (three callers) and the `dbg.sh` sweep above
over every public `strings` member that returns a fresh block:

- `func_graphemes_count.rs:lower`: leaks the list. **Fixed by this bug (A).**
- `func_grapheme_at.rs:lower` (copying arm): leaks the list. **Fixed by this bug (A).**
- `func_graphemes.rs:lower`: returns the list as its own result, freed or claimed. Unaffected,
  measured 0 B.
- `gen_graphemes.rs:grapheme_at_window`: allocates nothing. Unaffected.
- `gen_scalar_seam.rs:lower_from_scalars`: unmarked fresh `String`. **Fixed by this bug (B).**
- `gen_scalar_seam.rs` `toScalars`: returns a `List OF Scalar`, which is not a bare `String`,
  so no mark is needed. Measured 0 B.
- `displayWidth`, `padLeftToWidth`, `padRightToWidth`: walk clusters/scalars without calling
  `lower_strings_graphemes`. Measured 0 B.
- Every other `strings` member in the sweep: measured 0 B. Unaffected.

Outside `strings`: this doc did not audit the rest of the tree for interior collection blocks
built only to read one word. A grep for a call that builds a list and returns only a scalar
read from it (`COLLECTION_OFFSET_COUNT` after a builder) is Phase 1's job.

## Fix Design

**A, `graphemesCount`:** split `lower_strings_graphemes`' count pass into its own helper,
for example `gen_graphemes.rs:emit_grapheme_count`, which returns the count register. Call
it both from `lower_strings_graphemes` (so `graphemes` keeps the same two-pass shape) and
directly from `func_graphemes_count.rs:lower`. `graphemesCount` then makes no allocation at
all. The count pass already exists and already computes the count from the string alone,
so this moves code without adding any.

With no allocation left, `graphemesCount`'s descriptor `errors: vec!["ErrOutOfMemory"]`
becomes a claim its lowering no longer raises. bug-698 removed exactly this kind of
mismatch. Drop the entry and re-render `mfb man strings graphemesCount` (see Open Decisions).

**A, `graphemeAt` (copying):** reuse `grapheme_at_window(builder, value, index)` to get the
`(ptr, len)` window into `value`'s own bytes, then `emit_materialize_string_from_bytes`, as
today's tail does. The window walk raises the same `ErrIndexOutOfRange` and stops at the
wanted cluster, so the list and its O(n) allocation go away. Risk: `grapheme_at_window`
was written for the self-update site. Check that its raise and its slot use are valid at an
ordinary expression site, and that the error is reported under the same member name
(`strings.graphemeAt`). Fallback if they are not: keep the list and register it as an
interior pending temp (a `List OF String` analogue of `register_fresh_string_temp`). That
fixes the leak but not the allocation.

**B, `fromScalars`:** call `builder.mark_fresh_string(Operand::from(result))` on the
register `lower_from_scalars` returns, right after the alloc-ok join, as `func_join.rs` and
`func_repeat.rs` do.

Expected `.ncode`/IR shift: every golden that calls `graphemesCount`, a copying
`graphemeAt`, or an unbound `fromScalars` changes. Run
`grep -rl "graphemesCount\|graphemeAt\|fromScalars" tests --include=*.mfb` to find the
fixtures. Nothing else should move. An unexpected diff elsewhere is a bug-hunt trigger.

## Phases

### Phase 1: failing test + audit (no behavior change)

- [x] Add `tests/runtime/rt_strings_unowned_blocks.rs`, built on `common::debug_report`
      (`build_debug`, `run_ok`, `arena_lines`, `counter`), like `rt_debug_soak.rs`'s
      `assert_live_bytes_within`. Each case puts the expression in a loop whose count is a
      literal `{N}`, builds it at N = 100 and 200, and asserts main-arena `live_bytes` growth
      is below `BLOCK_BOUND` (4096). The RED cases are
      `strings::graphemesCount(s)`, `len(strings::graphemeAt(s, 3))` and
      `len(strings::fromScalars(strings::toScalars(s)))`, where `s` is a record field
      (`b.lore`, as in the repro) so it is not a static string. Also pin the measured-flat
      siblings as POSITIVE cases (`graphemes`, `displayWidth`, `mid`, a bound
      `fromScalars`) so that a fix which disables an allocation is caught. Use
      `#![cfg(unix)]` as `rt_debug_soak.rs` does.
- [x] Add one value check per member (ASCII, emoji ZWJ/modifier, empty string,
      `graphemeAt` at 0, at the last cluster, and out of range → `ErrIndexOutOfRange`). The
      leak tests must not pass because a value changed.
- [x] Confirm the three RED cases fail at about 1,968 / 1,968 / 64 B per iteration for a
      47-byte `s`, and the positive cases pass.
- [x] Do the outside-`strings` grep from Blast Radius and record a verdict per site here.

Acceptance: the RED cases fail for the documented reason, the positive pins pass, and the
audit list is complete.
Commit: —

### Phase 2: the fix

- [x] `gen_graphemes.rs`: extract the count pass into `emit_grapheme_count` and use it from
      `lower_strings_graphemes` and `func_graphemes_count.rs:lower`. Remove
      `graphemesCount`'s `ErrOutOfMemory` descriptor entry once it cannot raise it.
- [x] `func_grapheme_at.rs:lower`: route through `grapheme_at_window` +
      `emit_materialize_string_from_bytes`, or use the interior-temp fallback (record which
      one here and why).
- [x] `gen_scalar_seam.rs:lower_from_scalars`: `mark_fresh_string` on the returned block.
- [x] Update the `gen_graphemes.rs:grapheme_at_window` doc comment, which says the copying
      lowering allocates.

Acceptance: the Phase 1 tests pass, the value checks are unchanged, and
`rt_scalar_seam_alloc` still passes (`fromScalars` still allocates once per call).
Commit: —

### Phase 3: regenerate expected outputs + full validation

- [ ] Regenerate only the goldens of fixtures that call the three members. Diff and confirm
      the delta is limited to those call sites.
- [ ] Full suite (`cargo test --release`; see `.ai/testing-gates.md` for the golden and
      artifact gates), never just one module, before re-baselining anything.
- [ ] Re-run `/tmp/cdleak` (all six modes): `count`, `literal`, `local` and `field` should
      peak near `none`'s ~1 MB. Re-run the canvas-demo bestiary scene: steady RSS.
- [ ] Spec sync: `mfb spec unicode` (grapheme segmentation section) should say that
      `graphemesCount` and `graphemeAt` walk clusters without materializing the list.
      Re-render `mfb man strings graphemesCount` for the `errors` change.

Acceptance: full suite green, golden deltas are exactly the intended change, and the repro
and the demo are flat.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_strings_unowned_blocks.rs` (live_bytes at N and 2N,
  three RED cases plus positive pins and value checks).
- Runtime proof: `/usr/bin/time -l /tmp/cdleak/build/cdleak.out count` peaks near 1 MB
  instead of 3,277,897,728 B, and the canvas-demo bestiary scene holds steady RSS.
- Doc sync: `src/docs/spec/unicode/01_tables-and-algorithms.md` (grapheme section) and the
  `graphemesCount` descriptor's `errors`.
- Full suite: `cargo test --release`, plus the golden gates in `.ai/testing-gates.md`.

## Open Decisions

- `graphemesCount`'s `ErrOutOfMemory`: **drop it** (the lowering cannot raise it once it
  stops allocating, per bug-698) vs. keep it as a reserved contract. Dropping it changes
  the rendered man page.
- `graphemeAt`: **reuse `grapheme_at_window`** (no allocation) vs. register the list as an
  interior temp (smaller diff, still O(n) alloc + scrub per call).

## Summary

The risk is small and local. Two lowerings build a whole grapheme list to read one word or
one entry out of it, and drop the list. A third returns a fresh `String` without the
freshness mark. The `graphemesCount` fix only reuses the count pass that already exists. The
care goes into `graphemeAt`'s error path at an expression site and into keeping the golden
delta limited to the three members' call sites. Segmentation, `graphemes`, the in-place
window arm, and every other `strings` member (all measured flat) are left alone.

## Resolution (2026-10-10)

Both Open Decisions went the way the Fix Design proposed.

**A, `graphemesCount`.** The count pass is its own emitter,
`gen_graphemes.rs:emit_grapheme_count_pass` (registers in `GraphemeCountRegs`,
labels in `GraphemeCountLabels`). `lower_strings_graphemes` calls it with the same
registers and mints the same labels in the same order as before, so
`strings::graphemes` emits what it did: `mfb build -ncode` of a program that calls
`len(strings::graphemes(io::readLine()))` is byte-identical between the base
compiler and this branch (`cmp`). `func_graphemes_count.rs:lower` runs the pass and
returns the count. It allocates nothing, so `ErrOutOfMemory` is gone from its
descriptor and from its man page (`mfb man strings graphemesCount` renders no
Errors section and still says "the call never fails").

**A, `graphemeAt` (copying).** `func_grapheme_at.rs:lower` is
`grapheme_at_window` + `emit_materialize_string_from_bytes`: the walk the in-place
arm already used, then the copy of the one span. Its only allocation is its
result. The walk raises the same `ErrIndexOutOfRange` under the same member name
(`strings.graphemeAt`); the value test checks index `-1` and index `count` on
seven strings. The interior-temp fallback was not needed.

**B, `fromScalars`.** `gen_scalar_seam.rs:lower_from_scalars` marks its result
fresh. Every path through it allocates the block it returns.

**C (found by the Phase 1 audit), `normalizeNfc`.** The report's sweep used an
ASCII string, which takes `normalizeNfc`'s copy-only fast path. On a non-ASCII
string the copying lowering allocates a scalar array (`scalarCount × 8` bytes,
`temp_slot`) to decompose and compose in, and never freed it: 8 bytes per scalar
per call (measured on the base compiler: `a😀b` 32 B, the 59-byte probe string
400 B). `func_normalize_nfc.rs:lower_out` frees it after the encode pass, and on
the two exits that raise `ErrOutOfMemory` between its allocation and then. The
in-place arm (`StringOut::Scratch`) keeps its array in the function's self-update
scratch and is unchanged.

### Phase 1 audit verdict

- `grep -rn lower_strings_graphemes src`: one caller left, `func_graphemes.rs`,
  which returns the list as its own result.
- Dynamic sweep (`/tmp/k/sw`, `mfb build --debug`, `live_bytes` at N = 100 and
  200, per-call growth) over 23 `strings` members with a non-ASCII argument
  (`upper`, `lower`, `caseFold`, `normalizeNfc`, `displayWidth`, `graphemesCount`,
  `graphemeAt`, `graphemes`, `toScalars`, `fromScalars`, `padLeftToWidth`,
  `padRightToWidth`, `padLeft`, `trim`, `trimChars`, `left`, `right`, `mid`,
  `replace`, `split`, `find`, `count`, `repeat`, `toBytes`, `stripPrefix`) and 14
  `collections` expressions (`sort`, `distinct`, `keys`, `values`, `filter`,
  `reduce`, `take`, `chunks`, `zip`, `flatten`, `join`, `split`, …): every one is
  0 B per call on this branch. On the base compiler the only non-zero rows are the
  three this report names and `normalizeNfc` (sub-issue C).
- A static per-site audit of every lowering outside `strings` that builds a block
  and returns a scalar was not done; the sweep above is the evidence for the
  members it lists and for no others.

### Runtime proof

`mfb build -q --debug`, arena 0, per-call growth of `live_bytes` from N = 100 to
N = 200 for the 59-byte probe string (`The quick brown fox jumps over the lazy dog
a😀b👍🏽c`, 49 clusters):

| expression | base | fixed |
|---|---|---|
| `strings::graphemesCount(s)` | 2,064 B | 0 B, and no allocation at all |
| `len(strings::graphemeAt(s, 47))` | 2,064 B | 0 B, one allocation (the result) |
| `len(strings::fromScalars(strings::toScalars(s)))` | 80 B | 0 B |
| `len(strings::normalizeNfc(s))` | 400 B | 0 B |
| `len(strings::graphemes(s))`, `displayWidth`, `mid`, `toScalars` | 0 B | 0 B |

The test asserts `alloc_calls == free_calls` and `live_bytes 0` at both counts, and
compares `alloc_calls` per call against a `len(s)` baseline: equal for
`graphemesCount`, one more for `graphemeAt`, two more for `normalizeNfc`.
`rt_scalar_seam_alloc` still passes (`fromScalars` allocates once per call).

`strings` man examples: `MFB=<worktree mfb> scripts/man-run-examples.sh strings
--run` reports `examples: 89 built: 89 ran: 89 failed: 0`.

Not re-measured here: `/tmp/cdleak` and the canvas-demo bestiary scene (the
programs are not in the tree).

## Golden impact

`.ncode`/`.ncodesum` move for the fixtures that call `strings::graphemesCount`, a
copying `strings::graphemeAt`, `strings::fromScalars` whose result is unbound, or a
non-constant `strings::normalizeNfc`. `strings::graphemes` call sites are
byte-identical. The rendered `mfb man strings graphemesCount` loses its Errors
section. The fixtures the artifact gate reports are listed in the commit message.
