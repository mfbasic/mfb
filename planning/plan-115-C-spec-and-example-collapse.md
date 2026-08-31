# plan-115-C: Spec consolidation and the `network-server` collapse

Last updated: 2026-08-31
Effort: medium (1h–2h)
Depends on: plan-115-B

Letters A and B changed the thread-entry rules and deleted `IMPORT self`, each
syncing the spec lines it directly invalidated. This letter finishes the job: it
rewrites the *narrative* the old rule was built around
(`src/docs/spec/threading/01_source-model.md`), states the namespace model that
A's Phase 1 verified, and cashes in the payoff by collapsing
`examples/network-server` from two projects back to one.

After this letter: the spec describes thread entries in terms of `ISOLATED`
alone, states plainly that a worker gets a fresh instance of the **declaring
project's** namespace and *how that is enforced*, and `examples/network-server`
builds with a single `mfb build` — no package, no prepare step.

References:

- `planning/plan-115-A-unified-thread-entry.md` — **the Prerequisites table for
  the whole plan-115 feature lives there.** Re-run both commands before starting.
- `planning/plan-115-B-remove-import-self.md` § Corrections — read before
  starting; B may have left a dangling reference in
  `src/docs/spec/threading/01_source-model.md`, which this letter owns.
- `.ai/specifications.md` — the embedded spec's conventions; read before editing
  `src/docs/spec/**`.
- `.ai/man-content.md` — if any `mfb man` prose mentions thread entries, the
  memory-vocabulary ban applies to the replacement text.
- `scripts/build-examples.sh` — the cross-target example gate this letter
  simplifies.

## Prerequisites

See `plan-115-A-unified-thread-entry.md` § Prerequisites — bug-480 and bug-482.
**Re-run both commands now.**

| Must be true | Command | Status |
|---|---|---|
| plan-115-B is complete and archived | `ls planning/plan-115-B-*.md` → no matches (moved to `planning/completed/`) | NOT MET |

If B is not complete, this letter cannot start, full stop — the example collapse
depends on the executable being able to host its own `ISOLATED FUNC` entries (A)
and on `connworker`'s helpers no longer needing a package to live in (B's
conversion establishes the spelling).

## 1. Goal

Two checkable outcomes:

1. `mfb build examples/network-server` succeeds from a clean checkout with **no
   prior package build and no `packages/` directory**, and the resulting binary
   passes the same `--tcp --thread` / `--tls --thread` runs the README documents.
2. `mfb spec language threads` and `mfb spec threading source-model` describe the
   entry rule as "an `ISOLATED FUNC`" with no mention of import-reachability,
   `EXPORT`-ness, or `self`, and state the namespace model together with the
   mechanism that enforces it.

### Non-goals (explicit constraints)

- **No compiler change.** This letter is docs plus one example. If a spec
  sentence cannot be made true by editing prose, that is a defect in A or B —
  file it, do not patch the compiler here.
- **No behavior change to the example.** The collapsed `network-server` must
  serve the same protocol, print the same lines, and accept the same flags. It is
  a restructure, not a rewrite.
- **No `mfb man` page rewrite** beyond removing statements the new rules
  falsify.
- **`examples/browser` keeps its package split** — it uses one for real reasons
  (a genuinely separate app unit), not because threads forced it.

## 2. Current State

### The example, today

`examples/network-server` is two projects because of the rule A deleted. Its
`worker/src/lib.mfb` says so in a header comment (lines 4-10), naming
`IMPORT_SELF_IN_EXECUTABLE` explicitly.

| What | Measure | Command |
|---|---|---|
| Executable source | 553 lines | `wc -l examples/network-server/src/main.mfb` |
| Worker package source | 236 lines | `wc -l examples/network-server/worker/src/lib.mfb` |
| README | 104 lines | `wc -l examples/network-server/README.md` |
| `connworker::` references in `main.mfb` | 19, across 8 distinct members | `grep -c "connworker::" examples/network-server/src/main.mfb` → 19 |
| Package exports | 8 (6 plain `FUNC`, 2 `EXPORT ISOLATED FUNC`) | `grep -n "^EXPORT" examples/network-server/worker/src/lib.mfb` |

The 8 exported members are `tickNanos`, `napMs`, `minHandshakeMs`,
`udpIdleNanos`, `counterText`, `hasBye` (wire helpers, shared by the
single-threaded path and the workers) and `serveTcpConnections`,
`serveTlsConnections` (the two `ISOLATED` entries).

`scripts/build-examples.sh:79-89` carries a `prepare_network_server()` that
builds the package and installs the `.mfp` before the executable, and
`examples/.gitignore` has a `network-server/worker/*.mfp` rule. Both exist only
to serve the split.

### The spec files still carrying the old model

