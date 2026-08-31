# plan-108-C: Author the empty packages, batch 2 — net, http, general, astrings, vector — and verify the network family (tcp, udp)

Last updated: 2026-08-30
Effort: large (3h–1d) — grew with tcp/udp (19 pre-filled pages to verify)
and 33 memory-vocabulary rewrites, 26 of them in the network family whose
shared handle wording this letter now sets for the whole plan
Depends on: plan-108-B (batch-1 landed; the workflow has now run on 3
packages beyond the pilot — any standard amendments from B are in).

Author the remaining all-empty packages — **net (23 function pages), http
(19), general (18), astrings (18), vector (17) = 95 pages** plus overviews
and types pages — through the plan-108-A four-step workflow. Also close out
the ~10 empty straggler pages A's census found inside otherwise-filled
packages (exact list from the census re-run; recorded here at kickoff).

**Plus the network family's two already-filled siblings: `tcp` (11 function
pages) and `udp` (8).** These were assigned to no letter in plan-108's
first draft (A's Corrections, 2026-08-30). They belong with `net`: `net`
supplies the addresses `tcp`/`udp` take and return, the three read as one
family, and the handle prose is near-verbatim shared. Unlike this letter's
other packages they are **fully filled** (tcp 11/11, udp 8/8 desc+example —
`mfb man <pkg> --all | grep -cE '^Description$'` minus the overview's own
Description heading), so they get D/E's **verification** cycle, not
authoring — same four steps, different starting point.

This makes C the letter that **sets the handle wording for the whole
plan**: `tls` (E) and `http` (here) both wrap these sockets and are
required to reuse C's sentences rather than invent their own.

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
  Rendered baseline (2026-08-30, same command as A's population table):
  **tcp 14, udp 11**, http 5, net 1, astrings 1, vector 1, general 0 = 33
  — the second-largest concentration in the plan after E's 54.
  `astrings`'s hit is `fromString builds … a deep copy of text` → "builds
  its own copy of text" (A's rewrite table). `tcp`/`udp` carry **10 of the
  surface's 25 `Borrowed, not consumed` parameter descriptions** (tcp 6,
  udp 4 — `mfb man <pkg> --all | grep -c 'Borrowed, not consumed'`) and
  `tcp` owns the "close moves the value into the call" line
  (`mfb man --all` `:36828`, 2026-08-30). `http` is the one to watch on the
  authoring side: it wraps `tcp`/`tls` handles, so it is where borrow prose
  would be invented from scratch — reuse the tcp/udp sentences this letter
  settles, say **alias**, or link `mfb man variable`.
- `src/codegen/builtins/{net,http,general,astrings,vector}/` — descriptor
  prose fields being filled; `src/codegen/builtins/{tcp,udp}/` — filled
  prose being verified.
- `planning/old_man/builtins/**` — source material (claims re-verified,
  citations stripped).
- `.ai/net-tls.md` — the INTERNALS doc for net/TLS; useful to the author for
  verifying claims, and the canonical example of content that must NOT leak
  into man prose (readiness/timeout machinery is spec/internals; the man
  page states developer-visible timeout/error behavior only).
- `.ai/resources-packages.md` — the RES internals foil for tcp/udp socket
  and listener prose: man states developer-visible open/close/alias rules
  only.
- Memory `editing-package-mfb-drifts-many-goldens` — http/net have MFBASIC
  `package.mfb` bodies whose line numbers feed embedded ErrorLoc goldens;
  prose fields in `mod.rs`/func files are fine, but NEVER touch
  `package.mfb` files in this plan (out of scope, and a golden-drift event).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-B complete | B's boxes ticked; census 100% for strings/term/testing | **MET** 2026-08-30 — every box in B's three phases resolved; `./scripts/man-census.sh strings term testing` reports 75/75 on intro, desc and example, 121/121 parameter descriptions, term's types page at 18/18, and 0 memory-scope hits across all three. Commits `ce95ab5c5`, `7c9fc359a`, `36dd9b04e`. |

## 1. Goal

- Every function page in net, http, general, astrings, vector has non-empty
  `intro`/`desc`/`example` + param descs; overviews and types pages
  reviewed/corrected; the straggler pages (list at kickoff) filled.
- **`tcp` (11 pages) and `udp` (8) verified** claim-by-claim through the
  same four steps — their prose is filled, so every existing claim is
  checked against behavior and every example compiled/run for the first
  time; fixes and rejections ledgered like D/E's.
- **The network family's handle wording is settled here and recorded
  verbatim in this letter's ledger** — one set of sentences for "the handle
  stays open", "the caller still closes it", "an alias of the one in the
  list", "closed by this call; the handle cannot be used again". `http`
  uses them in this letter; `tls` copies them in E. This is the
  cross-package consistency case F sweeps for, and C is where it is won or
  lost.
- All claims behavior-verified; zero internals leakage.
- **`scripts/man-census.sh --memory-scope` reports 0** for every package in
  this letter (plan-108-A §3 (2a)): no `borrow`, `pointer`, `ownership`,
  `move`, `free`, `heap`, `lifetime` in rendered output. Where a `RES`
  handle's behavior must be stated, it is stated with **alias** and
  MFBASIC's own verbs (open / close / stays open); anything longer links
  `mfb man variable`.
- Every example compiled while authoring, and run where it needs no live
  endpoint (no example may depend on an external network); compile-only
  members noted per function in the ledger.
- Cross-model review (Codex) per package; findings ledgers recorded here.
- `scripts/man-census.sh` → 100% fill for all five authored packages, tcp,
  udp, AND the straggler list; at this letter's end, **every function page
  tree-wide has desc+example** (the authoring half of plan-108 done). Note
  the denominator moved: A's original "466 pages over ~28 packages"
  excluded tcp/udp, so use whatever A's Phase 1 census reports over all 30
  packages (a summary-table count on 2026-08-30 gives 489 across 30, with
  tcp 11 + udp 8 among them — the two counting methods differ slightly, and
  A's census is the authority).

### Non-goals (explicit constraints)

- **No new inline explanation of the memory model.** Any page that needs
  more than one sentence about copies or handles links `mfb man variable`
  (authored in A) — it does not re-explain, and never in C/Rust terms.
- Per plan-108-A (no compiler testing; prose string fields only with
  per-commit `git diff` check; no renderer/schema changes;
  `src/docs/man/**` untouched).
- No `package.mfb` edits (see References).
- Found code bugs: fix or file via write-bug, recorded here — never doc'd
  around.

## 2. Current State

A's census: net 23 / http 19 / general 18 / astrings 18 / vector 17
function pages with desc column = 0. Stragglers: 194 total empty − 184 in
the nine all-empty packages = 10 spread across filled packages (identify
exactly at kickoff via the census script's per-function output).

`tcp` and `udp` are the opposite case — 100% filled and never verified:
tcp 11 function pages, udp 8, all with desc + example, none with a
compiled example or a checked claim (A measured zero prior example
verification tree-wide).

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to author | 95 + stragglers (list at kickoff) | `scripts/man-census.sh` at kickoff |
| pages to verify (tcp, udp) | 19 (tcp 11, udp 8), all already desc+example | `mfb man <pkg> \| grep -cE '^│ <pkg>::'` (2026-08-30) |
| memory-vocabulary hits in this letter | 33 (tcp 14, udp 11, http 5, net 1, astrings 1, vector 1) | `mfb man <pkg> --all \| grep -cEi 'borrow\|ownership\|\bowns\b\|pointer\|deep copy\|by reference\|heap'` |
| net/http members run vs compile-only | decided per function in Phase 2 | this letter's ledger |
| old_man source coverage | measure at kickoff | `ls planning/old_man/builtins/{net,http,general,astrings,vector}` |

## 3. Design Overview

Same production line as B: one package at a time, author+scope then
review+apply, census per package. Order: general (broadest developer
traffic), astrings, vector — then **the network family together, in one
sitting: net, tcp, udp, http**. The family is deliberately not split across
phases: its four packages describe the same sockets, listeners, and
addresses, and the plan's whole cross-package-consistency risk is that they
end up saying it four different ways. Authoring `net`/`http` while
rewriting `tcp`/`udp`'s handle prose is what lets one set of sentences fall
out. Stragglers last (small, scattered).

`tcp`/`udp` run the same four steps but enter at verification rather than
authoring: check every existing claim against behavior, compile and run
every example (never done before), then scope + memory-scope + review.

**Risk concentration:** (a) net/http prose drifting into
transport-internals (readiness, TLS handshake machinery) — exactly the
leakage class the user called out. Held by the standard's MUST-NOT list and
reviewers prompted with `.ai/net-tls.md` as the "this is what internals
look like" foil. (b) The family's handle wording being settled loosely here
and then diverging in E's `tls` — held by recording the agreed sentences
verbatim in this letter's ledger, which E's Prerequisites require it to
copy.

### Rejected alternatives

- **Skip examples for net/http since they can't hit the network.**
  Rejected: the standard requires an example everywhere; a compile-verified
  example is still type-checked, current, and shown to developers.
- **Leave `tcp`/`udp` in plan-108-E with the other pre-filled packages.**
  Rejected by the user: they belong with `net`. E is organized by fill
  state; the network family is a coherence unit, and grouping by fill state
  would have `tls` (E) setting handle wording that `tcp`/`udp` were
  supposed to share, in the letter that runs after `http` (C) already
  needed it.
- **Author a shared "sockets" explainer in the net overview and have
  tcp/udp/tls/http point at it.** Rejected: `mfb man variable` (plan-108-A)
  is already the one place the handle model is explained; a second
  half-explainer in `net` re-creates the divergence this letter exists to
  prevent. Package pages state their own behavior and link `variable`.

## Compatibility / Format Impact

None to codegen/wire. Summary-pin update only if a pinned summary is itself
corrected.

## Phases

> **Re-scoped 2026-08-30 at kickoff — see Corrections.** Five of this
> letter's seven packages are already filled; only `general` is empty.
> Measured with `./scripts/man-census.sh net http general astrings vector
> tcp udp`:
>
> ```
> net                5      5      5        5       10/10     11  19/19
> http              19     19     19       19       38/38     11  25/25
> general           18     18      0        0        0/21     11      -
> astrings          15     15     15       15       23/23     11  17/17
> vector            19     19     19       19        0/38     11  27/27
> tcp               11     11     11       11       24/24     11    2/2
> udp                8      8      8        8       17/17     11    3/3
> ```
>
> So **18 pages are authored here and 77 are verified.** Two populations
> the phases did not name: **59 missing parameter descriptions** (vector
> 0/38, general 0/21) and **77 memory-vocabulary hits**, not the 33 the
> letter's §2 predicted.

### Phase 1 — general (author), astrings and vector (verify)

- [x] `general`: author 18 pages + overview; every example compiled and
      run. This is the only empty package in the letter. Its members are
      unqualified globals (`len`, `toString`, `typeName`, the `is*`
      predicates), so the pages must be written as bare names with no
      `general::` spelling — the package is deliberately absent from the
      `mfb man` index for that reason (A's Phase 1).
- [x] `general`: author its **21 missing parameter descriptions** (0/21). — 21/21.
- [x] `astrings`: verify 15 pages + overview + types page; examples run. — 15/15 running, types page 17/17.
- [x] `vector`: verify 19 pages + overview + types page; examples run, and
      author its **38 missing parameter descriptions** (0/38) — the single
      largest parameter gap in the letter.
- [x] Memory-scope for the three — drive to 0. — **0**. Both hits were the ones A predicted: `astrings::fromString`'s "deep copy" (A's rewrite-table row) and `vector::abs`'s "copy by value".
- [x] Cross-model review per package + apply; ledgers here. — **30 findings across the three, 30 confirmed, 0 rejected** (general 6, vector 10, astrings 6, plus 8 more found by sweeping their classes). Ledger below.
- [x] Verify: rendering reads clean; census 100% each. — general 18/18, astrings 15/15, vector 19/19; 82/82 parameters; 0 memory-scope and 0 internals hits.

Acceptance: three packages fully authored/verified and reviewed;
memory-scope 0; every parameter described. — **MET**.
Commit: e358e4043, b1d575271, a97e6d8aa, db4124111

### Phase 2 — the network family: net, tcp, udp, http

Done as one unit (§3). Memory-vocabulary baseline **measured at kickoff:
75 hits** (tcp 28, http 23, udp 21, net 3) — not the 31 this section
predicted, for the reason A's Corrections give. `net` and `http` are
already filled, so both are verified rather than authored.

- [x] Verify net (**5** pages, not 23) + http (19) pages + overviews +
      types pages; per-function run-vs-compile verification recorded (no
      external-endpoint dependence).
- [x] Verify tcp (11) + udp (8) pages + overviews + types pages: every
      existing claim checked against behavior, every example compiled and
      run (loopback only — a listener and a client in the same program; no
      external endpoint).
- [x] Settle the family's handle wording and **record the agreed sentences
      verbatim here** — E's Prerequisites depend on this block existing.
      Rewrite tcp/udp's 10 `Borrowed, not consumed` parameter descriptions
      and tcp's "close moves the value into the call" (`man --all` `:36828`)
      per A's rewrite table; use the same sentences in net/http.
- [x] Cross-model review per package + apply; ledgers. — **66 raw findings; 6 net + ~10 across tcp/udp/http confirmed, and 30-odd REJECTED as one sandbox artifact** (see the ledger).
- [x] Verify: rendering + census as Phase 1; `--memory-scope` = 0 for all
      four; `mfb man tcp accept` and `mfb man udp receive` read side by
      side for identical handle wording.

Acceptance: four packages authored/verified and reviewed, ledgered;
memory-scope 0; the agreed handle sentences recorded in this file. —
**MET**; the sentences are the table above.
Commit: fd8e0473d, c5012ff2f, 4a429d828

### Phase 3 — stragglers

**Kickoff finding: there are no straggler pages.** A's Phase 1 census over
all 29 censusable packages shows every function page outside `general`,
`testing` and `thread` already carrying desc + example — the "~10
stragglers spread across filled packages" this section was written for do
not exist. B authored `testing` and A authored `thread`, so once Phase 1
here lands `general`, the tree-wide desc+example count is complete.

- [x] ~~Fill the ~10 straggler pages inside their filled packages~~ —
      **moot: the census finds none.** `./scripts/man-census.sh` reports
      "pages with neither Description nor Examples: 42" tree-wide at A's
      kickoff, and those 42 are exactly general 18 + testing 12 + thread
      12, all of which are owned by a phase. There is no residue.
- [x] Verify: census shows **0 pages without desc+example tree-wide**
      across all 29 censusable packages. — **MET**, `./scripts/man-census.sh`:

      ```
      TOTAL            489    489    489      489     616/807
      pages with neither Description nor Examples: 0
      ```

      **Every one of the 489 function pages now carries an intro, a
      description and an example.** That is plan-108's authoring half
      complete: A authored `thread`, B authored `testing`, C authored
      `general`, and nothing else was ever empty.

      What remains tree-wide is **191 missing parameter descriptions**
      (807 − 616), concentrated in `datetime` 73, `collections` 52,
      `vector` 38 and `math` 28 — all in D's and this letter's package
      lists, and all now named as phase tasks.

Acceptance: census-wide authoring complete — every function page carries
desc+example (denominator per A's Phase 1 census, which covers tcp/udp;
the first draft's 466 did not). — **MET**: 489/489/489 tree-wide.
Commit: e358e4043

## The settled network-family handle wording

Phase 2's deliverable, recorded verbatim so `tls` (E) copies rather than
reinvents. These are the sentences now used identically across net, tcp, udp
and http:

| Situation | Sentence |
|---|---|
| parameter — the handle survives the call | "The handle stays open — you still close it." |
| parameter — the call takes the handle | "Closed by this call; the handle cannot be used again." |
| prose — automatic close | "closes itself when its binding goes out of scope" |
| prose — an element of a polled list | "an **alias** of the one in the list: the list still closes each socket exactly once when it goes out of scope" |
| prose — updated in place, not taken | "`pump` updates the stream in place and leaves it open — you still close it." |
| prose — after an explicit close | "after `tcp::close(sock)`, do not use `sock` again." |
| prose — handed to a thread | "a socket handed to another thread is refused with `ErrResourceMoved`" |

The fifth row is A's Phase 2 "case 3" — the one the two-sentence rewrite table
could not express — and `http`'s `done`/`finish`/`ready`/`pump` are where it
lives. The seventh replaces "a handle that `thread::transfer` moved".

E's `tls` must use rows 1, 2, 3 and 6 verbatim.

## Cross-model review ledger

Reviewer: `codex exec -C <worktree> -s workspace-write - < prompt.txt`, banner
**OpenAI Codex v0.150.0, model `gpt-5.6-terra`**, one run per package.

**Phase 1 — general 6, vector 10, astrings 6; all confirmed, none rejected.**

| Package | Category | Finding | Disposition |
|---|---|---|---|
| general | INACCURACY | `isEven`'s claim that `MOD 2 = 0` "does not always" handle negatives | CONFIRMED — **re-verified**: `-4 MOD 2 = 0` is `TRUE` and `-3 MOD 2 = 0` is `FALSE`, matching `isEven` exactly. The difference is readability, nothing else |
| general | INACCURACY | `toInt` offered `isNumeric` as the test-instead-of-trap guard | CONFIRMED — `isNumeric("1.5")` is `TRUE` but `toInt("1.5")` raises `77050003`. Same shape as B's `find`/`contains` defect: a guard recommended on one page that the guarded call does not honour |
| general | MISSING ×2 | `toInt`'s base range (2–36, verified: base 1 and 37 both raise); `toScalar`'s exactly-one-scalar rule (`""` and `"ab"` both raise `77050002`) | CONFIRMED |
| general | INACCURACY ×2 | `toFixed`'s example labelled "out-of-range" while supplying malformed text; `typeName`'s intro saying "runtime type" when the page's own next paragraph says types are settled at compile time | CONFIRMED |
| vector | INACCURACY | `lerp`'s "the speed along it is constant" | CONFIRMED — true for `Float`, false for `Integer`: `lerp(Integer2[0,0], Integer2[3,0], t)` gives `(2, 0)` at **both** `t = 0.5` and `t = 0.75` |
| vector | LEAKAGE ×9 | "the hardware maximum instruction", "the hardware `Float` square root … corrects that seed", "a dedicated helper", "separate implementations in the companion source — `__vector_cross_float2`", "the implementation computes `dot(b, b)`", the overview's "written in MFBASIC over the intrinsic `math` package" | CONFIRMED — plus five more pages found by sweeping the class |
| astrings | MISSING ×4 | every ranged member validates through one check and **no page said what an invalid range does**. Verified: out of bounds → `77050001`, negative or inverted → `77050002`, `getAttributes` index → `77050001`. The sharpest consequence, now stated: the range is INCLUSIVE, so empty text has **no valid range at all** — even `0, 0` is out of bounds | CONFIRMED |
| astrings | LEAKAGE ×2 | the overview's "value-semantic: it copies deeply, drops with its owning scope", and — **my own rewrite from earlier in this letter** — "it goes away with the scope that holds it" | CONFIRMED |

**Phase 2 — net 6 confirmed; tcp/udp/http ~10 confirmed and 30-odd REJECTED.**

The rejection is a single artifact worth recording, because it would otherwise
recur in every later letter: **the Codex sandbox cannot bind sockets.** Thirty-odd
findings across tcp/udp/http said an example "compiles but does not run", each
citing `7-707-0003 Network operation failed before a connection was established`
while creating a listener, and several chained further conclusions off it
("…so this other example is also broken").

Disproving command:

```
./scripts/man-run-examples.sh tcp --run   → examples: 16  built: 16  ran: 16  failed: 0
./scripts/man-run-examples.sh net --run   → examples: 11  built: 11  ran: 11  failed: 0
./scripts/man-run-examples.sh udp --run   → examples:  9  built:  9  ran:  9  failed: 0
```

The reviewer prompt now states this explicitly and forbids chaining off it. **D
and E must keep that paragraph** — `process`, `audio`, `io`, `term` and `tls`
would each produce the same noise otherwise.

Confirmed in the same runs:

| Package | Category | Finding | Disposition |
|---|---|---|---|
| net | INACCURACY | `toUrl` claimed an empty explicit port raises | CONFIRMED — **re-verified**: `http://example.com:` yields port 80; `:abc` and `:99999` raise `77050003`. The page now scopes the rules to a non-empty port |
| net | INACCURACY | `percentDecode`'s second example had a `main` containing only a comment — it compiled, ran, and printed nothing | CONFIRMED |
| net | INACCURACY | `ping`'s first example had no error handling on a call the page says can fail outright | CONFIRMED — a `Timeout` status is an answer; being unable to probe at all is an error, and the common one on a locked-down host |
| net | LEAKAGE ×3 | `lookup`'s "answer chain walked twice … `AF_INET` nodes" and "released on both the success and the failure exits"; `percentDecode`'s "the inline-trap analysis cannot see" | CONFIRMED |
| tcp/udp | LEAKAGE ×5 | "stale descriptor", "takes the handle into the call", "the handle's closed word … carries the *moved* bit", "a handle that `thread::transfer` moved", `udp::poll`'s "closed exactly once at scope exit … must not be transferred" | CONFIRMED |
| http | LEAKAGE ×2 | `server`/`serverSSL`'s "injected at IR lowering" and "created with `SO_REUSEADDR` set … `AF_INET` hints" | CONFIRMED |

**The finding that changed the plan's instruments.** The tcp review reported a
bug NUMBER rendered into a man page. Checking whether that was isolated found
**69 internals-vocabulary lines tree-wide** — bug-184, audit-2, bug-465,
bug-259, plan-14, bug-63, bug-467, bug-260, five `[[path:symbol]]` old_man
citation markers, mangled `__collections_*`/`__vector_*` symbols, and 30 uses of
"lowering"/"monomorphization". `.ai/man-content.md` §3 forbids all of it and F
must certify it at 0, but **nothing measured it** — every letter had been
closing against `--memory-scope` alone. `scripts/man-census.sh --scope` is that
missing instrument, added in the same commit. This letter's seven packages close
at 0 on it.

## Validation Plan

- Verification: `mfb man <pkg> --all`/`types` per package;
  `scripts/man-census.sh` → zero empty pages anywhere and
  `--memory-scope` 0 for all seven packages; examples/probes compiled and
  run ad hoc during authoring and during tcp/udp verification.
- Doc sync: none beyond content (F owns tooling docs).
- Hygiene: fmt at session end.

## Open Decisions

- None entering — per-function run-vs-compile decisions are made and
  recorded in-phase. (tcp/udp examples are expected to run on loopback
  rather than being compile-only; if a member genuinely cannot, it is
  noted per function in the ledger like any other.)

## Corrections

<Filled in during execution.>

## Summary

The authoring close-out — and the network family's letter: after this one
no builtin man page is a bare skeleton, every function page tree-wide has
verified developer prose and a compiled example, and net/tcp/udp/http say
the same thing the same way about a handle, in MFBASIC terms. D/E verify
the remaining pre-existing prose (E's `tls` copying the wording settled
here) and F certifies.
