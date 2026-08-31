# bug-456: `MFB_OPT>1` acceptance sweeps can never pass — full-text `.ncode` goldens are level-variant by design, so 7 fixed mismatches mask real failures

Last updated: 2026-08-25
Effort: small (<1h)
Severity: LOW
Class: Footgun (verification harness; no miscompile)

Status: Open
Regression Test: — (Phase 1: `MFB_OPT=3 test-accept` on a clean tree must exit 0)

plan-100 gave `scripts/test-accept.sh` the `MFB_OPT` switch so the whole
1271-fixture suite can run at a chosen dial level — now the standing
verification step for every new `-O2+` optimizer row (it caught two real
miscompiles during the loop-rows landing that spot checks missed). But seven
fixtures embed **full-text `.ncode` goldens**, which pin default-level machine
code; at `MFB_OPT=3` their code legitimately differs, so every sweep reports
exactly these 7 mismatches and exits 1. A real behavioral regression hides in
the noise unless the operator eyeballs the mismatch list every time.
**The single correct behavior a fix produces: an `MFB_OPT>1` sweep compares
only level-invariant goldens (`.run`, build.log, `.ast`, `.ir` — all upstream
of or invariant to the dial) and exits 0 on a healthy tree, while the default
run keeps comparing everything.**

The 7 fixtures (from the 2026-08-25 sweeps; re-derive with the command below,
never from this list):

```
rt-behavior/collections/func_map_getor_hash_probe/…macos-aarch64.ncode
rt-behavior/collections/list-ops-codegen-rt/…macos-aarch64.ncode
rt-behavior/control-flow/control-flow-if/…macos-aarch64.ncode
syntax/app/macos-app-mode-io/…macos-aarch64.app.ncode
syntax/app/macos-app-mode-plumbing/…macos-aarch64.app.ncode
syntax/lexical/parser-hello-world/…macos-aarch64.ncode
syntax/match/control-flow-match/…macos-aarch64.ncode
```

## Unresolved measurement, added 2026-08-31 (coordinator, pre-dispatch)

**The dial-sensitive golden population is larger than the 7 this document
names, and the gap is not explained.** Resolve it before writing the fix; a fix
keyed to `.ncode` alone is either wrong or vacuous depending on the answer.

Every golden kind currently in the tree:

```
$ find tests -path '*/golden/*' -type f | sed 's/.*\.//' | sort | uniq -c | sort -rn
1305 log     811 ir      811 ast     651 run     140 ncodesum
  21 nplan    21 nir      18 nobj      7 ncode      2 mir
```

`.ncode` is **7**, matching this document. But `.nir`, `.nplan`, `.nobj`, `.mir`
and `.ncodesum` are all *native backend* dumps downstream of the `-O` dial, and
together they are **202 files across 21 fixtures** — 14 fixtures more than the 7:

```
$ find tests -path '*/golden/*' \( -name '*.nir' -o -name '*.nplan' -o -name '*.mir' \) \
    | sed 's#/golden/.*##' | sort -u | wc -l
21
```

Five of the seven `.ncode` fixtures (`control-flow-if`, `macos-app-mode-io`,
`macos-app-mode-plumbing`, `parser-hello-world`, `control-flow-match`) carry a
`.nir`/`.nplan` golden **as well**. So for those five, the reported sweep saw
`.ncode` mismatch while `.nir` did not — for the same fixture, same build. That
is either a real and interesting fact about where the dial's effect first
appears, or a sign the sweep did not compare those kinds at all.

Three possibilities, and the fix differs under each:

1. **The other native kinds genuinely do not drift at `-O3`** (the dial's effect
   is confined to final code emission). Then keying the skip to `.ncode` is
   correct — but say so, with the measurement, because it is surprising.
2. **The sweep never compares them.** Then the "exactly 7 mismatches" figure is
   an artifact of what the harness looks at, this document's premise is
   incomplete, and the real fix is larger.
3. **They do drift and the count of 7 was measured narrowly** (one fixture glob,
   one target). Then the fix must cover all 202 files, not 7.

**Do not resolve this by reading — run it.** Re-derive per this document's own
instruction ("re-derive with the command below, never from this list"), and
capture the *kinds* of every mismatch, not just the count. Note the run is
expensive and contends the gate; see bug-470 for why a concurrent run can also
report phantom diffs, and re-run uncontended before trusting any list.