| File | What needs rewriting |
|---|---|
| `src/docs/spec/threading/01_source-model.md` | The narrative built around import-reached entries; B may have left a dangling `self` reference |
| `src/docs/spec/language/16_threads.md:31` | "each started thread receives its own fresh instance of the entry function's package" — describes a per-package partitioning that does not exist |
| `src/docs/spec/language/13_modules-and-packages.md:148` | Same claim, restated for isolated functions |

### Verified properties

| Claim | How verified |
|---|---|
| The worker's global region is whole-program, not per-package | `src/codegen/engine/builder/mod.rs:1042` — `globals_base = module.globals.len() + package_global_count`; the worker arena is sized from that one number (`src/codegen/runtime/thread/runtime_helpers.rs:608`) |
| Initializers re-run per worker | `src/codegen/runtime/thread/runtime_helpers.rs:1097` — the trampoline calls `arena_init.in_run_order()` before the entry |
| The per-module namespace boundary is enforced by scoping, not partitioning | Established by plan-115-A Phase 1. **Read A's § Corrections before writing the spec sentence** — if Phase 1 falsified the premise, this letter's §3 spec text is wrong as drafted |

## 3. Design Overview

Two independent pieces that do not interact: the spec rewrite and the example
collapse. Either can land first; the example is scheduled last because it is the
one with a runtime gate.

**The spec's central correction.** The current text claims a worker gets "a fresh
instance of the entry function's package". That describes a partitioning the
implementation does not have — every global in the linked program is
re-initialized per worker. The observable behavior matches the claim only because
**name resolution** prevents a worker from naming a global outside its declaring
project. The rewrite must say both halves: the guarantee *and* that scoping is
what provides it. Stating only the guarantee leaves the next person to rediscover
the gap; stating only the mechanism loses the contract.

This matters beyond tidiness: anything that later observes globals non-lexically
— reflection, a debug dump, a cross-module registry — silently breaks the model.
The spec is where that is recorded.

**Where risk concentrates.** The example collapse, because `serveTcpConnections`
and `serveTlsConnections` are 100+ lines of live protocol code being moved
across a project boundary and re-qualified. It is scheduled last and gated on a
real run, not a build.

**Byte-identity is not a gate here** and nothing is expected to diff in
`.ncode`/`.ncodesum` — this letter touches no compiler code. `examples/` is not
in the artifact-gate population; the example's proof is that it runs.

### Rejected alternatives

- **Leave the example split.** It would be the only remaining artifact asserting
  a rule that no longer exists, and its header comment names a deleted
  diagnostic. Keeping it means shipping documentation that is false.
- **Collapse the example but keep `connworker` as a source-package dependency.**
  Possible once bug-480 lands (prerequisite), but it preserves a two-project
  build for no benefit now that the executable can host its own entries.

## Compatibility / Format Impact

- `examples/network-server/worker/` is deleted; `examples/network-server/packages/`
  is deleted. Anyone following the old README's two-step build gets a path that
  no longer exists — the README is rewritten in the same commit.
- `examples/.gitignore` loses its `network-server/worker/*.mfp` rule.
- `scripts/build-examples.sh` loses `prepare_network_server()`.
- No compiler-observable contract changes.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` **in the same
> commit as the work it describes**. Use `- [~]` for partial with one line on
> what remains. Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`.
> Fill each `Commit:` the moment it lands. **An unticked box means NOT DONE.**

### Phase 1 — Rewrite the threading spec narrative

Docs only, no code. Independently landable.

- [ ] Read `plan-115-A` § Corrections first. If A's Phase 1 falsified the
      namespace premise, **stop and re-scope this phase** — the sentences below
      are drafted against the premise holding.
- [ ] `src/docs/spec/language/16_threads.md:31` — replace "its own fresh instance
      of the entry function's package" with the declaring-project formulation,
      and add the mechanism: the worker's arena carries a fresh, re-initialized
      copy of the program's writable globals, and a worker can only *name* the
      globals of the project that declares its entry (plus that project's
      imports), which is what makes the instance per-project.
- [ ] `src/docs/spec/language/13_modules-and-packages.md:148` — same correction
      for the isolated-function paragraph.
- [ ] `src/docs/spec/threading/01_source-model.md` — rewrite the narrative around
      `ISOLATED` as the sole entry marker. Resolve any dangling `self` reference
      B left behind.
- [ ] Add an explicit note that initializer side effects re-run once per
      `thread::start` — newly visible now that an executable's top level (where
      startup-flavored bindings live) can host entries.
- [ ] `grep -rn "imported package" src/docs/spec/` — sweep for surviving
      statements of the deleted rule.
- [ ] `grep -rn "ISOLATED" src/docs/spec/` — confirm every remaining statement
      matches the implemented rule after A.
