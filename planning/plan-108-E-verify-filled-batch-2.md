# plan-108-E: Verify the pre-filled packages, batch 2 — crypto, os, io, process, audio, tls, json, csv, money, regex, app

Last updated: 2026-08-30
Effort: huge (> 3d) — 105 pages across 11 packages plus 54 of the surface's
94 memory-vocabulary rewrites (the tls/process/audio handle prose); split
across sessions by phase
Depends on: plan-108-D (verification batch 1 landed; audit pace and reviewer
calibration proven on 160 pages).

Verify the remaining pre-filled packages — **crypto (17 function pages), os
(15), io (15), process (15), audio (12), tls (10), json (5), csv (5), money
(4), regex (4), app (3) = 105 pages** plus overviews and types pages —
through the same verification cycle as D. This batch carries the plan's one
KNOWN accuracy defect, most of the environment-dependent examples, and
**the largest share of the memory-vocabulary rewrite**.

`tcp` and `udp` were unassigned in plan-108's first draft (A's Corrections,
2026-08-30); they are assigned to **plan-108-C**, alongside `net`. C
therefore lands the network family's handle wording BEFORE this letter runs
— see Prerequisites: `tls` must match what C established, not invent its
own.

See plan-108-A §3 for the workflow and the standard. Per A: verification is
`mfb man` rendering + ad-hoc example/probe runs — no compiler test gates.

References:

- **plan-108-A §3 (2a) — the memory-vocabulary hard ban.** Permitted:
  **copy**, **mutate**, **value**, **alias** (`RES` handles only).
  Banned from rendered output: `borrow`, `pointer`, `ownership`/`owns`,
  `move`, `free`, `heap`, `refcount`, `lifetime`, `deep/shallow copy`,
  `by reference`, `drop` (memory sense) — use A's rewrite table, and link
  `mfb man variable` instead of re-explaining the model on a package page.
  Run `scripts/man-census.sh --memory-scope <pkg>` before closing each
  package; record before/after counts in the ledger.
  **This letter carries the bulk of the violation.** Rendered baseline
  (2026-08-30): tls 23, process 18, audio 10, crypto 1, io 1, money 1 =
  **54**; os/json/csv/regex/app 0. The whole rendered surface holds 94
  memory-sense hits outside the datetime arithmetic carve-out — **54 of
  them (57%) are in this letter**, and C holds the next 26 (net 1, tcp 14,
  udp 11). Budget for it: this is a rewrite pass over the resource
  packages' handle prose, not a spot fix. Named offenders to fix by hand,
  each attributed to its package (line numbers into
  `mfb man --all > /tmp/man-all.txt`, 2026-08-30):
  **tls** `:36160` "The returned Socket is a borrowed pointer — an alias
  into the list" and `:35662` "closing the socket never frees";
  **process** `:12846` "Letting a Process drop at scope exit" and `:12964`
  "not treated as an ownership". Plus **15 of the 25 `Borrowed, not
  consumed` parameter descriptions** — process 7, audio 4, tls 4
  (`mfb man <pkg> --all | grep -c 'Borrowed, not consumed'`); the other 10
  (tcp 6, udp 4) are C's.
- `src/codegen/builtins/{crypto,os,io,process,audio,tls,json,csv,money,regex,app}/`.
- **plan-108-C's net/tcp/udp handle wording** — C fixes 26 memory-sense hits
  across net/tcp/udp before this letter starts. `tls` wraps the same
  socket concepts and must reuse C's sentences verbatim; diff
  `mfb man tcp accept` against `mfb man tls accept` rather than writing new
  prose. This is the cross-package consistency case F sweeps for.
- **Known defect to fix here**: the `process` package prose claiming a
  resource "cannot be stored as a collection element" — WRONG per spec
  §15.6 (`List/Map OF RES …` is valid; ownership floats up); memory
  `resources-in-collections-yes-records-no`. Fix the prose, and record the
  corrected wording in this letter's ledger.
