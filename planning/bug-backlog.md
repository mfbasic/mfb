# Open bug backlog — triage and work order

Last updated: 2026-09-06 (second refresh)
Open bugs: **7** (`find bugs -maxdepth 1 -name 'bug-*.md' | wc -l`)
Severity split: **0 CRITICAL · 1 HIGH · 5 MEDIUM · 1 LOW–MEDIUM**

The audit-3 security pass (goal-08) is **complete**.

## 2026-09-06 — what landed

Archived on 2026-09-06 (`git log --since=2026-09-06 --diff-filter=R --name-status`):
**456, 479, 487, 516, 527, 543, 550, 551, 552, 553, 554, 555, 556**, plus **488**
closed on evidence rather than a fix. **557** landed the same day from a peer.
**558** was filed (the `mfb man` Errors table unions every overload's errors).

Four user rulings are recorded **in the bug docs themselves** under a
"USER DECISION" heading — 543, 550, 527 and 515. Three are landed; 515 is the
only one still open, and its ruling is: ship both an explicit-cost and a profile
overload, with **the profile calling the explicit one underneath** so there is one
validation site and one use site.

### Two corrections worth carrying forward

**bug-479's "defect D is a product decision" was wrong**, and the same shape may
recur. Every thread op already raised `ErrResourceClosed` on
`THREAD_STATE_CLOSED`, so there was no contract to invent; what blocked it was
ORDERING. Before recording a defect as needing a product decision, check whether
the behaviour is already implemented somewhere and only unreachable.

**bug-553's "fs, process, udp are the model" was wrong.** Measured, none of them
is: `fs` 40 empty / 1 non-empty, `process` 15 / 0, `udp` 9 / 1. When a doc names a
sibling as the standard to copy, verify the sibling first — 21 of 32 packages
carry at least one `errors: vec![]`.

## What is left, and why each is not trivial

- **536** (HIGH) — shape B-2 and shape C. **Shape C is not a bug fix**: a
  recursive-type value is never freed, and the fix needs recursive
  copy-insertion, which does not exist. Do not dispatch it as one. Shape B-2 (a
  `String` returned by a user/`.mfb`-bodied function) is a **double-free** risk
  and wants its own change and audit.
- **484** — `canvas::Picture` never renders on any backend; x-large, and no
  renderer has a picture arm at all.
- **520** — named time zones; huge, and it is a data + serialization design
  question, not a bug.
- **540** — the Windows app `term` is a reduced implementation. Note nothing in
  this repo ever EXECUTES a Windows binary, so it cannot be verified here the way
  543 was verified on four Linux boxes.
- **472** — man examples are never compiled. Carries an explicit user decision
  AGAINST building the gate (plan-108-A rejected-alternatives), so it needs a
  ruling before work, not after.
- **515** — has its ruling; in flight.
- **558** — the man Errors table; small, but the LAYOUT is a product choice with
  three options written up.

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

## Tier 1 — audit-3: DONE

Nothing open. 499 (spawned child inherits fds), 504 (emitted PE has no ASLR) and
510 (text-decoder DoS cluster) are landed and archived, as are 514, 535, 538,
539 and 545. **Do not re-dispatch these** — the table that used to sit here said
"agent running" for all three and was stale for a full session.

## Tier 2 — the one remaining HIGH

| Bug | Sev | Effort | Title | Note |
|---|---|---|---|---|
| 536 | HIGH | large | scope drop leaks: shapes **B-2** and **C** remain | **memory gate** |

**536 is the only open HIGH.** Three of its four parts are done:

- **Shape A** — `RETURN <constructor>` abandoned the fresh block. Fixed
  `f9be6e128`, merged `c210cc67d`.
- **Shape B, native half** — an unbound `String` from a *native* producer was
  never freed (`acc = acc + len(toString(i))` leaked 64 B per evaluation). Fixed
  `cd8699103` by **fail-closed freshness provenance**: a producer that just
  `arena_alloc`ed the block it returns marks it, and `register_pending_temp`
  frees a bare `String` only on that mark. Unmarked keeps leaking, never wild-frees.
  Golden delta was 142 `.ncodesum` + 4 `.ncode` + 1 `.mir` and **zero**
  `.run`/`build.log`.
- **Shape B-2, callee half** — a `String` returned by a user / `.mfb`-bodied
  function. **Open**, and it is what still costs the decoders: `csv::parse` is
  byte-identically unchanged by the native fix. The bug doc's old claim that "csv
  has a SECOND leak that is NOT shape B" is **wrong** and now corrected there — it
  is shape B one level up (`row = append(row, __csv_fieldValue(...))`). Needs a
  transitive `function_returns_fresh_string` NIR predicate; it is a
  **double-free** risk, not a leak risk, so it wants its own change and audit.
- **Shape C** — a value of a recursive type is never freed. **Blocked**: it needs
  recursive COPY-insertion, which does not exist, and the naive fix is a double
  free. It is a design pass, not a bug fix. Do not dispatch it as one.

514 is landed. 519, 532 and 535 are landed; 538 is landed and 539 also fixed a
pre-existing GTK draw-callback SIGSEGV and put the Linux GTK app backend under
byte-identity coverage for the first time. 532 **unblocked bug-534's `split`**.

## Tier 3 — MEDIUM, grouped so a single agent can take a cluster

**Regex/strings surface: the whole cluster is LANDED.** 529/531/533
(`2860dd7e7`, `5e93d26a3`, `426660224`), then 530 (`f75616ed2`), 528
(`f071d0f45`) and 534 (`fe7903170`).

**Three findings from it outlive the bugs:**

- **A THIRD dead-handler miscompile**, and a different shape from the first two.
  `strings::left`/`right`/`padLeft`/`padRight` raised `ErrInvalidArgument` while
  declaring `errors: vec![]`, so `inline_builtin_is_infallible` proved them
  infallible, the compiler warned `TYPE_INLINE_TRAP_DEAD_HANDLER`, **deleted the
  live handler**, and the program aborted with `7-705-0002`. Fixed `6d9f4b79a`.
  bug-486 and bug-533 were name-keyed-over-an-overload; this one is simply a
  member lying in its descriptor. **A function-level `TRAP` test cannot see any
  of the three — write the INLINE form.**
- **The invariant that should have caught it never runs** — filed as **bug-550**.
  `raise_error_bare`'s declaration check is a `debug_assert!`, and CI builds
  release on every job, so all **55** `debug_assert!`s in the tree are compiled
  out everywhere. Needs a decision: a debug-assertions CI job, or promoting the
  miscompile-guarding ones to real `assert!`.
- **Two branches editing one package both shift its embedded line numbers**, so
  neither parent's `.ir` goldens are right for the merged tree. 528 and 534
  collided on exactly that; resolving the conflict by picking a side would have
  produced goldens matching neither compiler. Regenerate post-merge — the gate
  named the two affected fixtures precisely (2 of 1932).

**Resource / close contracts** (one agent): the cluster is **complete** — 524,
525, 526, 522 and 523 are all landed.
**522 is landed (`09453380a`)** — the `transfer` page's list was stale since
bug-464; `transfer`, `accept` and the intro now agree with the registry and all
three name the five resources that may not cross. A pin asserts every `sendable`
bit against an explicit table.
**523 is landed (`593d68965`)** — each `types` page states the record-field and
collection-element shapes once and derives transferability from the `sendable`
bit (a new `unsendable_reason` carries the per-resource reason); `mfb man
variable` gained a runnable example of each shape. It also corrected two FALSE
statements — `process`'s package description and `mfb spec stdlib transports`
both said a handle may not be a record field, which §15.4 and
`record-res-field-export-rt` refute.

Follow-ups these left behind, both already-filed bugs rather than new ones:
`scripts/man-run-examples.sh` reaches package pages only, so `mfb man variable`'s
new examples were verified by hand (**bug-472**); and `audio::close`'s new
raise-on-double-close has no runtime proof on any host for want of a device.
**524 is landed (`be88539c8`)** — `process::close` is now `process::closeInput`;
the behaviour did not move and zero `.run` goldens changed. It also leaves 523 a
concrete correction: `process`'s package description claims a handle "cannot be a
field of a record", which is false.
**525 is landed (`9053e5eb0`)** — every built-in `close` now refuses an
already-closed handle with `ErrResourceClosed`, and both `listen` members default
`backlog` to `128`. It was never a decision waiting to be made: `mfb spec language
resource-management` §15 already required the raise, so `tls` and `audio` were the
non-conforming side and `src/docs/spec/stdlib/17_transports.md` was the stdlib spec
contradicting the language spec. Proven before/after on all three TLS backends
(macOS Network.framework, box 2228 OpenSSL, box 2230 Schannel); `audio` has no
runtime proof anywhere (no device) and rides a lowering pin.
**526 is landed (`521b731e0`)** — `mfb man tls poll` printed a signature that did
not compile. A registry-wide renderer pin now fails any rendered signature whose
collection element is an unmarked resource; it found exactly one violation, so
the sibling census is complete.

**App backends**: **541 is landed (`0ba90b19f`)** — all three gates, with a real
runtime RED on box 2230 (`gate01=size80` -> `gate01=raised` against the console
oracle). **540 is PARTIAL (`a02bd12df`)**: WIN-01 and WIN-05 landed, WIN-02/03/04
remain and are root-caused in the doc. WIN-05 was a NEW find while fixing 541 —
Windows `term::on` reset 3 fields where every other backend resets 7.

**WIN-02/03 are blocked on an INSTRUMENT, not on work.** Headless Windows never
reaches `WM_SIZE`, so proving a resize needs a `term::` twin of
`MFB_CANVAS_RESIZE_W/_H`. That adds test-only surface to the product, so it is an
Open Decision. WIN-04's correct fix is sharing `emit_app_io_write`'s ~430-line
cluster walk, whose labels are untagged and would collide.

**A THIRD bug was found here and it is the one worth remembering.**
`scripts/test-winapp.sh` named `canvas::rgb`, which plan-122-D's canvas migration
removed the same day. Under `set -e` that is a hard stop at the BUILD, so 17
assertions — the canvas frame, the entire Vulkan section, both resize runs —
exited "passing" without executing. This is the ONLY instrument in the repo that
runs a Windows binary at all, so its silence removed the sole runtime coverage
for a whole platform. Verified independently: `canvas::rgb` does not exist.

That makes three findings this session of the same shape — **a harness and the
code it exercises drift, and the harness reports success**: bug-470's guards
covered 3 of 11 writers; `curve448_secret_paths_are_branch_free` covered one of
two fields; this script covered one of two halves of itself. None FAILED. Each
passed while checking less. When a rename lands, grep `scripts/` for the old
spelling — nothing else will tell you.

**datetime**: 520 (no named zones — huge; carries interaction notes from the
three landed siblings). 518, 519 and 521 are landed.

**crypto**: 515 (no memory-hard password KDF) is the only one left. **511 is
landed (`e24914d5d`)** and **517 is landed (`5c2024f71`)**.

**517 was the one backlog item blocked on a ruling, and the owner gave it**: the
SHA-1 advisory is scoped to the USE — `hash` warns, `hmac`/`hkdf`/`pbkdf2` do
not. That reverses `plan-109-A:92` ("regardless of which public function consumes
the selector"), so the plan is left unedited as a record of what was decided
then and the reversal lives in the bug doc and in
`ir::verify::values::hash_selector_use_is_sound`.

Two things from it that generalize:

- **Suppression is fail-closed, and that was a choice inside the ruling.** The
  ruling names members that should not warn; implementing it as "fire only at
  `hash`" would make silence the default. Implemented the other way round, so a
  local-bound selector, a `MATCH` literal, a bare occurrence and a value nested
  inside the argument all still warn.
- **A builtin member has TWO call-target spellings in IR** and only one is
  obvious. `crypto::hash` arrives dotted (`crypto.hash`); the `.mfb`-bodied
  `hmac`/`hkdf`/`pbkdf2` arrive as `#crypto_hmac`, the `internal_name::internalize`
  form of the package's own `__crypto_hmac`. Matching only the dotted name
  suppressed NOTHING and looked correct. Any future predicate keyed on a call
  target must accept both, via the mangling contract.

**511 left a follow-up worth doing** (a test-coverage gap, not a defect). The
report named ONE secret-dependent branch; there were two —
`__crypto_pack25519` also branched on the borrow out of its trial subtraction,
and one of the values packed there is the X25519 shared secret. It survived
because `curve448_secret_paths_are_branch_free` enforced the property **for the
448 field only**, while the constant-time primitive and its packer already
existed. Same "two lists" shape as bug-470 and bug-533. The durable fix is a test
that ENUMERATES the curve fields and asserts a branch-free secret path for each,
so adding a curve fails until it is covered.

**registry / supply chain** (audit-3 MEDIUM carryover):
489 (response terminal injection) · 490 (client redirect credential leak) ·
491 (`pkg install` not bound to the lock)

**Older carryover**: **453, 454 and 483 are landed** (`f332f18e6`, `94b2ec1e1`,
`7b0ab81be`). Remaining: 479 (inline TRAP on thread start — one decision left,
see Tier 2) · 484 (`picture::drawItem` never renders — x-large, sequenced AFTER
plan-116-I) · 487 (state-mutating operand UAF — **memory gate**) · 527 (range
parameter naming, large) · 515 (memory-hard password KDF) · 520 (named zones,
huge) · 472 (man examples never compiled — **blocked on a user decision**
recorded in plan-108-A) · 543 (spawn fd parity — **the owner has ruled**; Linux
is settled, the macOS mechanism is the open question) · 488 (deliberately open
pending a long clean period).

**Newly filed today, all found while fixing something else — none is a
regression:** 550 (55 `debug_assert!`s that never run, because CI builds
release) · 552 (riscv64 linker quadratic, unreachable until 453 removed the
ceiling above it) · 553 (28 `tls`/`tcp` members declare `errors: vec![]`).

**The lesson the cross-platform cluster paid for: name the instrument.** All
three needed something the artifact gate structurally is not.
- **453** — the gate reported 1930/0, *identical to the untouched baseline*, and
  that zero IS the containment proof, because relaxation is a no-op in range. It
  says nothing about execution; only **box 2229** (real riscv64) shows a relaxed
  five-rung chain runs.
- **454** — proved by a **negative control** on box 2230: with the separator left
  POSIX, cross-build and gate stay GREEN while the program fails on Windows.
  That demonstrates the instrument gap instead of asserting it, and is the
  cheapest way to prove a per-platform fix is the fix.
- **483** — the Windows row of its matrix had only ever been READ from source;
  measuring it changed the answer. Its own doc's proposed macOS design turned out
  to be a use-after-free, found by running it, not by reading it.

**And the gate lock (bug-470) refused one of MY runs**, correctly: a subagent's
`test-accept.sh` held the tree lock, my gate exited 98 having checked nothing,
and `DIFFS=0` next to a refusal would have read as success if refusal shared exit
code 1 with "found diffs". The distinct code is what made it detectable. The same
collision hit the agent minutes earlier as 4 phantom mismatches in fixtures the
bug does not touch — same cause, one unmistakable outcome and one plausible wrong
one.

**Resource bookkeeping holes found by bug-535's sweep** (both hidden by the same
"any other call into the package" condition, both reproduce on `4d56f1a1a`):
545 and 546 are both landed. **546 (`6da957747`) is worth reading before any
codegen work that classifies a type**, because its root cause generalizes: every
`codegen::builtins::is_resource_type` / `is_thread_sendable_resource_type` call
answers for the BUILT-IN registry only, and a user-declared `RESOURCE` fell
through to a default of `true` for both flatness modes — "this handle is a flat
copyable block that may be relocated into another thread's arena". Use the
model-aware `is_resource_nominal` / `is_sendable_resource_nominal` instead. The
same blind spot had a SECOND consumer (`defer_resource_flag`), which meant
bug-425's guarantee never held for user resources; **479 is the remaining bug
that shares 546's error message**, so read them together. The invariant is
recorded in `.ai/resources-packages.md`.

## Tier 4 — test-infrastructure flakes (cheap, and they are costing us now)

| Bug | Sev | Effort | Title |
|---|---|---|---|
| 488 | LOW | small | `rt_tls_connect_allow_self_signed` port gate is per-process |
| 456 | LOW | small | `mfb opt` sweep level-variant ncode goldens |
| 472 | MED | small | man examples are never compiled |

537 is landed. **The rest are worth doing early despite being LOW.** 488 and 537
produced false reds on four separate suite runs during the audit-3 fix pass,
every time two `cargo test` runs shared the machine — which is exactly the
agent-plus-lead setup this backlog prescribes.

**470 is landed (`fea98e3cb`)** — and it turned out to be three fixes, not one.
The prior branch's per-tree `mkdir` lock was the right mechanism; what was
missing was everything around it. Both halves of the defect were REPRODUCED
(the doc had said "inferred, not reproduced"): pre-fix, an `artifact-gate` ran
to completion in the same tree as a live `test-accept`, and a `test-accept` in
one worktree refused one in another. The fix itself leaked the lock on
`test-accept.sh`'s SUCCESS path (`trap` replaces, it does not chain — only
INT/TERM survived, so a killed run released correctly and only success leaked).
And the lock covered 3 of the **11** scripts that rewrite fixture dumps in-tree;
regenerate-then-gate is a normal workflow, so the `regen-*` scripts mattered.
The transferable lesson is in `tests/gate_lock_covers_every_writer.rs`: a
recogniser for "which scripts contend" was written three times and
under-reported every time, so it is now an exhaustive classification with a
blindness guard. 488, 456 and 472 remain; each is <1h and each removes a
recurring misdiagnosis risk from every later bug.
