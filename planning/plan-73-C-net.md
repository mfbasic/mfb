# plan-73-C: net family timeout migration

Last updated: 2026-08-01
Effort: large (3h–1d)
Depends on: plan-73-A (convention, constants, spec section). Independent of B/D, but by letter order lands after B.

Migrate the `net` family to the plan-73 timeout convention (plan-73-A §1):
`net::poll`, `net::accept`, `net::connectTcp`, `net::setReadTimeout`,
`net::setWriteTimeout`. Also collapse the two net-only expiry error codes
(`ErrReadTimeout` 77070005, `ErrWriteTimeout` 77070006) into the single
`ErrTimeout` (77050008), because "one way timeouts work" includes one expiry error.

References:

- `.ai/compiler.md` (READ FIRST), `.ai/specifications.md`.
- plan-73-A — the convention + canonical spec section.
- Codegen: `src/target/shared/code/net/{poll,io,mod}.rs`; specs
  `src/target/shared/runtime/net_specs.rs`; descriptor `src/builtins/net.rs`;
  constants `src/target/shared/code/error_constants.rs`.
- Man: `src/docs/man/builtins/net/{poll,accept,connectTcp,setReadTimeout,setWriteTimeout,read,readText,write,writeText}.md`.
- Spec: `src/docs/spec/diagnostics/02_error-codes.md`, `src/docs/spec/stdlib/*`.

## Prerequisites

See plan-73-A's Prerequisites table. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-73-A complete | `mfb spec language builtin-functions` shows "Timeout convention" | MET — A landed a234b2e87 |

If plan-73-A is not complete, this sub-plan cannot start, full stop.

## 1. Goal

Post-migration net semantics (all per plan-73-A §1):

- `net::poll(sock[, timeoutMs]) AS Boolean` (readiness query): omit = block until
  readable; `0` = immediate check (`FALSE` if not readable); `> 0` = wait, clamp
  `2147483647`; `< 0` = `ErrInvalidArgument`. (Only change: omit flips from
  immediate to block; today omit pads `0`.)
- `net::accept(listener[, timeoutMs]) AS Socket` (producing): omit = block until a
  client; `0` = one immediate attempt, `ErrTimeout` if none pending; `> 0` = wait
  then `ErrTimeout`; `< 0` = `ErrInvalidArgument`. (Today `0`/omit = block forever,
  `≤0` = block — this flips explicit `0` and rejects negatives.)
- `net::connectTcp(host, port[, timeoutMs])` / `(address[, timeoutMs])` (producing):
  omit = block until connected or the OS refuses; `0` = one non-blocking connect
  attempt (`ErrTimeout` unless it completes immediately); `> 0` = bounded; `< 0` =
  `ErrInvalidArgument`. The `DEFAULT_CONNECT_TIMEOUT_MS = 120000` safety default is
  **removed** — the "never wedge a thread" property now belongs to callers that
  pass a positive timeout (http already does: `__HTTP_CONNECT_TIMEOUT_MS`).
- `net::setReadTimeout` / `setWriteTimeout(sock, timeoutMs)` (socket option):
  `0` = subsequent reads/writes are non-blocking (immediate `ErrTimeout` when no
  data / not writable); `> 0` = bounded, `ErrTimeout` on elapse; `< 0` =
  `ErrInvalidArgument` (already rejected). Unbounded is the socket's initial state;
  the setter has no omit form, so it can only bound — restoring unbounded via the
  setter is not expressible (documented). **`0` flips from "disable timeout" to
  "non-blocking".**
- `net::read`/`readText`/`write`/`writeText` raise **`ErrTimeout`** on a
  read/write-timeout expiry (was `ErrReadTimeout`/`ErrWriteTimeout`).
- Every net fixture/example, the net man pages, and the error-codes spec match;
  `cargo test` + `artifact-gate` green.

### Non-goals

- No new net functions or overloads (list-poll is deferred, not part of plan-73).
- No change to UDP receive/send semantics beyond the shared negative/expiry rule.

## 2. Current State

From the audit (man pages + codegen read):

- `net::poll`: `0` = immediate; `> 0` wait, clamp `2147483647`; `< 0`
  `ErrInvalidArgument`; omit pads `0` (`src/builtins/net.rs` POLL,
  `src/target/shared/code/net/poll.rs`). Already convention-shaped **except** omit.