- Memory `mfb-string-escape-is-u-not-x` — `\x{…}` is regex-PATTERN-only
  syntax, not a string escape: the regex pages must state this boundary
  precisely (it is exactly the confusion a developer hits).
- `.ai/resources-packages.md` — resource-internals foil: man pages state
  developer-visible resource lifetime rules only.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-D complete | D's boxes ticked; census 100% | **MET** 2026-08-31 — `./scripts/man-census.sh collections math encoding fs datetime` reports 183/183 on intro, desc and example and **286/286 parameter descriptions**; `--memory-scope` 0 (plus the 15 classified datetime arithmetic borrows) and `--scope` 0. Commits `56f99703d`, `3d5759dfe`, `b6256f0c6`, `4d275306b`, `dcf42e404`, `c5646161a`. |
| `mfb man variable` exists (this letter links it instead of re-explaining) | `mfb man variable` renders | **MET** — delivered by plan-108-A Phase 2b, commit `f816298ea` |
| C's net/tcp/udp handle wording is landed and readable | `mfb man tcp accept`, `mfb man udp receive` | **MET** — landed in `fd8e0473d`/`4a429d828` and recorded as a verbatim table in plan-108-C, "The settled network-family handle wording". `tls` copies rows 1, 2, 3 and 6. |

## 1. Goal

- All 105 pages + 11 overviews + types pages verified claim-by-claim and
  scope-checked; the `process` resources-in-collections defect fixed with
  the corrected wording recorded.
- **`scripts/man-census.sh --memory-scope` reports 0 for every package in
  this letter** (baseline 54; see References). The handle contract each
  rewritten sentence carried is preserved in MFBASIC terms — "stays open",
  "the caller still closes it", "an alias of the one in the list" — not
  deleted. `tls` ends with the SAME wording C gave `tcp`/`udp` for the same
  situation.
- Every example compiled during the pass, and run where the environment
  permits (crypto/json/csv/money/regex are pure — run them; os/process
  where side-effect-safe: env reads, temp-dir spawns; io/audio/tls/app
  compile-only where they need a tty/device/endpoint) — each compile-only
  call noted per function in the ledger.
- Cross-model review (Codex) per package; ledgers recorded here.
- The `errorcode`/`perf` resolution from A executed if A assigned them here
  (whatever pages they own verified the same way, or the out-of-scope
  reason restated).
- Census still 100%; every registry package now authored or verified.

### Non-goals (explicit constraints)

- **No new inline explanation of the memory model.** Any page that needs
  more than one sentence about copies or handles links `mfb man variable`
  (authored in A) — it does not re-explain, and never in C/Rust terms.
- Per plan-108-A (no compiler testing; prose string fields only with
  per-commit `git diff` check; no renderer/schema changes; no
  `package.mfb` edits; `src/docs/man/**` untouched).
- No wording churn on accurate, in-scope prose.
- Found code bugs: fix or file via write-bug, recorded here.

## 2. Current State

A's census: these eleven packages carry desc+example on 105 of ~111 pages
(stragglers were authored in C Phase 3). Same migration-era provenance and
zero prior audit as D's batch. The `process` defect is already known-wrong
by memory + spec cite — it needs the fix, not a re-derivation.

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to verify | 105 (+11 overviews, types pages) | `scripts/man-census.sh` at kickoff |
| known defects entering | 1 (`process` resources blurb) | memory + spec §15.6 |
| compile-only examples | one ledger row per function so classified | this letter's ledger |

## 3. Design Overview

Same per-package cycle as D. Order: process first (carries the known
defect — land the certain fix early), then io, os, crypto, tls, audio, app,
then the small pure four (json, csv, money, regex) as a closing sweep.

**Risk concentration:** example optimism — an example that "runs fine here"
but is environment-fragile (stdin EOF, audio device, tls endpoint). Held
by: compile-only classification for tty/device/endpoint members, recorded
per function — never an unrecorded skip (no silent gaps).

### Rejected alternatives

