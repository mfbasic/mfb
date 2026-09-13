# bug-582: checkpoint pinning accepts an unproven larger transparency-log fork

Last updated: 2026-09-12
Effort: medium (1h–2h)
Severity: HIGH
Class: Security / transparency-log integrity

Status: FIXED (`ac1a6ec79`)
Regression Test: `client::tests::fetch_checkpoint_rejects_an_unproven_larger_fork`
and `client::tests::verify_publish_inclusion_rejects_an_unproven_larger_fork`
in `repository/src/client.rs`.

Once a client has pinned checkpoint `(size=N, root=A)`, `fetch_checkpoint`
accepts any validly server-signed checkpoint with `size>N`, without requesting
or verifying a consistency proof.  It immediately overwrites the local pin.
`verify_publish_inclusion` uses this unsafe helper, so an online registry can
present a larger fork and erase the old root before any consistency check.

Correct behavior: after the first trust-on-first-use checkpoint, a candidate
larger head must not replace the pin until an RFC 6962 consistency proof proves
it extends the pinned root.

References:

- Security review, 2026-09-11 (transparency protocol pass)
- Existing contrast: `client::verify_log_consistency` already follows the
  required verify-before-pin order.

## Failing Reproduction

Add a case after the existing four-leaf checkpoint pin that serves a different,
validly signed five-leaf root but no `/log/consistency` proof:

```
cargo test -p mfb_repository client::tests::fetch_checkpoint_rejects_an_unproven_larger_fork
```

- Observed today: `fetch_checkpoint` accepts the larger checkpoint and rewrites
  `checkpoint` on disk.
- Expected: it rejects and keeps `(N, A)` unchanged.

Add the same attack to `verify_publish_inclusion`, which currently calls
`fetch_checkpoint` directly.

## Root Cause

`repository/src/client.rs:fetch_checkpoint_unpinned` rejects only a smaller
size and a different root at the *same* size.  `fetch_checkpoint` then writes a
larger candidate without consulting `/log/consistency`.  In contrast,
`verify_log_consistency` deliberately leaves the old pin intact until
`log::verify_consistency` succeeds, but no call path requires it before
`fetch_checkpoint` or `verify_publish_inclusion` advances the pin.

## Goal

- Make every checkpoint advancement consistency-proof-gated.
- Ensure failed proof retrieval/verification never changes the pin.

### Non-goals (must NOT change)

- Do not reject the first checkpoint, which has no predecessor.
- Do not change checkpoint or consistency-proof wire encodings.
- Do not rely on checkpoint signatures alone to establish append-only history.

## Blast Radius

- `client.rs:fetch_checkpoint` — fixed in this bug.
- `client.rs:verify_publish_inclusion` — fixed because it advances through the
  unsafe helper.
- `client.rs:verify_log_consistency` — unaffected semantically; use it as the
  correctness reference and avoid duplicating incompatible logic.

## Fix Design

Fold the verify-before-pin logic into the one checkpoint-advance primitive, so
all callers must obtain and verify consistency when a pin exists and the size
increases.  Preserve a separate non-pinning fetch only where a caller genuinely
needs a candidate head.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add red larger-fork tests for direct checkpoint and publish inclusion.
- [x] Census all local checkpoint writers and candidate-head readers.

Acceptance: tests prove the larger fork currently overwrites the pin. **Met** —
both new tests failed on the pre-fix compiler, on the accept-the-fork assertion
(`a larger head with no consistency proof must not be accepted`, and
`inclusion must not be reported against an unproven fork`).

**Census** (`grep -rn --include='*.rs'` for `fetch_checkpoint`,
`verify_log_consistency`, `verify_publish_inclusion`, `write_checkpoint`,
tree-wide): the only pin writers are `local::write_checkpoint`'s two call sites
inside `client.rs`. Consumers outside the crate are
`src/cli/resolve.rs:641` and `src/cli/pkg.rs:181` (both already on the safe
`verify_log_consistency`), and `src/cli/pkg.rs:216` / `src/cli/pkg.rs:1645`
(both `verify_publish_inclusion`).

**The doc understated the reach, and the census is what showed it.** The report
framed this as publish-time. `pkg install --proof` (`pkg.rs:1645`) calls
`verify_publish_inclusion` with **no** prior `verify_log_consistency` anywhere
in that flow, so on the install path the unsafe helper was not merely a second
chance to be forked — it was the *only* checkpoint contact. That is the
strongest exploit path and it is an install, not a publish.
Commit: — (tests landed with the fix, `ac1a6ec79`)