- `net::accept`: omit pads `0`; `≤ 0` takes the blocking path (block forever);
  `> 0` polls to deadline and raises `ErrTimeout` (`src/docs/man/builtins/net/accept.md:34-38`,
  `src/target/shared/code/net/io.rs`).
- `net::connectTcp`: omit/`0`/negative all fall to `DEFAULT_CONNECT_TIMEOUT_MS`
  (120000) bounded default; positive honored (`src/target/shared/code/net/mod.rs:454`,
  `:658`; `src/docs/man/builtins/net/connectTcp.md:46-52`).
- `net::setReadTimeout`/`setWriteTimeout`: `0` disables the timeout (block
  indefinitely); positive bounds; negative → `ErrInvalidArgument`; expiry raises
  `ErrReadTimeout`/`ErrWriteTimeout` (`src/docs/man/builtins/net/setReadTimeout.md:40-49`;
  `src/target/shared/code/net/poll.rs` set-timeout helper; `io.rs` read/write path).
- Expiry codes: `ERR_READ_TIMEOUT_CODE = 77070005`, `ERR_WRITE_TIMEOUT_CODE =
  77070006` (`src/target/shared/code/error_constants.rs:221`, `:224`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `net::connectTcp` call lines | 38 | `grep -rn --include='*.mfb' -F 'net::connectTcp' tests examples | wc -l` |
| `net::accept` call lines | 19 | `grep -rn --include='*.mfb' -F 'net::accept' tests examples | wc -l` |
| `net::setReadTimeout` call lines | 14 | `grep -rn --include='*.mfb' -F 'net::setReadTimeout' tests examples | wc -l` |
| `net::setWriteTimeout` call lines | 10 | `grep -rn --include='*.mfb' -F 'net::setWriteTimeout' tests examples | wc -l` |
| `net::poll` call lines | 8 | `grep -rn --include='*.mfb' -F 'net::poll' tests examples | wc -l` |
| `ErrReadTimeout`/`ErrWriteTimeout` references (tests+src) | UNMEASURED | first task of Phase 3 |

Per-site flip census (which pass literal `0` / omit / negative) is the first task
of each phase; small and within-family.

### Verified properties

- `DEFAULT_CONNECT_TIMEOUT_MS = "120000"` is the connect safety default consumed at
  `net/mod.rs:658` — VERIFIED (`grep -n DEFAULT_CONNECT_TIMEOUT_MS`).
- http passes an explicit connect timeout so removing the default does not un-bound
  http — VERIFIED: `http_package.mfb` uses `__HTTP_CONNECT_TIMEOUT_MS` at the
  `net::connectTcp(...)` call. RE-CONFIRM before deleting the default.
- `ErrReadTimeout`/`ErrWriteTimeout` are net-only (not raised by tls, which uses
  `ErrTlsFailed`/`ErrTimeout`) — VERIFIED from the tls man error table; collapsing
  them affects only net + the error-codes doc.

## 3. Design Overview

Four independent flips + one error collapse, ordered least-to-most blast radius:

1. **`net::poll` omit = block (Phase 1).** Smallest: change omit padding in
   `src/builtins/net.rs` from `0` to `TIMEOUT_UNBOUNDED_SENTINEL`; add a block path
   (poll with an infinite timeout) in `poll.rs`. Everything else already conforms.
2. **`net::accept` explicit-`0` + negatives (Phase 2).** In `io.rs`, split the
   current `≤ 0 → block` into: sentinel → block, `0` → one attempt (`ErrTimeout`),
   `< 0` → `ErrInvalidArgument`. Descriptor omit padding → sentinel.
3. **`net::connectTcp` (Phase 3).** Remove `DEFAULT_CONNECT_TIMEOUT_MS`; sentinel →
   block, `0` → one non-blocking connect attempt, `< 0` → error, `> 0` bounded.
   Confirm http still passes an explicit timeout.
4. **`setReadTimeout`/`setWriteTimeout` + error collapse (Phase 4 — largest blast
   radius).** Flip the option's `0` from disable→non-blocking; make read/write
   expiry raise `ErrTimeout`; retire `ERR_READ_TIMEOUT_CODE`/`ERR_WRITE_TIMEOUT_CODE`
   and their symbols from `io.rs`, `error_constants.rs`, and the error-codes spec.

**Correctness risk concentrates in Phase 4** — it changes an error code observed by
many fixtures and touches the read/write hot path across POSIX + Winsock
(`io.rs` has `WSAETIMEDOUT` handling). Schedule last, behind its own tests.

**Design uncertainty (schedule first):** that an *unbounded* `net::poll` and a
*one-attempt* `connectTcp` lower cleanly. Phase 1 falsifies the first cheaply;
Phase 3 the second. If either needs a carve-out, revisit the convention with A.

**Rejected alternative:** keep `ErrReadTimeout`/`ErrWriteTimeout` as ErrTimeout
subtypes. Rejected per the "one expiry error" goal; the more specific codes carried
no behavior callers branched on that `ErrTimeout` cannot.

## Compatibility / Format Impact

- **Behavioral, intentional flips:** `net::poll(sock)` omit → block;
  `net::accept(l, 0)` → immediate `ErrTimeout`; `connectTcp` omit → block (120 s
  default removed) and `0` → immediate attempt; `setReadTimeout/​setWriteTimeout(s, 0)`
  → non-blocking; read/write expiry error 77070005/77070006 → 77050008; negatives
  now rejected where previously folded into a default.
- **Unchanged:** arities, names, `net::poll` `Boolean` return, positive-timeout
  behavior, UDP payload semantics, the `2147483647` clamp.
- Error-code table loses 77070005/77070006.

## Phases

> Keep checkboxes current in-commit; fill `Commit:`; unticked = NOT DONE.

### Phase 1 — net::poll omit = block

- [x] omit padding for `POLL` → `TIMEOUT_UNBOUNDED_SENTINEL`. — DONE in
      `src/target/shared/code/builder_values.rs::lower_runtime_helper_call` (net poll
      padding lives there, not `src/builtins/net.rs`); split `net.poll` from
      `net.accept` (accept keeps `0` until Phase 2).
- [x] `src/target/shared/code/net/poll.rs`: route the sentinel to a
      block-until-readable path (poll with a -1 timeout); keep `0`=immediate,
      `>0`=wait+clamp, `<0`=`ErrInvalidArgument`. — DONE (sentinel→`poll_infinite`
      →`bitwise_not(ZERO)`=-1 before the negative-reject).
- [x] Migrate `net::poll` sites. — NONE needed: `func_net_poll_valid` uses explicit
      `1000`; `byte-identity/net`'s omit sites are codegen-only (`-ast -ir`, not run);
      `func_net_poll_invalid` is compile-error. Regenerated `byte-identity/net`
      `.ncodesum` (5 targets).
- [x] Rewrite `src/docs/man/builtins/net/poll.md` to the convention (cite A's
      section). — DONE (omit=block; `,0`=immediate; example switched to `,0`; See-also
      cites `mfb spec language builtin-functions`).
- [x] Tests: rt-behavior proving omit blocks and `,0` is immediate. — DONE:
      `tests/rt-behavior/net/net-poll-timeout-convention-rt` (runtime-proven on
      loopback: `immediate FALSE` / `omit TRUE` / `neg invalid`).

Acceptance: poll tests pass; `artifact-gate` diffs=0 after `.ncodesum` regen; man
cites the section. — MET (`cargo test` green; net acceptance passes; gate below).
Commit: —

### Phase 2 — net::accept explicit-0 + negatives

- [ ] `src/builtins/net.rs`: `ACCEPT` omit padding → sentinel; reject negatives.
- [ ] `src/target/shared/code/net/io.rs`: sentinel→block, `0`→one attempt (`ErrTimeout`),
      `>0`→wait then `ErrTimeout`, `<0`→`ErrInvalidArgument`.
- [ ] Migrate the 19 `net::accept` sites; regenerate goldens.
- [ ] Rewrite `src/docs/man/builtins/net/accept.md` (cite the section).
- [ ] Tests: `accept(l,0)`→`ErrTimeout` when none pending; omit blocks; `<0`→invalid.

Acceptance: accept tests pass; `artifact-gate` diffs=0; `cargo test` green.
Commit: —

### Phase 3 — net::connectTcp

- [ ] Confirm http passes an explicit connect timeout (`grep __HTTP_CONNECT_TIMEOUT_MS src/builtins/http_package.mfb`).
- [ ] `src/target/shared/code/net/mod.rs`: remove `DEFAULT_CONNECT_TIMEOUT_MS` and its
      `≤0→default` path; sentinel→block, `0`→one non-blocking attempt (`ErrTimeout`),
      `<0`→`ErrInvalidArgument`, `>0` bounded. `src/builtins/net.rs`: omit padding → sentinel.
- [ ] Migrate the 38 `connectTcp` sites (most pass positive timeouts — unaffected;
      flip only omit/`0`/negative sites); regenerate goldens.
- [ ] Rewrite `src/docs/man/builtins/net/connectTcp.md` (cite the section; note callers
      own the never-wedge property).
- [ ] Tests: `connectTcp(host,port,0)` to a non-listening port → `ErrTimeout`; `<0`→invalid.

Acceptance: connect tests pass; http fixtures still pass (still bounded); `artifact-gate` diffs=0.
Commit: —

### Phase 4 — set*Timeout flip + ErrTimeout collapse (largest blast radius)

- [ ] Census `ErrReadTimeout`/`ErrWriteTimeout`/77070005/77070006 across src+tests
      (`grep -rn -E '77070005|77070006|ErrReadTimeout|ErrWriteTimeout' src tests`).
- [ ] `src/target/shared/code/net/io.rs` + `poll.rs` (set-timeout helper): option `0`
      → non-blocking; read/write expiry raises `ERR_TIMEOUT_CODE`; remove
      `ERR_READ_TIMEOUT_CODE`/`ERR_WRITE_TIMEOUT_CODE` and their symbols. Preserve the
      Winsock `WSAETIMEDOUT`→timeout mapping (bug-109) but map to `ErrTimeout`.
- [ ] `src/target/shared/code/error_constants.rs`: delete the two codes (no dead
      constants left). Update `src/target/shared/runtime/net_specs.rs` if it names them.
- [ ] Migrate every fixture asserting 77070005/77070006 → 77050008, and any relying on
      `setReadTimeout(s,0)` = unbounded; regenerate goldens.
- [ ] Rewrite `src/docs/man/builtins/net/{setReadTimeout,setWriteTimeout,read,readText,write,writeText}.md`;
      update `src/docs/spec/diagnostics/02_error-codes.md` (remove the two codes).
- [ ] Tests: read on a socket with `setReadTimeout(s,0)` and no data → `ErrTimeout`
      (77050008); positive timeout expiry → 77050008; `<0`→invalid.

Acceptance: all read/write-timeout tests assert 77050008; no reference to
77070005/77070006 remains (`grep` empty); error-codes spec has neither; `cargo test`
full green; `artifact-gate` diffs=0; man_citations + spec-citation green.
Commit: —

## Validation Plan

- Tests: rt-behavior under `tests/rt-behavior/net/` (or existing net dirs) for each
  flip + negative rejection + the error collapse; unit tests in `src/builtins/net.rs`.
- Coverage check: the `0`/omit/negative branches and the `ErrTimeout` raise are in
  the suite denominator.
- Runtime proof: run locally — `accept(l,0)` on an idle listener → 77050008;
  `connectTcp(host,port,0)` to a closed port → 77050008; a read past a `0`
  read-timeout → 77050008.
- Doc sync: 9 net man pages + error-codes spec + citations + `.ai/specifications.md`.
- Acceptance: `cargo test`, `scripts/artifact-gate.sh` diffs=0 (`.ncodesum` regen for
  all four targets on the macOS host per the fast-codegen-gate note), acceptance
  golden harness for touched fixtures.

## Open Decisions

- **setReadTimeout unbounded-restore** — accept that the setter can only bound (a
  fresh socket is unbounded; no in-band value restores it) vs. add a separate
  clear/`net::clearReadTimeout`. Recommended: accept and document; no new function
  (plan-73 adds none). (§1)
- **ErrReadTimeout/ErrWriteTimeout** — collapse into `ErrTimeout` (recommended,
  baked into Phase 4) vs. keep as subtypes. If the owner vetoes the collapse, drop
  Phase 4's error work and keep only the `0`-flip. (§3)

## Corrections

<Filled during execution.>

## Summary

The net family is the broadest single migration: four value-flips plus an
error-code collapse across the POSIX/Winsock read/write path. Risk is in Phase 4
(observable error-code change on a hot path); Phases 1–3 are value-padding flips.
tls (plan-73-D) carries the cross-backend risk; net stays single-backend-per-platform.