- **Defer the `process` fix to a bug doc.** Rejected: it is a one-line
  prose correction with the disproof already in hand (spec §15.6); fixing
  in-letter with the ledger entry is the write-bug small-triage path.

## Compatibility / Format Impact

None to codegen/wire. Summary-pin update only if a pinned summary is itself
corrected.

## Phases

### Phase 1 — process, io, os

- [x] Verify **14+15+19** pages + overviews + types pages; the `process`
      resources-in-collections defect fixed (ledger below); io stdin
      examples verified by *running* rather than classified compile-only —
      the harness gained `STDIN_FILE=`, so all 22 run.
- [x] Memory-scope rewrite for `process` (**32** rendered hits, not 18),
      `io` (**9**, not 1) and `os` (**12**, not 0 — the plan was wrong on
      all three; see Corrections). C's net/tcp/udp sentences reused
      verbatim where the situation matched; process-specific sentences in
      the ledger.
- [x] Cross-model review per package + apply; ledgers.
      `os`'s review is re-queued — see Corrections item 4.
- [x] Verify: rendering reads clean; census still 100%
      (14/15/19 pages, 26/7/9 parameters); `--memory-scope` = 0 for all
      three; examples 18/18, 22/22, 21/21.

**Ledger — the known `process` defect (the one this letter entered with).**

> Old: "Like every resource handle it cannot be copied, stored as a
> collection element, or carried in a record; it is closed automatically by
> lexical drop when its binding leaves scope."
>
> New: "Like every resource handle it cannot be copied and cannot be a field
> of a record, but it can be held in a collection (a
> `List OF RES process::Process` is how you supervise several children). It
> closes itself when its binding goes out of scope."

Both halves compiled before the wording was written, and the compiler
supplied the exact spelling — the first attempt said `List OF
process::Process` and `TYPE_RESOURCE_REQUIRES_RES` corrected it to
`List OF RES process::Process`:

```
LET kids AS List OF RES process::Process = []      ' builds
TYPE Holder                                        ' TYPE_RESOURCE_FIELD_FORBIDDEN
  p AS process::Process
END TYPE
```

Cite: spec §15.6; memory `resources-in-collections-yes-records-no`.

**Ledger — process, from the review (16 findings). Two are runtime bugs,
now filed:**

- **bug-474 (HIGH), `detach` breaks `waitFor` for every other child.**
  `process::detach` installs `signal(SIGCHLD, SIG_IGN)`, which is
  program-wide, so a later `waitFor` on an unrelated handle returns `0`
  instead of the real exit code. Reproduced: `shell("sleep 0.1; exit 7")`
  prints `7` alone and `0` once any other child is detached. Documented
  explicitly on both `waitFor` and `detach` until it is fixed.
- **bug-475 (MEDIUM), `waitFor` hangs on an undrained child.** The page
  claimed unread output "is discarded when the pipe buffer fills". It is
  not — the child blocks in `write` and never exits. Reproduced:
  `shell("yes | head -c 1048576")` under a 5s alarm exits 142 with no
  output. The page now tells you to drain first, which is the difference
  between a working program and a hang.

Other process findings applied: the `Signal` platform mapping that three
pages promised "is tabulated in `mfb man process types`" **did not exist** —
the types page renders only one-line variant descriptions. The mapping is
now a real table on the overview (Unix signal / Windows behaviour / what
`didSignal` reads back), and the two other pages point at it. Added the
undocumented negative-`timeoutMs` behaviour of `poll` (verified: waits with
no deadline) and the `ErrResourceClosed` that `poll`/`receive`/`receiveBytes`
raise on a detached handle (verified).

**Ledger — io (16 findings).** Accuracy: `io::print`/`io::write` accept an
`AttributedString` as well as a `String`, which three pages denied
("Only `String` is accepted") — verified by compiling and running
`io::print(astrings::fromString(...))`. `io::pollInput`'s "a `TRUE` result
promises that the next read will not block" is false for `io::readChar` when
only the first byte of a multi-byte scalar has arrived; the page now says
`TRUE` means *one byte* is ready and lists the two cases that still block.
`readLine` was said to read "the same way" as `input` — it does not, it
suppresses terminal echo, which is the whole reason to prefer it for a
passphrase. Added: input ending mid-line returns those bytes as the final
unterminated line (verified). Leakage: every "file descriptor 0/1/2" and
"`isatty` probe" removed.

