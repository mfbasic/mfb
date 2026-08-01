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
B–E point back to this table.

| Must be true | Command | Status |
|---|---|---|
| Working tree builds & tests green at HEAD | `cargo test` (full suite) | UNVERIFIED — run before starting |
| Codegen artifact baseline is clean | `scripts/artifact-gate.sh` (debug) → diffs=0 | UNVERIFIED — run before starting |
| No competing in-flight edits to `src/builtins/{net,tls,audio,thread}.rs` or `src/target/shared/code/{net,tls,audio,*thread*}` | `git status` clean on those paths | UNVERIFIED |

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
   Every timeout man page in B–E cites it. This section is the normative contract;
   thread conforms immediately, the other families conform through plan-73-D, and
   plan-73-E audits that all man pages cite it.
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

- [ ] In `src/target/shared/code/error_constants.rs`, introduce
      `TIMEOUT_UNBOUNDED_SENTINEL` (the existing `i64::MIN` bit pattern) as the
      convention-level name; keep `THREAD_RECEIVE_BLOCK_SENTINEL` as a re-export/alias
      only if still referenced, else rename all uses. No unused constant may remain.
- [ ] Confirm/annotate `ERR_TIMEOUT_CODE` as the single expiry error for producing
      calls; add a doc comment pointing to the canonical spec section (Phase 2).
- [ ] Tests: `cargo test` for the constants module compiles; existing thread tests
      still green (no behavior change yet in this phase).

Acceptance: constants build and are referenced by at least the thread helper; no
new dead constant (`cargo build` warns none); `cargo test` green.
Commit: —

### Phase 2 — Canonical spec section

Write the one convention document.

- [ ] Add a "Timeout convention" subsection to
      `src/docs/spec/language/18_builtin-functions.md` (or the location chosen in
      Open Decisions) stating the normative table from §1 verbatim, including the
      readiness-query-vs-producing-call rule and the negative/`0`/omit meanings.
- [ ] Note in the section that per-function conformance is completed across
      plan-73; thread conforms as of this plan.
- [ ] Update `.ai/specifications.md` obligations if it enumerates spec sections.
- [ ] Tests: `cargo test` spec-citation tests still resolve; `mfb spec language` renders.

Acceptance: `mfb spec language builtin-functions` shows the new section; spec
citation tests green.
Commit: —

### Phase 3 — Thread family migration

Make all four thread waits obey the convention.

- [ ] Census: list every `thread::{send,receive,transfer,accept}` call in
      `tests`/`examples` that omits the timeout or passes literal `0`
      (`grep -rn --include='*.mfb' -E 'thread::(send|receive|transfer|accept)' tests examples`),
      marking which flip. Record the list in this sub-plan's Corrections if it
      differs from the aggregate counts.
- [ ] `src/builtins/thread.rs`: change send/transfer omitted-timeout padding from
      `0` to `TIMEOUT_UNBOUNDED_SENTINEL` (block); keep receive/accept omit padding
      as the sentinel; keep negative rejection in resolve/validation.
- [ ] `src/target/shared/code/runtime_helpers_thread.rs`: route send/transfer omit
      to the block path; change receive/accept explicit-`0` result from
      `ErrNotFound` to `ErrTimeout`; verify the sentinel is honored uniformly.
- [ ] Migrate every flipped fixture/example so its expected behavior matches the new
      semantics (rewrite any test that asserted immediate-`ErrNotFound` at `0`, or
      relied on send/transfer omit = immediate). Regenerate affected goldens with the
      acceptance harness (`scripts/sync-goldens.sh`), seeding new rt-behavior goldens
      from a RELEASE build.
- [ ] Rewrite `src/docs/man/builtins/thread/{send,receive,transfer,accept}.md` to
      the convention (follow `.ai/man_template.md`; run `scripts/update_man.sh`),
      each citing the Phase-2 section. Update `src/docs/spec/threading/08_queue-semantics.md`,
      `src/docs/spec/language/16_threads.md`, and `src/docs/spec/threading/12_validation.md`.
- [ ] Update `src/docs/spec/diagnostics/02_error-codes.md` if `ErrNotFound` was the
      documented receive/accept-at-`0` code.
- [ ] Tests: add/adjust rt-behavior tests proving `receive(t,0)`→`ErrTimeout`,
      `accept(t,0)`→`ErrTimeout`, `send`/`transfer` omit blocks (bounded by a
      positive-timeout variant in the harness to stay deterministic), and negative→
      `ErrInvalidArgument` for all four.

Acceptance: the new rt-behavior tests pass; `cargo test` full suite green;
`scripts/artifact-gate.sh` diffs=0 after golden/`.ncodesum` regen; man_citations
and spec-citation tests green; `mfb man thread send` shows the new semantics.
Commit: —

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

<Filled during execution.>

## Summary

Real risk in plan-73 is concentrated in the tls backends (plan-73-D); this anchor
sub-plan carries almost none — it fixes the thread family's internal split and
lays the shared convention + constants + spec section that B–E build on. Untouched
here: net, tls, audio, http, and any new poll/async surface.
