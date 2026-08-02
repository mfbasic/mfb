# plan-73-A: Timeout convention foundation + thread family

Last updated: 2026-08-01
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing

This is the anchor sub-plan of plan-73. It (1) defines **one** timeout convention
for the whole language and writes it as a canonical spec section, (2) establishes
the shared runtime constants that convention needs, and (3) migrates the
`thread` family — the pilot — to conform. The single behavioral outcome across
plan-73: **every builtin that can wait interprets its optional trailing
`timeoutMs AS Integer` identically** — omit = unbounded, `0` = one immediate
attempt, `> 0` = wait up to that long, `< 0` = `ErrInvalidArgument`, and expiry
raises `ErrTimeout` for a producing call or returns the not-ready value for a
readiness query. No backwards compatibility is kept; all in-tree callers, man
pages, and spec pages are migrated to the new semantics.

References:

- `.ai/compiler.md` — runtime completion gate, validation/function tests, register lifetimes (READ FIRST; this plan touches builtins, codegen, runtime helpers, diagnostics).
- `.ai/specifications.md` — keep the embedded spec current with every compiler change.
- `.ai/remote_systems.md` — remote boxes (thread runtime proof runs locally; tls in plan-73-D needs remotes).
- Spec today: `src/docs/spec/threading/08_queue-semantics.md`, `src/docs/spec/language/16_threads.md`, `src/docs/spec/threading/12_validation.md`, `src/docs/spec/diagnostics/02_error-codes.md`, `src/docs/spec/language/18_builtin-functions.md`.
- Man today: `src/docs/man/builtins/thread/{send,receive,transfer,accept}.md`.

## Prerequisites

These are a precondition on the whole plan-73 feature, stated once here; sub-plans
B–F point back to this table. (Sub-plan F — `io::pollInput` — was appended during
plan-73-E's Phase 1 audit, which found it was a waiting built-in the original
A–E split had missed; it depends only on this anchor, like B–D.)

| Must be true | Command | Status |
|---|---|---|
| Working tree builds & tests green at HEAD | `cargo test` (full suite) | MET — 0 failed at base 594235307 (2026-08-01) |
| Codegen artifact baseline is clean | `scripts/artifact-gate.sh` (debug) → diffs=0 | MET — `artifact-gate.sh target/debug/mfb` diffs=0 at base 594235307 |
| No competing in-flight edits to `src/builtins/{net,tls,audio,thread}.rs` or `src/target/shared/code/{net,tls,audio,*thread*}` | `git status` clean on those paths | MET — worktree forked from clean main |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue and before you
> stop. If you stop, report the status of *all* prerequisites, not just the one
> that blocked you.

Everything below is written against the world where these hold.

## 1. Goal

- A single canonical **"Timeout convention"** spec section exists and is the one
  source every timeout-taking man page cites.
- A shared unbounded-wait sentinel and a single expiry-error path exist in the
  runtime constants, consumed by real code (no dead constants).
- `thread::send`, `thread::receive`, `thread::transfer`, `thread::accept` all
  obey the convention: omit = block until the event or a terminal condition;
  `0` = one immediate attempt raising `ErrTimeout` (never `ErrNotFound`) when the
  event is not already available; `> 0` = bounded, `ErrTimeout` on expiry; `< 0`
  = `ErrInvalidArgument`.
- Every in-tree `thread::*` fixture/example and every thread man/spec page match
  the new semantics; `cargo test` and `scripts/artifact-gate.sh` are green.

### The convention (normative — all of plan-73 implements this)

For any builtin that can wait, the optional trailing `timeoutMs AS Integer`:

| `timeoutMs` | Meaning | Readiness query returns | Producing call does |
|---|---|---|---|
| omitted | unbounded — block until the event or a terminal condition (closed / cancelled / EOF / OS refusal) | (n/a — waits) | (n/a — waits) |
| `0` | one immediate, non-blocking attempt | current-state value (`FALSE` / `-1`) | the event if already available, else `ErrTimeout` |
| `> 0` | wait up to that many ms (clamped to `2147483647` where the host takes a C `int`) | not-ready value on deadline | `ErrTimeout` on deadline |
| `< 0` | rejected | `ErrInvalidArgument` (77050002) | `ErrInvalidArgument` (77050002) |

"Readiness query" = a call that has a not-ready value to return (`net::poll`,
`audio::poll`). "Producing call" = a call that yields a resource/message/bytes
and has no not-ready value (`accept`, `connect`, `receive`, `transfer`, `send`,
`net::read` under a read-timeout). The **only** family-specific choice is which of
those two a function is; everything else in the table is identical everywhere.