**Ledger — os.** 12 memory-vocabulary hits, all one pattern: "an owned
`String`" for a value the host copied out. A developer experiences that as
"your own copy, unaffected by later host changes", so `owned` is simply
dropped (`os::hostName` "copied into a `String`"), and the map overview's
"follow the ordinary owned-value rules" became "behave like any other
value". `os::sleep`'s second example was a worker fragment with no `main`
that could not build; it is now a complete program that starts the worker,
cancels it, and prints `cancelled`.

Acceptance: three packages verified; known defect fixed and recorded;
memory-scope 0.
Commit: —

### Phase 2 — crypto, tls, audio, app

The plan's heaviest memory-vocabulary phase: tls 23 + audio 10 + crypto 1 =
34 rendered hits.

- [x] Verify **20+11+11+2** pages + overviews + types pages, **plus
      `canvas`'s 13** — `canvas` was assigned to no letter at all
      (Corrections item 2). Runs recorded per package; the harness gained a
      per-example timeout so a device-blocking example is reported rather
      than hanging the run (Corrections item 5).
- [x] Memory-scope rewrite across tls/audio/crypto **and canvas** to 0.
      `tls` reuses C's `tcp`/`udp` sentences verbatim; `mfb man tls accept`
      and `mfb man tcp accept` now carry the same handle sentence. All three
      named offenders fixed:

      | Was | Now |
      |---|---|
      | "The returned `Socket` is a **borrowed** pointer — an alias of a list element — so the list retains ownership and closes every socket" | "The returned `Socket` is an **alias** of the one in the list — closing it closes that one — so the list still closes every socket" |
      | "closing an accepted socket never frees the shared context, which is owned by the `Listener`" | "The `Listener` holds the server's TLS settings and every accepted `Socket` shares them: closing an accepted socket leaves them intact" |
      | "`close` consumes the `Socket` it is given: the value is moved into the call" | "`close` closes the `Socket` it is given: the handle cannot be used again" |

- [x] Cross-model review + apply; ledgers.
- [x] Verify: rendering + census as Phase 1; `--memory-scope` = 0 for
      tls, audio, crypto, app and canvas; `mfb man tls accept` diffed
      against `mfb man tcp accept` — identical handle wording.

**Ledger — canvas (the unassigned package).** 11 memory-vocabulary hits and
one accuracy defect. `Image` is genuinely a resource, so the rewrite keeps
that and drops only the C/Rust framing: "an owned resource, released when it
leaves scope" → "a resource; it closes itself when its binding goes out of
scope"; "nothing in the installed scene points at anything the caller owns"
→ "the installed scene is entirely its own"; "A handle naming a destroyed
image is not dangling" → "An id naming a destroyed image is harmless: it is
just an integer". The load-bearing fact — a scene holds an image's *id* and
never keeps the image open — is stated more prominently than before, on the
overview, `present` and `imageRef`.

**Accuracy defect (pre-existing, found here).** `present` and
`destroyImage` both claimed "the runtime defers freeing the backing texture
until the GPU has finished with it". No such mechanism exists:
`rg -n IMAGE_DIRTY src/` gives four hits, all writes plus the constant and
an import — nothing reads the flag; `helper_geometry.rs:131,346` return
`__canvas_emptyHeader()` / `[]` for `CASE Picture(pic)`; there is no
texture, atlas or upload anywhere. Destroying is safe for a simpler reason,
which the pages now give: the scene holds the id, not the image. Confirmed
against the plan-98 session, which owns this code.