### Phase 2 — the fix

- [x] Centralize consistency verification before every pin advance.
- [x] Route publish inclusion through the safe primitive.

The gate moved **into** `fetch_checkpoint`, which is now the single pin-advance
primitive; `verify_log_consistency` is a named alias of it. Three cases:

| pin state | behaviour |
|---|---|
| none | trust on first use — no predecessor to prove an extension of |
| `size == pinned_size` | `fetch_checkpoint_unpinned` has already refused every other root at this size, so this IS the pinned head: nothing advances, nothing is proven, and **no** `/log/consistency` request is made |
| `size > pinned_size` | fetch and verify the RFC 6962 proof against the **pinned** root before `write_checkpoint` |

A smaller size never reaches the gate — `fetch_checkpoint_unpinned` rejects it
as a ROLLBACK. The pin is read *before* the fetch, so the value proven against
is the one the client already accepted.

Acceptance: attacks reject without changing the old pin; valid extensions pass.
**Met**, and the tests assert both sides:

- *Negative, absent proof*: a validly signed larger fork with no
  `/log/consistency` route is refused and the pin still reads `(4, root4)`.
- *Negative, internally-valid proof*: a proof computed over the **fork's own**
  history (`consistency_path(4, &fork)`) is still refused — "consistency proof
  does not reproduce". This is the sharper half: it proves the check is
  anchored to the pinned root rather than to the registry's arithmetic.
- *Positive*: a genuine 4 -> 6 extension advances the pin and the proof is
  requested `from=4&to=6`.
- *Positive*: re-reading the already-pinned head succeeds and issues **no**
  consistency request.
- *Positive*: the honest publish under a proven 2 -> 4 extension still verifies
  inclusion and advances the pin.

The positive pins are not optional. Without them "reject every advance" would
have been green, and it would have broken every honest `pkg publish` and
`pkg install --proof`.
Commit: `ac1a6ec79`

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository`.
- [x] Re-run rollback, same-size fork, valid extension, and larger-fork cases.

`cargo test -p mfb_repository --no-fail-fast` -> **332 passed, 0 failed,
exit 0**. `cargo check --all-targets` -> exit 0. Both were run in an isolated
`git worktree add --detach` at HEAD, because a peer agent held in-flight
`repository/src/abi.rs` edits in the shared checkout that did not compile; the
tested tree was then `diff`ed byte-for-byte against the committed one.

Rollback, same-size fork and bad-signature are re-run unmodified by the
pre-existing `fetch_checkpoint_pins_then_refuses_rollback_fork_and_bad_signatures`,
which passes untouched — the gate is purely additive to the size-growth path and
does not relax any existing refusal.

**The instrument.** This is repository client transport logic: it emits no IR
and moves no golden, so the artifact gate is structurally blind to it and its
silence would prove nothing. The `mfb_repository` unit suite, with its
loopback-HTTP stub registry, is the only instrument that exercises the protocol,
and it is where both REDs and all four positive pins live.

Acceptance: full suite green and no unproven history can replace a pin. **Met.**
Commit: `ac1a6ec79`

## Outcome

Fixed in `ac1a6ec79`.

Two findings outlive the bug:

- **A safety gate placed in a caller is not a gate.** bug-276 R2 got the
  verify-before-pin *order* right but installed it in `verify_log_consistency`,
  one of the two callers of the pin-advance path, leaving `fetch_checkpoint`
  free to skip it. The comment at `src/cli/pkg.rs:176` even documented the
  hazard — "fetch_checkpoint only enforces monotonicity, so a fork that simply
  grows passes it" — and the response was to pick the safe caller at *that* site
  rather than to fix the primitive. Every later call site then had to re-derive
  the same choice, and `pkg install --proof` got it wrong. The fix is to make
  the unsafe spelling not exist.
- **A correct doc comment that describes a defect is a bug report nobody
  filed.** That comment was accurate for eighteen months. When the fix landed it
  became false, so it was corrected rather than left — a stale "why we avoid X"
  comment is worse than none once X is safe, since it invites someone to
  re-introduce a second unsafe path to avoid.
