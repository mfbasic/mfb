# plan-171-H: Shared function exits (AArch64)

Last updated: 2026-10-21
Effort: large (3h–1d)
Depends on: plan-171-G (letter order; the row runs last in plan-171-A's
machine hook, after A's epilogue pairing has fixed the exit sequences)

`finalize_frame` writes a full epilogue (one restore per callee-saved
register, then `add sp`) in front of **every** `ret`, and a function has many.
`sorts` has 11791 `ret`s in 1133 functions (`/tmp/p171h/tool/exits.py`, §2).
Most of them are error exits: a fallible operation that fails ends in
`bl _mfb_rt_park_error; <epilogue>; ret`, and 6481 of the `sorts` exits have
exactly that shape (`/tmp/p171h/tool/tails3.py`). Inside one function every
epilogue is the same text. So all but one copy can become a single `b` to a
shared copy, and repeated error exits can also share their `bl`.

Outcome: on `macos-aarch64` and `linux-aarch64` at `-O1` and above, including
`--debug` builds, each function keeps one copy of each distinct exit sequence.
Every other exit with the same sequence is one `b` to it, and every other
error exit calling the same helper is one `b` to a shared `bl`. `-O0`,
x86-64 and riscv64 output are byte-identical to the start of this sub-plan,
and every program's output is unchanged.

References: plan-171-A's list, plus
- `src/codegen/engine/util/vreg_frame.rs:finalize_frame`, `restore_callee_saved`
  (the per-`ret` epilogue), plan-171-A §4.3 (paired restores, post-index fold).
- `src/codegen/error/emission/park_error_helper.rs` (the shared park; why its
  callers end in `bl; epilogue; ret`).
- `src/debugger/backtrace/mod.rs:state_at` (`Effect::Leave` handling),
  `src/debugger/arm64/mod.rs` (`Effect::of` for branches),
  `src/debugger/breakpoints/errors.rs` (`breakerror`'s `back` address).
- `planning/optimizations.md` rows "Tail merging (cross jumping)",
  "Prologue / epilogue optimization".
- `mfb spec tooling debugger`; `mfb man debug`.

## Prerequisites

See plan-171-A §Prerequisites. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-171-G complete | `ls planning/plan-171-G-* 2>/dev/null` → no match | NOT MET |

Everything below assumes A–G have landed. In particular
`optimizer::opt2::machine::finish` exists and runs after A's epilogue pairing.

## 1. Goal

- One Level-1 machine row, "Shared function exits", in
  `src/optimizer/opt2/shared_exit.rs`, called **last** in `machine::finish`,
  gated `level_enabled(1)` and `active_backend().supports_shared_exits()`
  (AArch64 `true`, x86-64 and riscv64 `false`).
- Per function, exits are grouped by their exact instruction text
  (`source` ignored). In each group of two or more, one canonical copy stays,
  every other copy becomes `b` to it, and the most frequent helper call
  directly in front of the exits is shared too (§4).

### Non-goals

- No change to any value, trap, error code or error origin. The `ErrorLoc` a
  raise reports is built before the shared `bl` and passed in `x3`, so it still
  names the raising site.
- No change to `-O0` output, or to x86-64 and riscv64 (Win64 unwind data is
  plan-169's subject; riscv64 is out of the owner's scope for this letter).
- No merging of anything but an exit sequence (the run of epilogue
  restores, `add sp` and `ret`) plus at most one directly preceding `bl`.
  General cross-jumping of body code stays the open L3 catalog row.
- No tail calls (owner decision 2026-10-21; plan-171-G §3 records the census).

## 2. Current State

- `finalize_frame` emits, before every `ret`, one `restore_callee_saved` load
  per register in reverse, then `add sp, total` (plan-171-A §2.1, point 5).
  After A, that becomes paired `ldp`s, with the `add sp` folded into a
  post-index `ldp` on small frames. Either way the epilogue proper is the
  same text at every `ret` of one function. Some exits also reload result
  registers from the stack (`ldr x0..x3, [sp, #k]`) directly before it; those
  reloads vary per exit (§2 Verified properties).
- Error exits: `park_error_helper.rs` makes the park one shared function
  (`_mfb_rt_park_error`) that "returns to the function that raised it". Every
  raising site therefore ends `bl _mfb_rt_park_error`, then that function's
  epilogue and `ret`.
- Labels are function-scoped. Every `-ncode` function opens with `label entry`
  (`grep -m2 '"name": "entry"' sorts.ncode` → two functions). `rg -n
  'shared_exit' src` → 0, so the label prefix `shared_exit_` is free.
- The debugger's frame walk scans the function linearly up to the pc and
  resets to the body state only on `Effect::Leave` that leaves the function
  (`ret`, or a branch outside `[start, end)`): `state_at`, `Effect::Leave`
  arm. An in-function `b` is not a leave, and a merged exit site runs no
  epilogue instruction before its `b`. So the scan's state at every pc
  equals the state the code is really in. A unit test proves it (Phase 2).
- `breakerror` reads the park's return address (`Raise::back`) to run to the
  raiser. `mfb dbg`'s own tests build `--debug -O0`
  (`tests/cli/cli_dbg/session_launch_and_list.rs:build_debug`), so this row
  never reaches them.
- No AArch64 unwind tables are emitted: `__unwind_info` is a placeholder
  section name in `src/os/macos/link/commands.rs`, and no `.eh_frame` is
  written (`rg -li 'eh_frame|compact_unwind|__unwind_info|debug_frame|UNWIND_INFO' src`
  → only `src/os/macos/link/{mod,commands}.rs`, the placeholder). There is
  nothing to keep in step with moved epilogues.

### Measured populations

Corpus, compiler and method as plan-171-G §2 (`macos-aarch64`, `-O1`, four
example programs, scratch builds). Prototype in Phase 1.

| | brogue | dungeon | life | sorts |
|---|---|---|---|---|
| functions | 243 | 1038 | 202 | 1133 |
| `ret`s | 1885 | 10945 | 1596 | 11791 |
| exits `bl _mfb_rt_park_error; restores; ret` | 815 | 5593 | 599 | 6481 |
| instructions saved, today's epilogues | 10916 (1.99%) | 66875 (6.15%) | 7607 (5.15%) | 70919 (6.83%) |
| instructions saved, after A's pairing (modelled) | 6428 (1.17%) | 40179 (3.69%) | 4650 (3.15%) | 42717 (4.11%) |

The first savings row is measured on today's dumps
(`/tmp/p171h/tool/hmerge.py`). The second row replays the same dumps with
plan-171-A §4.3's pairing rules applied to each epilogue: adjacent 8-byte
restores in one `ldp`, `ldr q` restores paired, and `add sp` folded when the
frame is ≤ 504 bytes with the save area at offset 0
(`/tmp/p171h/tool/hmerge_postA.py`). It is a model of A's output, not a
measurement of it. Phase 1 measures the real number on the post-G tree.

Variants weighed while planning (same corpus):
- Merging only the error exits: 37828 in `sorts` (`tails3.py`), a subset of
  the row above.
- Converting the shared exits to tail calls under the safe frame-escape rule:
  148 more in `sorts` (`tailfn.py`). Rejected by the owner.

### Verified properties

- *Most exits in a function share one text.* `finalize_frame` builds one
  restore list and emits it before each `ret` (read: `finalize_frame`,
  `restore_callee_saved`). The share of `ret`s in their function's largest
  group is 100% / 98.56% / 100% / 99.39% (brogue / dungeon / life / sorts).
  19 functions in `sorts` and 38 in `dungeon` have two or more groups, because
  §4 step 1's run also takes in result reloads (`ldr x0..x3, [sp, #k]`) that
  sit directly before the epilogue and differ per exit. Inspected in `sorts`:
  one group ends `ldr x1/x0/x2/x3; ldr lr; add sp; ret`, another
  `ldr lr; add sp; ret`. Grouping by exact text keeps that correct; each text
  is its own group (`/tmp/p171h/tool/groups.py`).
- *Every committed aarch64 `.ncode` golden has functions with ≥ 2 exits*
  (5–10 per golden, `groups.py` over `find tests -name '*.ncode'`), so all
  seven are expected to change.
- *An exit sequence holds no pc-relative instruction.* Restores are
  `sp`-based loads, `add sp`, `ret`; the shared `bl` is resolved by symbol. A
  copy behaves the same at any address.
- *Origin of a raised error is unaffected.* The park reads code, message and
  `ErrorLoc` from registers set before the `bl`
  (`builder_error_emission.rs:emit_park_error_block_from_registers`). The
  shared `bl` receives each site's own registers.

## 3. Design Overview

One post-frame rewrite per function, after every other machine row, so it sees
the final exit text (A's pairs, B's branches). Nothing upstream changes: no
builder, no frame code, no allocator.

**Correctness risk** is in two places, both unit-tested:
- the canonical copy must stay reachable by fall-through from its own block;
- a call is shared only when the `bl` is directly adjacent to the exit (no label
  between), so no branch lands between the `bl` and the epilogue.

**Debug-experience change (owner decision):** at `-O1`+ a stop inside a
shared exit reports the canonical copy's source line. This covers stepping
over a `RETURN` and `breakerror` running to the raiser. The printed error
origin stays exact. `--debug -O0` is unchanged; the man and spec pages say so.

**Gate class:** behavior-preserving, golden-changing. Expected diffs: every
aarch64 golden with a function that has two or more identical exits, which
is all seven committed `.ncode` goldens (§2) and the aarch64 `.ncodesum`
family. A
`linux-x86_64`, `linux-riscv64` or Windows golden diff is a bug to localize.

**Rejected alternatives**
- *One epilogue block built by `finalize_frame` itself* (emit the epilogue once
  and rewrite each `ret` to `b`). That would be Level 0, changing `-O0`
  output and the debugger's `-O0` contract. The owner chose `-O1`+.
- *Tail calls through the shared exit:* 148 instructions in `sorts`
  (§2), against a frame-escape proof whose failure corrupts memory.
- *Error exits only:* a subset (§2); the owner chose all identical exits.
- *x86-64 and riscv64:* owner scope. On Win64 the exit layout is also what
  plan-169's unwind data describes.

## 4. Detailed Design

`shared_exit::share_exits(instructions)`, in each function:

1. **Find exits.** For each `ret` at index `r`, the exit starts at the
   earliest `s ≤ r` such that every instruction in `s..r` is an epilogue
   instruction: an `ldr*`/`ldp*` with base `sp` (any index mode), or
   `add_sp`. A label stops the run.
2. **Group** exits by the text of `s..=r` (op and fields, `source` ignored).
   Skip groups of one member, and skip groups whose sequence is `ret` alone
   (a `b` saves nothing there).
3. **Call subgroup.** Among a group's members, take those whose
   instruction `s−1` is a `bl` (directly adjacent). Pick the target `X` with the
   most such members; ties go to the lexicographically smallest symbol. It is
   a subgroup only with ≥ 2 members.
4. **Canonical copy.** If there is a call subgroup, it is the last member (by
   index) whose `s−1` is `bl X`. Otherwise it is the last member. Insert
   `label shared_exit_<n>` at its `s`. With a call subgroup, also insert
   `label shared_exit_call_<n>` immediately before its `bl X`. `n` counts
   groups within the function, so labels stay unique.
5. **Rewrite other members.** A call-subgroup member: replace `bl X` through
   `ret` with `b shared_exit_call_<n>`. Any other member: replace `s..=r` with
   `b shared_exit_<n>`. The `b` takes the replaced first instruction's
   `source`.
6. Count rewritten members into `stats::SHARED_EXITS`.

The canonical copy is never removed and keeps its own fall-through
predecessor. Every rewritten member ended in `ret`, so no instruction after it
was its successor. The new labels are reached only by the new branches. `b` has
a ±128 MiB range, so no relaxation is involved.

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — census pattern and post-G baseline

- [ ] Add an `exits` pattern to `tools/ncode-census/census.py`. Per function it
      counts `ret`s, groups and instructions saved by §4. Prototype that
      produced §2's measured row:
      ```python
      import json, sys, collections
      def funcs(p):
          fs = []; cur = None
          for l in open(p):
              s = l.strip()
              if s.startswith('"instructions"'): cur = []; fs.append(cur)
              elif s.startswith('{ "op"') and cur is not None: cur.append(json.loads(s.rstrip(',')))
          return fs
      def isrest(x): return ((x['op'].startswith('ldr') or x['op'].startswith('ldp')) and x.get('base') == 'sp') or x['op'] == 'add_sp'
      def key(xs): return json.dumps([{k: v for k, v in x.items() if k != 'source'} for x in xs])
      for p in sys.argv[1:]:
          saved = n = 0
          for I in funcs(p):
              n += len(I); exits = []
              for i, x in enumerate(I):
                  if x['op'] == 'ret':
                      j = i
                      while j > 0 and isrest(I[j-1]): j -= 1
                      exits.append((j, i))
              groups = collections.defaultdict(list)
              for j, i in exits: groups[key(I[j:i+1])].append((j, i))
              for k, g in groups.items():
                  L = len(json.loads(k))
                  if len(g) < 2: continue
                  saved += (len(g) - 1) * (L - 1)
                  calls = collections.Counter(I[j-1]['target'] for j, i in g if j > 0 and I[j-1]['op'] == 'bl')
                  if calls:
                      t, c = calls.most_common(1)[0]
                      if c >= 2: saved += c - 1
          print(p.split('/')[-1], 'instrs', n, 'saved', saved, f'{100*saved/n:.2f}%')
      ```
- [ ] Run it for `macos-aarch64` on the post-G tree. Record the measured
      "after A" row in §2, replacing the modelled one, and note the model's
      error.

Acceptance: §2's second savings row is measured.
  Check: `grep -c 'modelled' planning/plan-171-H-*.md` → 0 (est. 20 min — corpus build).
Commit: —

### Phase 2 — the row (Level 1)

- [ ] `src/optimizer/opt2/shared_exit.rs`: `share_exits` (§4).
      `Backend::supports_shared_exits` with its three impls. Called last in
      `optimizer::opt2::machine::finish`.
- [ ] `src/optimizer/opt2/shared_exit/tests/mod.rs`:
      - two and five identical exits;
      - a call subgroup of two targets with unequal counts;
      - a tie between call targets;
      - a label between `bl` and epilogue (call not shared, epilogue shared);
      - a label inside the restore run;
      - a group of `ret` alone (untouched);
      - two distinct exit texts in one function (two groups);
      - canonical copy last vs. first in the function;
      - `-O0` untouched.
- [ ] `src/debugger/backtrace/tests.rs`: `state_at` on an assembled function
      with merged exits. The pc is (a) after a `b shared_exit_0` in a later
      block, (b) inside the shared epilogue between two `ldp`s, and (c) at the
      shared `bl`. Each yields the CFA and `lr` slot of the unmerged layout.
- [ ] Catalog row "Shared function exits" (level 1, stage `machine`) and
      `stats::SHARED_EXITS`. `planning/optimizations.md`: "Tail merging (cross
      jumping)" `~` with the landing note (function exits only, Level 1,
      AArch64; owner decision 2026-10-21), and a line under "Prologue /
      epilogue optimization".