**Ledger — the shared handle vocabulary (audio, tls).** Both packages spoke
C. `Borrowed, not consumed.` → `The handle stays open — you still close it.`
(12 parameter descriptions across audio, 8 across tls); `Consumed by the
call` → `Closed by this call; the handle cannot be used again.`; "closed
automatically by lexical drop when its binding leaves scope" → "closes
itself when its binding goes out of scope". `audio::close` also carried an
internals line — "IR lowering routes each operand to a distinct
per-direction internal body (`audio.closeInput` / `audio.closeOutput`)" —
now "It accepts either direction, and does the right thing for each — a
capture stream and a playback stream shut down differently."

**Ledger — crypto.** 18 rendered hits, but **9 were not editable prose**:
they are the auto-derived Errors table rendering `ErrOutOfMemory —
"Allocation failed."`, whose text is the runtime's own error string. That is
a census-scope defect, not a content one; see Corrections item 3. The 9
authored lines were fixed ("validated before any allocation" → "The check
happens before any work is done"; "the internal entropy scratch buffer is
zeroed, so no later allocation can observe the generated bytes" → "the
working copy of the entropy is wiped, so nothing later in the program can
read the generated bytes back out of it"; "a fast, seedable PCG64 generator"
→ "a fast, seedable random sequence").

Acceptance: four packages verified and reviewed; memory-scope 0; tls handle
wording matches C's tcp/udp.
Commit: —

### Phase 3 — json, csv, money, regex (+ errorcode/perf per A's ruling)

- [x] Verify **4+4+3+4** pages + overviews + types pages; regex `\x{…}`
      pattern-vs-escape boundary stated precisely; A's errorcode/perf
      assignment executed (`errorCode` 1 hit fixed, `perf` already 0).
- [x] Memory-scope: `money` 1, `json` 1, `regex` 2, `errorCode` 1 — all
      fixed; `csv` and `perf` already 0, confirmed.
- [x] Cross-model review + apply; ledgers.
- [x] Verify: rendering + census as Phase 1; `--memory-scope` = 0; the
      package-coverage cross-check is recorded below — and it found a gap.

**Ledger — the `\x{…}` boundary, stated from a probe rather than from
memory.** The regex overview said only that "a backslash the regex needs is
written `"\\"` in a source literal". That understates it, because the
failure is *silent*. Measured:

```
LET naive AS String = "\x{41}"     ' len 5  -> x{41}
LET pat   AS String = "\\x{41}"    ' len 6  -> \x{41}
regex::match("A", pat)   = TRUE
regex::match("AA", naive) = FALSE
```

`\x` is not an MFBASIC escape (the Unicode escape is `\u{…}`), so the
backslash is dropped and the pattern silently becomes the **quantifier**
`x{41}` — "41 letter `x`s". The overview now shows both spellings, both
outcomes, and the rule ("one backslash in the *pattern* is two in the
*source*"; a pattern read from input needs no doubling).

**Small-package memory hits, all one line each.** `json::parse` "does not
consume a native stack frame" → "does not use a stack frame";
`money::round` "settles a line item or an allocation" → "or a share";
`regex::findAll` "a position already consumed by the previous match" → "a
position the previous match already covered"; `regex::replace` "An unbraced
reference consumes the longest valid run" → "takes the longest run of digits
it can"; `errorCode` "no conversion and no allocation" → "with no conversion
at all".

**Package-coverage cross-check — this is where the gap was found.**
`ls src/codegen/builtins/ | grep -v '^mod.rs' | wc -l` reports **31**, not
the 30 the plan assumed, and the extra one is `canvas`, which appeared in no
letter's package list (`grep -l canvas planning/plan-108-*.md` returned
nothing). Absorbed into Phase 2 above and completed there. All **31**
packages are now covered:

| Letter | Packages |
|---|---|
| B | astrings, bits, general, strings, term, testing, thread, vector |
| C | http, net, tcp, udp |
| D | collections, datetime, encoding, fs, math |
| E | app, audio, **canvas**, crypto, csv, errorcode, io, json, money, os, perf, process, regex, tls |

Acceptance: all remaining packages verified and reviewed; the **31**-package
coverage cross-check recorded here.
Commit: —