- [ ] Check `mfb man thread start` and `mfb man thread` prose (rendered from
      `src/codegen/builtins/thread/func_start.rs` and `mod.rs`, **not** from
      Markdown) for statements the new rules falsify. Verify by rendering, and
      hold any replacement text to `.ai/man-content.md`'s memory-vocabulary ban.

Acceptance: `mfb spec language threads`, `mfb spec language modules-and-packages`
and `mfb spec threading source-model` render with no mention of
import-reachability, `EXPORT`-ness or `self` in the entry rules, and state both
the namespace guarantee and its enforcing mechanism. `mfb man thread start`
renders consistent with them.
Commit: —

### Phase 2 — Collapse `examples/network-server` (largest blast radius)

The payoff, and the one piece with a runtime gate.

- [ ] Move all 236 lines of `examples/network-server/worker/src/lib.mfb` into the
      executable — either appended to `src/main.mfb` or as a sibling
      `src/wire.mfb` (prefer the sibling; 553 + 236 in one file is worse for a
      reader). Change every `EXPORT` to `PUBLIC` — `EXPORT` is illegal in an
      executable (`EXPORT_IN_EXECUTABLE`) and this plan deliberately did not
      change that.
- [ ] Rewrite the moved file's header comment (lines 4-10). It currently explains
      the two-project split by naming `IMPORT_SELF_IN_EXECUTABLE`, a diagnostic
      that no longer exists. Replace it with what the file now is: the shared wire
      format plus the two worker entries.
- [ ] `examples/network-server/src/main.mfb` — drop `IMPORT connworker` (line 41)
      and strip the `connworker::` prefix from all 19 references across the 8
      members.
- [ ] Delete `examples/network-server/worker/` and
      `examples/network-server/packages/`.
- [ ] `examples/.gitignore` — remove the `network-server/worker/*.mfp` rule.
      Leave `browser/*/*.mfp` alone.
- [ ] `scripts/build-examples.sh:75-89` — delete `prepare_network_server()` and
      its call site, plus the comment block above it explaining the thread-entry
      rule. Leave `prepare_browser` untouched.
- [ ] `examples/network-server/README.md` — rewrite the "two projects" section
      (lines 8-16) and the rationale at lines 29-31 as a single `mfb build`.
      Keep the protocol/flags documentation intact.
- [ ] Verify `examples/network-client` still interoperates unchanged — it is the
      matching client and this letter must not touch its wire format.

Acceptance: from a clean checkout with no `packages/` directory,
`mfb build examples/network-server` succeeds in one command; the binary run with
`--tcp --thread` and with `--tls --thread` serves a session end-to-end against
`examples/network-client`, producing the same output as before the collapse.
`scripts/build-examples.sh` passes for every cross-target it covers.
Commit: —

## Validation Plan

- **Tests:** none added — this letter has no compiler surface. The example is its
  own gate.
- **Coverage check:** `scripts/build-examples.sh` must still *reach*
  `network-server` after `prepare_network_server` is deleted. Confirm it appears
  in the run's output; a removed prepare step that also removes the example from
  the loop would make the gate vacuously green.
- **Runtime proof:** `network-server` and `network-client` exchanging a full
  session on both `--tcp --thread` and `--tls --thread`. A successful *build* is
  not sufficient — the two `ISOLATED` entries moved across a project boundary and
  only a run exercises them.
- **Doc sync:** this letter *is* the doc sync. Additionally run
  `scripts/man-census.sh --memory-scope` if any `mfb man` prose changed; it must
  report 0 unclassified hits.
- **Acceptance:** `cargo test --no-fail-fast` (expected green and untouched —
  a failure here means this letter changed something it should not have);
  `scripts/test-accept.sh`; `scripts/build-examples.sh`.
  No `artifact-gate` run is needed — no compiler code changes — but run it once
  at the end to confirm that claim rather than assume it.

## Open Decisions

- **Sibling file or one big file for the moved wire helpers?** Recommend a
  sibling `src/wire.mfb`: 789 lines in one file reads worse, and the wire
  helpers are a coherent unit. Costs nothing — `PUBLIC` is project-wide, so the
  split is free. (§Phase 2)
- **Keep a threads example that still uses a package?** After this collapse no
  example demonstrates a package-hosted entry. Recommend **leaving it to the
  test suite** (`thread-package-fanout-rt`, renamed in B) rather than keeping a
  second example alive for coverage. (§Phase 2)

## Corrections

<!-- Filled in DURING execution. Record the claim, what was actually true, and
     the evidence. -->

- (none yet)

## Summary

The engineering risk is entirely in Phase 2's code motion: two live protocol
handlers crossing a project boundary, where a build succeeds but a session can
still be wrong. Hence the runtime gate rather than a compile gate.

Phase 1's real content is correcting a spec claim that was never quite true — the
per-package instance — and recording that scoping, not partitioning, is what
makes it hold. That is the sentence most worth getting right in this whole plan,
because it is the one a future change can silently invalidate.