- [ ] rt fixtures: `tests/rt-behavior/functions/many-exits` (valid): a function
      with eight `RETURN`s on different paths and one with six fallible
      operations whose errors are caught and printed (code and origin line per
      site). `tests/rt-error/functions/many-exits-uncaught`: an uncaught raise
      from the fourth of six sites, whose expected stderr names that site's
      line. These prove the origin survives sharing.
- [ ] `tests/runtime/rt_codegen_forms.rs::shared_exits`: in the fixture's
      aarch64 `-O1` dump, the eight-`RETURN` function has one `ret` per
      distinct exit text and `b shared_exit_` branches. At `-O0` it has eight
      `ret`s. The `linux-x86_64`/`linux-riscv64` dumps contain no
      `shared_exit_` label.
- [ ] `rt_regalloc_levels_agree`: add both fixtures.
- [ ] Golden re-baseline (aarch64 only), following plan-171-A Phase 3's
      procedure. Proof is the `-ncode` diff of ONE fixture per aarch64 target
      showing only exit rewrites (`census.py --diff`, `exits` pattern).
- [ ] Runtime: `bash scripts/test-accept.sh target/release/mfb /tmp/171h-accept`
      on macOS; `bash scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64`.
- [ ] Benchmark (macOS host): `./benchmark/language/run.sh 10` vs.
      `benchmark/language/baseline/mfb-O1.log` with `rank.py`; record rows and
      binary sizes of the four corpus programs (`ls -l`) before/after in
      Corrections. No baseline regeneration.