## Validation Plan

- Verification: `mfb man <pkg> --all`/`types` per package; census still
  100%; examples/probes compiled and (where possible) run ad hoc; the
  ledger has a row for every compile-only example (no silent gaps).
- Doc sync: none beyond content.
- Hygiene: fmt at session end.

## Open Decisions

- None entering — classification calls are made and recorded in-phase.

## Corrections

1. **Every memory-hit count in this letter was wrong, all in the same
   direction.** Measured with `./scripts/man-census.sh --memory-scope <pkg>`
   at execution time:

   | Package | Plan said | Actually | |
   |---|---|---|---|
   | `process` | 18 | **32** | |
   | `io` | 1 | **9** | |
   | `os` | 0 ("confirm, do not churn") | **12** | the plan told this letter to skip a package that needed twelve fixes |
   | `tls` | 23 | **49** | |
   | `audio` | 10 | **25** | |
   | `crypto` | 1 | **18** | 9 of them not editable, see item 3 |

   Phase totals corrected in place. The `os` row is the instructive one: a
   "already 0 — confirm" instruction, trusted, would have shipped twelve
   defects.

2. **`canvas` was assigned to no letter.** The plan's closing cross-check
   asserted 30 registry packages; `ls src/codegen/builtins/ | grep -v '^mod.rs'
   | wc -l` says **31**. `canvas` — 13 pages, 11 memory-vocabulary hits and a
   false claim about deferred texture frees — was in no letter's list. Added
   to Phase 2 and completed. The cross-check in Phase 3 is now written out as
   a per-letter table so the next reader can re-derive it rather than trust a
   number.

3. **`--memory-scope` counted auto-derived Errors-table rows.** The Errors
   table is generated from the `errorCode` constant descriptors, and
   `ErrOutOfMemory`'s `message` is the string the runtime prints when the
   error is raised (`_mfb_str_error_allocation`). It reads "Allocation
   failed.", so every page that can raise it showed the banned word
   `allocation` in a cell no page author can edit — and editing it would
   change program output and drift goldens, which this plan is barred from
   doing. Added as **carve-out 2**, classified and counted separately exactly
   like the datetime arithmetic borrows, never silently dropped. 20 rows
   across the surface.

4. **The `os` cross-model review did not complete** — the reviewing model
   returned `You've hit your usage limit ... try again at 3:34 AM` partway
   through, after ~129k tokens. `process` (16 findings) and `io` (16) both
   completed and are applied. `os`'s content was independently verified here
   (12 memory hits fixed, all 21 examples run, `--fill` 19/19/19/19 9/9); the
   review is re-queued rather than counted as done, and is tracked in F.

5. **This letter's content changes landed inside plan-108-D's commit.**
   `734646ce6` was staged with `git add -A` while E's `process`, `io`, `os`,
   `crypto`, `tls`, `audio` and `canvas` edits were already in the working
   tree, so that commit carries both letters' package changes (141 files)
   even though its message describes only D's five packages. `git show --stat
   734646ce6` shows the E packages in it. Not amended — the hash is already
   recorded on D's three phases, and amending would invalidate it. E's
   remaining edits and both plan files land in the following commit. The
   lesson is the obvious one: stage by path when two letters are in flight.

6. **The example harness had three defects, all of which made a broken
   example look fine.** Two are recorded in plan-108-D's Corrections (working
   directory, documented-failure exits). The third surfaced here: **no
   per-example timeout**. An `http` example blocked a run for ~3 hours while
   looking identical to "still working", and `audio`'s device-opening
   examples did the same. `RUN_TIMEOUT` (default 60s) now bounds every
   example and reports a timeout as the failure it is. macOS has no
   `timeout(1)`, so the harness backgrounds the child against a
   `sleep N; kill -9` and maps 137 → 124.

## Summary

The verification close-out: every remaining migrated-prose package audited,
the one defect we already knew about fixed with its disproof cited, and
every example in the registry finally compiled — leaving F to certify the
whole surface and retire the dead tooling.
