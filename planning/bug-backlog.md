# Open bug backlog — triage and work order

Last updated: 2026-09-05
Open bugs: **21** (`find bugs -maxdepth 1 -name 'bug-*.md' | wc -l`)
Severity split: **0 CRITICAL · 1 HIGH · 18 MEDIUM · 2 LOW/other** (re-derived from
each open bug's `Severity:` line on 2026-09-05; several rows carry a
parenthetical qualifier after the word, so grep for the leading word, not the
whole line)

The audit-3 security pass (goal-08) is **complete**: all 20 of its
CRITICAL/HIGH findings are landed and archived — 499, 504 and 510 were the last
three and are all in `bugs/completed/`.

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

**Regex/strings semantic divergence**: 529, 531 and 533 are **landed**
(`2860dd7e7`, `5e93d26a3`, `426660224`). Remaining: 534 (no split/count/
AttributedString — `split` unblocked by 532) · 528 (`pad` counts scalars,
`displayWidth` counts columns) · 530 (`utf8Encode` return overload invisible in
its signature).

Two things from that cluster worth carrying forward:

- **531 and 533 are BREAKING**, both on the owner's own recorded decision
  (`ded34df72`). `regex::find` now raises `ErrNotFound` instead of returning `-1`,
  and the return type did NOT move — so an unmigrated caller still compiles and
  fails at run time. Product code had zero call sites; both migrations are on the
  member's page.
- **533 turned a doc-shaped change into a MISCOMPILE**, and it is the second
  instance of a known trap. `strings::replace` and `collections::replace`
  dequalify to one bare native target `replace`, which sat on
  `inline_builtin_is_infallible`'s NAME-keyed list. Once the `String` overload
  could fail, an inline `TRAP` on it compiled with
  `TYPE_INLINE_TRAP_DEAD_HANDLER` and the live handler was ELIDED — the program
  aborted instead of recovering, and a function-level `TRAP` test cannot see it.
  Reproduced independently while reviewing: reverting the fix aborts the fixture
  with `7-705-0002`. `toString` was the first instance (bug-486). **Before making
  any overload of a shared bare native target fallible, check that list.**

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

**App backends** (one agent): 540 (Win app term reduced) ·
541 (backends do not enforce the inactive-term gate)

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

**Older carryover**: 453 (riscv64 jal range) · 454 (win64 `os::resourcePath`) ·
479 (inline TRAP on thread start — **memory gate**; **three of its four defects
are landed** in `a4a9d59dc`, and it is now ONE decision: the `TRAP` error path
has no safe default `Thread` value. A resource gets a CLOSED record so operations
short-circuit; `simple_thread_handle_helper` `pthread_mutex_lock`s the queue
pointer off the handle with no null guard, so a null handle AND a zeroed block
both fault, and `THREAD_STATE_CLOSED` cannot help because the lock precedes the
state read. Answering it means a runtime contract across every `thread` member
with a user-visible error code — a product decision, not a codegen arm) · 483 (tls write error code
per backend) · 484 (`picture::drawItem` never renders) · 487 (state-mutating
operand UAF — **memory gate**) · 527 (range parameter naming, large)

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