Acceptance: identical exits are shared at `-O1` on aarch64 only; outputs and
error origins are unchanged.
  Check: `cargo test --bin mfb opt2::shared_exit debugger::backtrace` → pass (est. 3 min);
  `cargo test --release --test rt_codegen_forms shared_exits` → pass (est. 2 min);
  `linux-runtime-proof.sh … 2223 linux-aarch64` → 0 mismatches (est. 25 min — the only check that executes Linux aarch64 code).
Commit: —

### Phase 3 — spec and docs

- [ ] `mfb spec tooling debugger` and `src/docs/man/debug/` (stepping and
      `breakerror` pages). At `-O1` and above, a stop in a shared exit, including
      `breakerror`'s run to the raiser, reports the shared copy's line. The
      printed error origin is exact. `--debug -O0` keeps every line exact. Run
      `.ai/man-content.md`'s rules for the man page; render both.
- [ ] `mfb spec architecture aarch64-instruction-set` (or the frame topic that
      describes epilogues): exits may be shared at `-O1`+.
- [ ] `.ai/codegen-invariants.md`: "after the machine hook, a function may have
      fewer `ret`s than exits; code that counts or patches epilogues must look
      for `shared_exit_` labels".

Acceptance: the pages render with the new text.
  Check: `cargo test --bin mfb spec` → pass (est. 2 min); `target/release/mfb man debug | grep -c 'shared'` → ≥ 1.