Whatever the answer, the fix must be keyed to a **derived predicate** ("is this
kind downstream of the dial?") rather than a hard-coded list of seven paths, or
the next fixture to gain a `.nir` golden silently reintroduces the bug this
closes.

References:

- `scripts/test-accept.sh:148-162` — the `MFB_OPT` switch (plan-100) and its
  deliberate non-echo into build.log.
- Memory note `optimizer-rows-need-giant-function-stress.md` — records
  `MFB_OPT=3 test-accept` as the mandatory behavior sweep for new rows.
- Found while landing the six Level-3 loop rows (2026-08-25): the sweep's
  useful signal (3 real bugs) sat among the 7 constants.

## Failing Reproduction

On a tree where the default-level suite is fully green:

```
bash scripts/test-accept.sh target/release/mfb /tmp/accept-out      # exit 0
MFB_OPT=3 bash scripts/test-accept.sh target/release/mfb /tmp/accept-o3
```

- Observed: `acceptance tests failed: 7 mismatch(es) (1271 test(s) ran)`,
  exit 1 — all seven are `.ncode`/`.app.ncode` full-text goldens.
- Expected: exit 0, with the level-variant comparisons skipped (and ideally a
  one-line note of how many were skipped — no silent caps).

## Root Cause

`scripts/test-accept.sh` compares every file present in a fixture's `golden/`
directory; `.ncode` full-text goldens are **drift sentinels for the default
level** (see `.ai/testing-gates.md` — byte-identity artifacts are not
behavior). `MFB_OPT` changes which machine code is correct, but the compare
set doesn't know any golden is level-dependent, so the sentinel fires on the
dial working as intended.

## Goal

- `MFB_OPT=3 test-accept` exits 0 on a tree whose default-level run exits 0,
  skipping (and counting, loudly) the `.ncode`/`.ncodesum` comparisons; a
  genuine `.run`/build.log deviation still fails the sweep.

### Non-goals (must NOT change)

- The default (`MFB_OPT` unset) run keeps comparing `.ncode` goldens — they
  are load-bearing drift sentinels there.
- Do NOT regenerate the 7 goldens at `-O3` or add per-level golden copies —
  that doubles maintenance for sentinels whose value is default-level drift.
- The `MFB_OPT=1` == default gate (explicit `-O1` byte-identity proof) must
  keep comparing everything — the skip applies to `MFB_OPT>1` only (or is
  keyed to "level differs from default").

## Blast Radius

- `scripts/test-accept.sh` compare loop — fixed by this bug (skip
  `*.ncode`/`*.ncodesum` when `MFB_OPT` > 1, count skips into the summary
  line).
- `scripts/artifact-gate.sh` — unaffected: it has no `MFB_OPT` switch and is
  definitionally the default-level byte gate.
- Operators' sweep logs/memory notes — update the memory note's expectation
  ("expect only the full-text .ncode fixtures to mismatch") to "expect exit
  0" once fixed.

## Fix Design

In the harness's golden-compare loop: when `opt_arg` is set to a non-default
level, skip files matching `*.ncode`/`*.ncodesum`, tally them, and append
`(N level-variant golden(s) skipped at -O$MFB_OPT)` to the summary — an
explicit count, not a silent cap. Risk is near zero; the subtlety is applying
the skip by *golden comparison*, not by fixture (the same fixtures' `.run`
and build.log must still be compared — they are the whole point of the
sweep).

## Phases

### Phase 1 — failing reproduction pinned

- [ ] Record the exit-1/7-mismatch behavior on a green tree (this file);
      confirm all 7 are `.ncode`-family and their `.run`/build.log matched.

Acceptance: reproduction documented as above.
Commit: —

### Phase 2 — the fix

- [ ] Harness skip + skip-count in the summary for `MFB_OPT>1`.

Acceptance: `MFB_OPT=3` sweep exits 0 on a green tree with
`7 level-variant golden(s) skipped`; seeding a deliberate `.run` corruption
still fails it; default run unchanged (byte-identical harness behavior with
`MFB_OPT` unset, proven by an ordinary full run).
Commit: —

### Phase 3 — full validation

- [ ] Default `test-accept.sh` full run (harness change must be inert there).
- [ ] `MFB_OPT=1` run — still compares everything (the plan-100 gate intact).
- [ ] `MFB_OPT=3` run — exit 0.

Acceptance: all three runs behave per their contracts.
Commit: —

## Validation Plan

- Regression: the three-run matrix above.
- Runtime proof: a deliberate behavioral corruption is still caught at
  `MFB_OPT=3`.
- Doc sync: `optimizer-rows-need-giant-function-stress.md` memory note;
  `.ai/testing-gates.md` gains a line on the level-variant skip.
- Full suite: default `test-accept.sh` + `cargo test --no-fail-fast`.

## Open Decisions

- Skip keyed on `MFB_OPT != ""` vs `MFB_OPT > 1`: recommended `> 1`, so the
  `MFB_OPT=1` byte-identity gate keeps its full compare set.

## Summary

A small harness ergonomics fix that turns the new standing `-O3` sweep into a
clean pass/fail signal; the only care point is keeping the `MFB_OPT=1` gate
and the default run byte-for-byte unchanged.
