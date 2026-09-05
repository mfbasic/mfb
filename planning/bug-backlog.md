# Open bug backlog — triage and work order

Last updated: 2026-09-04
Open bugs: **45** (`find bugs -maxdepth 1 -name 'bug-*.md' | wc -l`)
Severity split: **0 CRITICAL · 10 HIGH · 31 MEDIUM · 4 LOW/other**

The audit-3 security pass (goal-08) is done: 19 of its 20 CRITICAL/HIGH findings
are landed and archived; 499, 504 and 510 are the remainder and head this list.

## Working rules for this pass

- **Model:** fable for CRITICAL, opus for everything else. There is currently
  **no open CRITICAL**, so every dispatch below is opus.
- **Concurrency:** one background agent at a time, plus the lead working a
  second bug directly.
- **Landing:** commit and merge each bug as it completes — never batch.
- **Memory bugs carry an extra gate.** For any bug touching allocation,
  aliasing, ownership or drop (`487`, `536`, `538`, `479`, and the landed
  `495/496/497/498`), it is not enough that the tests pass: the fix must be shown
  **not to change the memory semantics of the language**. Required evidence, per
  the pattern established on bug-496/497:
  1. the RED test flips green;
  2. a *correct* program's observable behaviour is unchanged — name the
     documented contract in `.ai/collections.md` / `mfb spec` §14 the fix now
     realizes, and show the fix only ADDS a check/copy/lock rather than altering
     a value's lifetime or identity;
  3. the artifact gate's golden delta is confined to the emitting fixtures, with
     everything else byte-identical (that containment IS the semantics proof);
  4. a positive pin, not just the negative one — e.g. bug-497's
     `every_byte_list_producer_still_passes_the_write_header_check`, which
     caught that a new guard could reject valid programs.

## Tier 1 — finish audit-3 (in flight)

| Bug | Sev | Effort | Title | State |
|---|---|---|---|---|
| 510 | HIGH | medium | text-decoder DoS cluster (regex/json/csv/punycode) | agent running; also owes a verdict on a corruption lead |
| 499 | HIGH | medium | spawned child inherits fds (no CLOEXEC) | worktree has partial work; lead taking it |
| 504 | HIGH | medium | emitted PE has no ASLR (`.reloc`, DYNAMIC_BASE) | worktree has the RED test only, no fix |

## Tier 2 — HIGH, memory and correctness first

| Bug | Sev | Effort | Title | Note |
|---|---|---|---|---|
| 538 | HIGH | medium | `collections::get` of a recursive element aliases storage → append-grow UAF | **memory gate**; pairs with 536 |
| 536 | HIGH | x-large | scope drop leaks recursive types / return-constructor string temps | **memory gate**; same family as 538 |
| 514 | HIGH | large | `KeyPair` carries no curve tag (Ed vs X, 32-byte collision) | crypto: wrong-curve use is silent |
| 532 | HIGH | x-large | regex reports only match starts, so extraction is impossible | API gap, large |
| 539 | HIGH | large | GTK app-mode draws no positioned `term` output | platform |

535 is landed (`b93de7ed0`). Recommended order: **538 → 536** (one family,
cheaper together), then **514**. 519 is landed. The two x-large items (532, 536) deserve a dedicated agent
rather than sharing a slot.

## Tier 3 — MEDIUM, grouped so a single agent can take a cluster

**Regex/strings semantic divergence** (one coherent agent task):
529 (empty needle means four things) · 531 (absence: raise vs sentinel) ·
533 (empty-pattern replace is opposite) · 534 (no split/count/AttributedString) ·
528 (`pad` counts scalars, `displayWidth` counts columns) ·
530 (`utf8Encode` return overload invisible in signature)

**Resource / close contracts** (one agent):
524 (`process::close` is the only close that does not close) ·
525 (tcp/tls diverge on double-close and backlog) ·
522 (stale transferable list) · 523 (RES type pages omit shapes) ·
526 (`tls::poll` list overload renders without RES)

**App backends** (one agent): 540 (Win app term reduced) ·
541 (backends do not enforce the inactive-term gate)

**datetime**: 518 (withZone description contradicts the function) ·
521 (`toIso` claims a round-trip it truncates) · 520 (no named zones — huge)

**crypto**: 515 (no memory-hard password KDF) ·
517 (SHA-1 advisory cannot tell hashing from HMAC) ·
511 (X25519 secret-dependent branch)

**registry / supply chain** (audit-3 MEDIUM carryover):
489 (response terminal injection) · 490 (client redirect credential leak) ·
491 (`pkg install` not bound to the lock)

**Older carryover**: 453 (riscv64 jal range) · 454 (win64 `os::resourcePath`) ·
479 (inline TRAP on thread start — **memory gate**) · 483 (tls write error code
per backend) · 484 (`picture::drawItem` never renders) · 487 (state-mutating
operand UAF — **memory gate**) · 527 (range parameter naming, large)

**Resource bookkeeping holes found by bug-535's sweep** (both hidden by the same
"any other call into the package" condition, both reproduce on `4d56f1a1a`):
545 (alias rebind of a tcp/udp socket → missing `_mfb_str_error_resource_closed`
data object) · 546 (`thread::accept` of a user-declared `THREAD_SENDABLE`
resource → `native inlined field size not available`; shares a message with 479)

## Tier 4 — test-infrastructure flakes (cheap, and they are costing us now)

| Bug | Sev | Effort | Title |
|---|---|---|---|
| 537 | LOW | small | `rt_macos_tls_write_capacity` fixed port + sleep readiness |
| 488 | LOW | small | `rt_tls_connect_allow_self_signed` port gate is per-process |
| 470 | MED | small | artifact-gate and test-accept do not lock against each other |
| 456 | LOW | small | `mfb opt` sweep level-variant ncode goldens |
| 472 | MED | small | man examples are never compiled |

**These are worth doing early despite being LOW.** 488 and 537 produced false
reds on four separate suite runs during the audit-3 fix pass, every time two
`cargo test` runs shared the machine — which is exactly the agent-plus-lead
setup this backlog prescribes. 470 is the same class (two harnesses that do not
lock against each other). Each is <1h and each removes a recurring
misdiagnosis risk from every later bug.