Commit: —

## Validation Plan

- Tests: `opt2::shared_exit` units, `debugger::backtrace` unit, `rt_codegen_forms`,
  both fixtures, levels-agree.
- Coverage: every function with ≥ 2 exits passes through the row at `-O1`.
  Check that `mfb build -vv` reports a non-zero `SHARED_EXITS` count on the
  fixture before trusting a green gate.
- Runtime proof: macOS acceptance; `linux-runtime-proof.sh` on 2223.
- Doc sync: Phase 3.
- Final gate (once, after Phase 3): `cargo test --no-fail-fast > /tmp/171h.log 2>&1; echo EXIT=$?` → `EXIT=0`, `grep -c '^failures:' /tmp/171h.log` → 0 (est. 60–90 min; includes `artifact_gate_all` and `cli_dbg`).

## Decisions (owner, 2026-10-21)

1. Identical exit merging is part of plan-171, as its own letter H.
2. It covers every identical exit, not only error exits.
3. AArch64 only, `-O1` and above.
4. `--debug` builds are merged too, and the debugger docs state the line
   effect.
5. Tail calls are not in plan-171.

## Corrections

<Filled in during execution.>

## Summary

The largest single size win in plan-171: 3–4% of instructions by the post-A
model, in a rewrite that adds no instruction form and touches no builder. Its
risks are a misplaced canonical copy and a call shared across a label. Both
are unit-tested, and the error-origin fixtures prove raises still report their
own site.
