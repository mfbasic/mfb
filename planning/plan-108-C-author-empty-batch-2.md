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

- [ ] `general`: author 18 pages + overview; every example compiled and
      run. This is the only empty package in the letter. Its members are
      unqualified globals (`len`, `toString`, `typeName`, the `is*`
      predicates), so the pages must be written as bare names with no
      `general::` spelling — the package is deliberately absent from the
      `mfb man` index for that reason (A's Phase 1).
- [ ] `general`: author its **21 missing parameter descriptions** (0/21).
- [ ] `astrings`: verify 15 pages + overview + types page; examples run.
- [ ] `vector`: verify 19 pages + overview + types page; examples run, and
      author its **38 missing parameter descriptions** (0/38) — the single
      largest parameter gap in the letter.
- [ ] Memory-scope for the three — drive to 0.
- [ ] Cross-model review per package + apply; ledgers here.
- [ ] Verify: rendering reads clean; census 100% each.

Acceptance: three packages fully authored/verified and reviewed;
memory-scope 0; every parameter described.
Commit: —

### Phase 2 — the network family: net, tcp, udp, http

Done as one unit (§3). Memory-vocabulary baseline **measured at kickoff:
75 hits** (tcp 28, http 23, udp 21, net 3) — not the 31 this section
predicted, for the reason A's Corrections give. `net` and `http` are
already filled, so both are verified rather than authored.

- [ ] Verify net (**5** pages, not 23) + http (19) pages + overviews +
      types pages; per-function run-vs-compile verification recorded (no
      external-endpoint dependence).
- [ ] Verify tcp (11) + udp (8) pages + overviews + types pages: every
      existing claim checked against behavior, every example compiled and
      run (loopback only — a listener and a client in the same program; no
      external endpoint).
- [ ] Settle the family's handle wording and **record the agreed sentences
      verbatim here** — E's Prerequisites depend on this block existing.
      Rewrite tcp/udp's 10 `Borrowed, not consumed` parameter descriptions
      and tcp's "close moves the value into the call" (`man --all` `:36828`)
      per A's rewrite table; use the same sentences in net/http.
- [ ] Cross-model review per package + apply; ledgers.
- [ ] Verify: rendering + census as Phase 1; `--memory-scope` = 0 for all
      four; `mfb man tcp accept` and `mfb man udp receive` read side by
      side for identical handle wording.

Acceptance: four packages authored/verified and reviewed, ledgered;
memory-scope 0; the agreed handle sentences recorded in this file.
Commit: —

### Phase 3 — stragglers

**Kickoff finding: there are no straggler pages.** A's Phase 1 census over
all 29 censusable packages shows every function page outside `general`,
`testing` and `thread` already carrying desc + example — the "~10
stragglers spread across filled packages" this section was written for do
not exist. B authored `testing` and A authored `thread`, so once Phase 1
here lands `general`, the tree-wide desc+example count is complete.

- [ ] ~~Fill the ~10 straggler pages inside their filled packages~~ —
      **moot: the census finds none.** `./scripts/man-census.sh` reports
      "pages with neither Description nor Examples: 42" tree-wide at A's
      kickoff, and those 42 are exactly general 18 + testing 12 + thread
      12, all of which are owned by a phase. There is no residue.
- [ ] Verify: census shows **0 pages without desc+example tree-wide**
      across all 29 censusable packages.

Acceptance: census-wide authoring complete — every function page carries
desc+example (denominator per A's Phase 1 census, which covers tcp/udp;
the first draft's 466 did not).
Commit: —

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