### Non-goals (explicit constraints)

- Not adding any new public function or overload in plan-73. (The net/tls list-poll
  and http-async work that motivated this audit is explicitly deferred and is NOT
  part of plan-73.)
- Not changing the *arity* or parameter *names* of any function — only the
  interpretation of the timeout value and the error raised on expiry.
- Not changing non-timeout error codes, resource semantics, or thread isolation.
- Backwards compatibility is explicitly abandoned per the feature owner; do not
  add compatibility shims, dual-mode paths, or deprecation aliases.

## 2. Current State

Timeout handling is inconsistent across five packages (`net`, `tls`, `audio`,
`thread`, plus `http`'s internal fixed deadlines). The four incompatible meanings
of `0`, the two spellings of "forever", and the inconsistent negative handling
were audited in the conversation that produced this plan; the per-function table
is reproduced in each family sub-plan's Current State.

Thread today (`src/docs/man/builtins/thread/*.md`, `src/target/shared/code/runtime_helpers_thread.rs`):

- `thread::send` / `thread::transfer`: `timeoutMs` **defaults to `0`** (padded in
  lowering), and `0` fails immediately with `ErrTimeout` when the queue is full.
  There is **no unbounded form** — omitting the arg gives `0`, i.e. immediate.
- `thread::receive` / `thread::accept`: **omitting** the arg blocks forever
  (lowering pads `THREAD_RECEIVE_BLOCK_SENTINEL`, the `i64::MIN` bit pattern);
  explicit `0` fails immediately with **`ErrNotFound`**; a positive value expires
  with `ErrTimeout`.
- All four reject a negative explicit `timeoutMs` with `ErrInvalidArgument`.

So the thread family is internally split: send/transfer omit = immediate, but
receive/accept omit = block. The convention makes all four: omit = block,
`0` = immediate `ErrTimeout`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `thread::send` call lines (tests+examples) | 41 | `grep -rn --include='*.mfb' -F 'thread::send' tests examples | wc -l` |
| `thread::receive` call lines | 19 | `grep -rn --include='*.mfb' -F 'thread::receive' tests examples | wc -l` |
| `thread::transfer` call lines | 11 | `grep -rn --include='*.mfb' -F 'thread::transfer' tests examples | wc -l` |
| `thread::accept` call lines | 3 | `grep -rn --include='*.mfb' -F 'thread::accept' tests examples | wc -l` |
| thread man pages with `timeoutMs` | 4 | `ls src/docs/man/builtins/thread/{send,receive,transfer,accept}.md` |

Per-site flip census (which of the above pass literal `0` or omit the arg, and
therefore change behavior) is the first task of Phase 3 — it is small and
within-family, and does not affect the split (the split is by family/effort,
already fixed by the aggregate counts above).

### Verified properties

- `THREAD_RECEIVE_BLOCK_SENTINEL` exists and is the `i64::MIN` bit pattern used as
  the block sentinel — VERIFIED: `src/target/shared/code/error_constants.rs:965`
  comment + `runtime_helpers_thread.rs` (`grep -n THREAD_RECEIVE_BLOCK_SENTINEL`).
- `ERR_TIMEOUT_CODE = "77050008"` and `ERR_INVALID_ARGUMENT_CODE` exist —
  VERIFIED: `src/target/shared/code/error_constants.rs:152`.
- Thread queue helpers already implement both a block path and a bounded path —
  VERIFIED by reading `runtime_helpers_thread.rs` (the receive/accept sentinel
  path and the send/transfer bounded path both exist), so the send/transfer
  "omit = block" change reuses the receive/accept block path rather than writing a
  new one. RE-VERIFY the exact helper entry points before editing.

## 3. Design Overview

Three layers, landed in order:

1. **Shared constants (Phase 1).** Promote the block sentinel to a
   convention-level name (`TIMEOUT_UNBOUNDED_SENTINEL`, same `i64::MIN` bits) in
   `error_constants.rs`, re-exported so net/tls/audio can pad omitted timeouts with
   it in B–D. Land it *with* its thread consumer so it is never dead. Confirm
   `ErrTimeout` is the single expiry code; note the collapse of `ErrReadTimeout`/
   `ErrWriteTimeout` (77070005/77070006) is scheduled in plan-73-C (net), and
   `ErrNotFound`-at-`0` removal is in Phase 3 here.
2. **Canonical spec section (Phase 2).** Author the convention as a new spec
   section (recommended home: `src/docs/spec/language/18_builtin-functions.md` as
   a "Timeout convention" subsection, since it is a cross-package language rule).
   Every timeout man page in B–F cites it. This section is the normative contract;
   thread conforms immediately, the other families conform through plan-73-D (and
   `io::pollInput` through plan-73-F), and plan-73-E audits that all man pages
   cite it.
3. **Thread migration (Phase 3).** Flip send/transfer omit-default from `0` to the
   sentinel (block path); change receive/accept explicit-`0` from `ErrNotFound` to
   `ErrTimeout`; keep negative rejection. Update descriptor default-padding
   (`src/builtins/thread.rs`), the queue helper (`runtime_helpers_thread.rs`),
   fixtures/examples, and the four man pages + the threading spec.

**Design uncertainty (schedule first):** that one shared sentinel + one expiry
error can thread through the descriptor default-padding and the queue helper for a
whole family without special-casing. Phase 3 is the cheapest end-to-end proof;
if it needs a per-function carve-out, the convention itself is wrong and B–D must
be re-scoped before starting.

**Correctness risk (this sub-plan):** low — thread already has both a block path
and a bounded path; this is mostly a default-padding flip and an error-code swap,
proven locally (no remote boxes). The high-blast-radius codegen is tls (plan-73-D).

**Rejected alternative:** standardize on `0` = forever (smaller net/tls diff).
Rejected in the audit — it breaks the poll and thread families and forces polls to
invent a second magic value for "check now"; strictly worse. Not re-litigated here.

## Compatibility / Format Impact

- **Behavioral, intentional:** `thread::send(t, x)` / `thread::transfer(t, r)` with
  no timeout change from *immediate* to *block until space*. `thread::receive(t, 0)`
  / `thread::accept(t, 0)` change from `ErrNotFound` to `ErrTimeout`. These are the
  documented convention flips; the proof is this plan + the new spec section.
- **Unchanged:** arities, parameter names, the omit-blocks behavior of
  receive/accept, negative rejection, `ErrTimeout` code value, resource/isolation
  semantics.
- No `.mfp`/wire/layout change.

## Phases

> Keep checkboxes current in the same commit as the work; fill `Commit:` when each
> lands; an unticked box means NOT DONE.

### Phase 1 — Shared timeout constants

Establish the convention's runtime constants, landed with a real consumer.

- [x] In `src/target/shared/code/error_constants.rs`, introduce
      `TIMEOUT_UNBOUNDED_SENTINEL` (the existing `i64::MIN` bit pattern) as the
      convention-level name; keep `THREAD_RECEIVE_BLOCK_SENTINEL` as a re-export/alias
      only if still referenced, else rename all uses. No unused constant may remain.
      — DONE: `THREAD_RECEIVE_BLOCK_SENTINEL` was defined in `runtime_helpers.rs:40`
      (NOT error_constants.rs — see Corrections); removed it and renamed all 3 uses
      (builder_values.rs, runtime_helpers_thread.rs) to `TIMEOUT_UNBOUNDED_SENTINEL`.
- [x] Confirm/annotate `ERR_TIMEOUT_CODE` as the single expiry error for producing
      calls; add a doc comment pointing to the canonical spec section (Phase 2).
- [x] Tests: `cargo test` for the constants module compiles; existing thread tests
      still green (no behavior change yet in this phase). — `cargo test` EXIT 0.

Acceptance: constants build and are referenced by at least the thread helper; no
new dead constant (`cargo build` warns none); `cargo test` green. — MET.
Commit: a234b2e87

### Phase 2 — Canonical spec section

Write the one convention document.

- [x] Add a "Timeout convention" subsection to
      `src/docs/spec/language/18_builtin-functions.md` (or the location chosen in
      Open Decisions) stating the normative table from §1 verbatim, including the
      readiness-query-vs-producing-call rule and the negative/`0`/omit meanings.
      — DONE: added §18.4 with the normative table + readiness/producing lists.
- [x] Note in the section that per-function conformance is completed across
      plan-73; thread conforms as of this plan. — DONE (conforming-functions list).
- [x] ~~Update `.ai/specifications.md` obligations if it enumerates spec sections.~~
      — moot: `.ai/specifications.md` does not enumerate individual spec sections
      (`grep -n '18_builtin\|builtin-functions' .ai/specifications.md` → empty).
- [x] Tests: `cargo test` spec-citation tests still resolve; `mfb spec language`
      renders. — `mfb spec language builtin-functions` shows §18.4; spec-citation
      tests green (run before commit).

Acceptance: `mfb spec language builtin-functions` shows the new section; spec
citation tests green. — MET.
Commit: a234b2e87

### Phase 3 — Thread family migration

Make all four thread waits obey the convention.

- [x] Census: list every `thread::{send,receive,transfer,accept}` call in
      `tests`/`examples` that omits the timeout or passes literal `0`, marking which
      flip. — DONE (see Corrections C3). Behavior-flipping sites: only the
      `receive(_,0)`/`accept(_,0)` literals in `byte-identity/thread` (codegen-only,
      not run) and `func_thread_receive_valid` (syntax test, not run). All `send`/
      `transfer` literal-`0` sites keep their prior immediate-`ErrTimeout` behavior
      (`0` was already immediate for the write helper); omit sites gain block, which
      is a no-op when the queue has space (every in-tree omit site).
- [x] Change send/transfer omitted-timeout padding from `0` to
      `TIMEOUT_UNBOUNDED_SENTINEL` (block); keep receive/accept omit padding as the
      sentinel; keep negative rejection. — DONE in
      `src/target/shared/code/builder_values.rs::lower_runtime_helper_call` (NOT
      `src/builtins/thread.rs` — padding lives in codegen lowering; Corrections C1/C2).
      Also added the previously-missing `transferResource` padding.
- [x] `src/target/shared/code/runtime_helpers_thread.rs`: route send/transfer omit
      to the block path (write helper: sentinel-aware prologue + `wait_indefinite`
      `pthread_cond_wait` on NOT_FULL); change receive/accept explicit-`0` result
      from `ErrNotFound` to `ErrTimeout` (read helper: `timeout==0 → timeout` label,
      leaving the closed/completed paths on `ErrNotFound`); sentinel honored
      uniformly. — DONE.
- [x] Migrate every flipped fixture/example + regenerate goldens. — DONE: no in-tree
      fixture asserted `receive/accept(_,0)==ErrNotFound` at RUNTIME (the literal-`0`
      sites are codegen/syntax-only). Regenerated `byte-identity/thread` `.ncodesum`
      for all 4 targets (codegen changed) and seeded the new fixture (below).
- [x] Rewrite `src/docs/man/builtins/thread/{send,receive,transfer,accept}.md` to
      the convention, each citing the Phase-2 section (See-also
      `mfb spec language builtin-functions`). Update
      `src/docs/spec/threading/08_queue-semantics.md` and
      `src/docs/spec/language/16_threads.md`. — DONE.
- [x] ~~Update `src/docs/spec/threading/12_validation.md`~~ — moot: it mentions
      "full-queue timeouts" generically with no `0`/`ErrNotFound` wording to change
      (`grep -n "ErrNotFound\|= 0" 12_validation.md` → empty).
- [x] ~~Update `src/docs/spec/diagnostics/02_error-codes.md`~~ — moot: `ErrNotFound`
      is described generically there ("Requested item … not found"); the
      receive/accept-at-`0` code was never documented in that table, so nothing to
      remove for plan-73-A. (Net's code collapse in plan-73-C does touch this file.)
- [x] Tests: add rt-behavior test proving `receive(t,0)`→`ErrTimeout` and negative→
      `ErrInvalidArgument` for the read (`receive`) and write (`send`) helpers.
      — DONE: `tests/rt-behavior/threads/thread-timeout-convention-rt` (runtime-proven:
      prints `receive0 timeout` / `receive-neg invalid` / `send-neg invalid`).
      `accept`/`transfer` route through the SAME `thread_queue_read_helper`/
      `thread_queue_write_helper` (verified: `runtime_helpers.rs` `acceptResource`→
      read helper, `transferResource`→write helper), so this runtime proof covers
      them; their resource-plane CODEGEN is covered byte-identically by
      `byte-identity/thread` (all four overloads).

Acceptance: the new rt-behavior test passes; `cargo test` full suite green;
`scripts/artifact-gate.sh` diffs=0 after golden/`.ncodesum` regen; man_citations
and spec-citation tests green; `mfb man thread send` shows the new semantics.
Commit: a234b2e87

## Validation Plan

- Tests: rt-behavior fixtures under `tests/rt-behavior/threads/` for each of the
  four flips + negative rejection; unit tests in `src/builtins/thread.rs`.
- Coverage check: confirm the changed helper/​descriptor lines are in the suite
  denominator (the new fixtures exercise the `0` and omit paths).
- Runtime proof: run a small program locally that does `receive(t,0)` on an empty
  queue and prints the error code (expect `77050008`), and one that `send`s to a
  full queue with no timeout and observes it block then unblock.
- Doc sync: the four thread man pages, the three threading spec pages, the
  error-codes page, the new language spec section, `.ai/specifications.md`.
- Acceptance: `cargo test`, `scripts/artifact-gate.sh` (debug, diffs=0), and the
  acceptance golden harness for touched fixtures.

## Open Decisions

- **Canonical section home** — `src/docs/spec/language/18_builtin-functions.md`
  "Timeout convention" subsection (recommended, it is a cross-package language
  rule) vs. a new top-level spec topic. (§3 Phase 2)
- **`0` for readiness queries with an unbounded form** — under the convention,
  `net::poll(sock)` / `audio::poll(stream)` with no arg become *blocking* (wait
  until ready), and callers wanting the old immediate check write `, 0`. Recommended:
  keep it uniform (no poll-specific exception); the flips land in plan-73-C/B.
  Recorded here because it is a convention-wide consequence, resolved in those
  sub-plans. (§1 table)

## Corrections

- **C1 (Phase 1 constant location).** The plan's Verified-properties and Phase 1
  said `THREAD_RECEIVE_BLOCK_SENTINEL` lived at `error_constants.rs:965`. That line
  is only a *comment* mentioning the name; the actual `const` was defined in
  `src/target/shared/code/runtime_helpers.rs:40` (`grep -rn THREAD_RECEIVE_BLOCK_SENTINEL src`).
  Resolution: defined the new `TIMEOUT_UNBOUNDED_SENTINEL` in `error_constants.rs`
  (as the plan intends — a shared `pub(crate)` home re-exported to net/tls/audio),
  removed the `runtime_helpers.rs` definition, and renamed all uses. No behavior
  change; codegen byte-identical (artifact-gate diffs=0).

- **C2 (`thread::transfer` was never timeout-padded).** Current State §2 said
  "`thread::send` / `thread::transfer`: `timeoutMs` defaults to `0` (padded in
  lowering)". Verified FALSE for transfer: the `.nir` dump of
  `func_thread_transfer_valid` shows `thread.transferResource` reaching codegen with
  **2 args** (`[t, f]`) and no timeout — the only padding sites
  (`builder_values.rs::lower_runtime_helper_call`) cover `thread.send` (→`0`) and
  `thread.receive`/`thread.acceptResource` (→sentinel), never `transferResource`.
  So `transfer(t,r)` today passes an **uninitialised** `x2` to the write helper (it
  happens to work because the queue has space, so the timeout is never consulted and
  the prologue's `x2 < 0` check happens to see a non-negative leftover). This is a
  latent fragility. Phase 3 fixes it by padding `thread.transferResource` (2-arg)
  with `TIMEOUT_UNBOUNDED_SENTINEL` — which is both the convention's omit=block
  behavior AND removes the uninitialised-register hazard.

- **C3 (Phase 3 flip census).** `grep -rn --include='*.mfb' -E 'thread::(receive|accept)\([^,)]+,\s*0\s*\)' tests examples`
  → 3 literal-`0` read sites: `tests/syntax/threads/func_thread_receive_valid`
  (syntax/compile-only, not executed), and `tests/byte-identity/thread` lines 36
  (`receive`) + 49 (`accept`) (codegen `.ncodesum` only, `-ast -ir` build, not
  executed). No **executed** fixture asserted `receive/accept(_,0) == ErrNotFound`,
  so the `ErrNotFound→ErrTimeout` flip changed no runtime golden. `send`/`transfer`
  literal-`0` sites (queue-timeout-cancel, bounded-queues, byte-identity) keep their
  prior immediate-`ErrTimeout` semantics (write helper's `0` was already immediate).
  The codegen change did alter every thread fixture's emitted bytes; only
  `byte-identity/thread` commits `.ncodesum` goldens, regenerated for all 4 targets.

- **C4 (sync-goldens does not refresh target-infixed `.ncodesum`).**
  `scripts/sync-goldens.sh` copies actuals by *exact golden filename*, but a native
  `.ncodesum` golden carries a `.<target>` infix the build output lacks, so the
  names never match and the sums are silently left stale (the artifact-gate does the
  infix mapping; sync-goldens does not). Regenerated the 4 `.ncodesum` goldens by
  hand the way the gate computes them: `mfb build -q -ncode -target <T>` then
  `shasum -a 256 <ncode>` → `golden/<pkg>.<T>.ncodesum`. (Also hit the zsh
  no-word-split trap: `$flag_with_value` is ONE arg — passed `-target "$T"` as two.)

## Summary

Real risk in plan-73 is concentrated in the tls backends (plan-73-D); this anchor
sub-plan carries almost none — it fixes the thread family's internal split and
lays the shared convention + constants + spec section that B–E build on. Untouched
here: net, tls, audio, http, and any new poll/async surface.
