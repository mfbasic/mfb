# plan-108-F: Whole-surface certification + retire the stale man tooling

Last updated: 2026-08-30
Effort: medium (1h–2h) — grows to large if the certification sweeps or the
memory-vocabulary sweep find stragglers
Depends on: plan-108-E (every package authored/verified/reviewed).

Certify plan-108's end state with recorded, re-runnable checks — the same
lesson as plan-106-E: the certificate is a measured sweep, not a claim
assembled from letter tick-boxes (memory
`completeness-claims-need-an-audit`). Then retire the tooling and guidance
that still point at the RETIRED Markdown man tree, so the next author lands
in the registry workflow by default.

Per plan-108-A: the only verification instrument is `mfb man` rendering (+
the census script over it) — no compiler test gates.

References:

- `scripts/update_man.sh`, `scripts/update_man_package.sh` — drive claude
  CLI authoring against `.ai/man_template.md` / `man_type_template.md` /
  `man_package_template.md` for the retired `src/docs/man` tree
  (`update_man.sh:1-25`); dead tooling for builtins pages post-108.
- `AGENTS.md` "Creating or updating `mfb man` content" — guidance to
  extend with `.ai/man-content.md` AND the §3 (2a) memory-vocabulary ban,
  so the next author cannot reintroduce borrow/pointer prose without
  reading the rule. It must also learn the new `variable` topic (the
  section currently lists nine narrative topics).
- plan-108-A §3 (2a) + `src/docs/man/variable/package.md` — the ban and the
  page it points at; this letter certifies both.
- `src/cli/man.rs:1-15` — the renderer's own module doc describing the
  registry design (stays; it is accurate).
- `planning/old_man/**` — stays archived as-is (history), but no live doc
  may point to it as the authoring surface.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-E complete | E's boxes ticked; census 100% | NOT MET until E lands |

## 1. Goal

The certification, pasted into this file with each command's output:

- **Fill**: `scripts/man-census.sh` → every function page 100% on
  intro (per A's policy) / desc / example / param-desc; overview + types
  descriptions non-empty for every package.
- **Scope**: a rendered-output sweep for internals leakage —
  `./target/release/mfb man --all` (and per-package `types`) grepped for
  the MUST-NOT vocabulary from `.ai/man-content.md` (at minimum:
  `abi_inline|Body::|monomorph|lowering|NIR|\.ncode|__pkg_|#[a-z]+_[A-Za-z]+\$|\[\[`)
  → 0 hits; every residual hit either fixed or classified here with the
  reason it is legitimate developer vocabulary.
- **Memory vocabulary** (plan-108-A §3 (2a), the hard ban): the same sweep
  run with the banned memory list —
  `borrow|pointer|ownership|owns|owned|owner|move[sd]? (semantics|the value)|consume[sd]?|free[sd]?|frees|heap|stack-allocat|refcount|reference count|garbage collect|lifetime|dangling|deep copy|shallow copy|by reference|by value|RAII`
  over `mfb man --all` **and** the `src/docs/man/variable` topic — **0
  unclassified hits**. The ONLY sanctioned classification is carve-out 1,
  arithmetic borrow in `datetime` (15 rendered lines at baseline); every
  other hit is a defect this letter fixes, not classifies. Baseline to beat,
  measured 2026-08-30: 79 `borrow` / 15 `ownership` / 10 `owns` / 5 `heap` /
  2 `pointer` / 1 `deep copy` / 1 `by reference` across 15 packages — **94
  memory-sense hits** once the 15 `datetime` arithmetic borrows are
  excluded — 54 of them in plan-108-E's packages, 33 in plan-108-C's
  (which includes the network family's 26: tcp 14, udp 11, net 1).
- **The permitted vocabulary is actually used**: spot-check that the pages
  which USED to explain handles now say **alias** (or link
  `mfb man variable`) rather than having simply deleted the contract —
  `mfb man tcp accept`, `mfb man udp receive`, `mfb man tls accept`,
  `mfb man process spawn`, `mfb man audio play` read end-to-end here, with
  the result recorded. The first three are the sharpest test: C settled the
  network family's handle sentences and E's `tls` was required to copy
  them, so any divergence between these three is a failure of that
  hand-off, not a wording nit. Deleting a true statement to pass a grep is the
  failure mode this check exists for.
- **Citations**: rendered output contains no `[[path:symbol]]` markers
  (covered by the sweep above; called out because old_man ports are the
  likely source).
- **Example ledgers complete**: every letter's ledger accounts for every
  example as run or compile-only-with-reason — spot-check the union
  against the census's function list; 0 unaccounted.
- **Tooling retired**: `scripts/update_man.sh`, `update_man_package.sh`,
  and the three `.ai/man_*template*.md` files either deleted or rewritten
  for the registry workflow (Open Decision below); `AGENTS.md`'s man
  section rewritten to point at `.ai/man-content.md` + the registry fields;
  `rg -n 'src/docs/man' AGENTS.md .ai/ scripts/` → only intentionally
  historical references remain, each verified.
- **`mfb man variable` is reachable and consistent**: it renders, the
  topic list shows it, and every package page that gestures at the memory
  model links it rather than restating it (grep the rendered surface for
  competing explanations — the cross-package consistency check below).
- Memory sync: update this project's memory if the audit taught durable
  lessons (e.g. new sharp-edge behaviors discovered by probes); the
  `resources-in-collections-yes-records-no` memory's "the man blurb is
  WRONG" clause updated to reflect the fix landed.

### Non-goals (explicit constraints)

- Per plan-108-A (no compiler testing; no renderer/schema changes;
  `src/docs/man/**` prose guides remain out of scope — the sweep may
  incidentally note leakage there for a future plan, recorded without
  fixing).
- `planning/old_man/**` is not deleted (archive).

## 2. Current State (entering F)

All function pages across the **30** registry packages authored/verified
through A–E with per-package
review ledgers. What has NOT yet happened: any single sweep over the ENTIRE
rendered surface at once (letters worked per-package), and the
tooling/guidance still describes the retired tree.

### Measured populations

| What | Count | Command |
|---|---|---|
| rendered surface to sweep | re-measure (was 30,644 lines at plan-writing) | `./target/release/mfb man --all \| wc -l` |
| stale-tooling references | measure at kickoff | `rg -n 'src/docs/man\|update_man' AGENTS.md .ai/ scripts/` |
| memory-vocabulary hits at plan-writing (the number A–E drive to 0) | 79 borrow / 15 ownership / 10 owns / 5 heap / 2 pointer, 15 packages; 15 of the borrows are the datetime arithmetic carve-out | `./target/release/mfb man --all \| grep -cE '<word>'` (2026-08-30) |

## 3. Design Overview

Two sweeps (fill, scope) + the ledger completeness check, then the tooling
pass. Any straggler a sweep finds is a TASK in this letter, never a
deferral (plan-106-E's rule).

**Risk concentration:** (a) the scope grep's vocabulary list being too
narrow (leakage in words the grep doesn't know) — and for the memory ban
specifically, a page can teach a borrow model with no banned word in it
("the list still holds it, so do not close yours"), which no grep catches;
(b) B–E passing the grep by deleting true handle contracts instead of
restating them in MFBASIC terms. Mitigation: the grep is the
floor, the permitted-vocabulary read-through above is the ceiling; one final cross-model review run (Codex) reads the three largest
packages' rendered output end-to-end as a spot-check of the sweep itself,
and its findings calibrate whether a wider sweep is needed (recorded
either way) — its prompt quotes §3 (2a) and asks explicitly whether any
page still teaches a borrow/ownership mental model without using a banned
word.

### Rejected alternatives

- **Trust the per-letter reviews as certification.** Rejected: per-package
  passes cannot see cross-package inconsistency (terminology drift,
  duplicated-but-diverging explanations of shared concepts like scalar
  indexing) — the whole-surface sweep exists for exactly that class.

## Compatibility / Format Impact

None to codegen/wire.

## Phases

### Phase 1 — the certification sweeps

- [x] Fill sweep (census) — `./scripts/man-census.sh --fill`, whole surface:

```
TOTAL            502    502    502      502     831/831
pages with neither Description nor Examples: 0
```

**502 pages; every one has an intro, a description and an example; 831 of 831
parameters described; every package's overview intro and desc non-empty
(PKGDOC `11` on every row); every types page 100%.** No stragglers.

The 502 is itself a correction: the census used to over-count, because it
scraped every `│ pkg::name` cell in an overview and `math`'s new Constants
table put `math::pi` … `math::ln10` among them — seven "pages" that do not
exist (`mfb man math pi` → `error: unknown math function 'pi'`). Fixed in
plan-108-D; see that letter's Corrections item 2.
- [x] Scope sweep — `./scripts/man-census.sh --scope`, whole surface:

```
internals-vocabulary hits: 0
```

Two hits were found and fixed rather than classified:

- `vector::perpendicular` named its own private helpers: "**separate functions
  with separate implementations** in the companion source —
  `__vector_perpendicular_float2` and `__vector_cross_float2` — rather than one
  delegating to the other; the call dispatches to whichever name you wrote." →
  "They are nevertheless two separate functions, and neither is a shorthand for
  the other: prefer `vector::perpendicular` when you mean a quarter turn, and
  `vector::cross` when you mean the generalized product."
- `audio::close` described its own lowering: "IR lowering routes each operand to
  a distinct per-direction internal body (`audio.closeInput` /
  `audio.closeOutput`), because their teardown sequences differ." → "It accepts
  either direction, and does the right thing for each — a capture stream and a
  playback stream shut down differently."

Also swept out by hand, because the pattern list could not see them: bug
numbers in prose (`tls::listen` cited `(bug-465)`) and a host-API paragraph on
`tcp::accept` ("the listener is temporarily switched into non-blocking mode and
its original file-status flags are restored") — rewritten as the behaviour a
developer observes: "A bounded `accept` never overruns its `timeoutMs`, even in
the awkward case…"

**Citations:** `grep -cE '\[\[[A-Za-z_][A-Za-z_/.]*:'` over the whole rendered
surface → **0**.
- [x] Memory-vocabulary sweep — `./scripts/man-census.sh --memory-scope`,
      whole surface:

```
unclassified memory-vocabulary hits: 0
carve-out 1 (datetime arithmetic borrow): 15
carve-out 2 (derived Errors-table row): 20
```

Per-word, over the rendered surface split three ways (the banned list taken
from `--banned-list` so the sweep and the gate cannot drift apart):

| Surface | Word | Count |
|---|---|---|
| **registry package pages** (50,744 lines) | borrow / borrows / borrowed / borrowing | 6 / 5 / 1 / 3 |
| | allocation | 20 |
| | **everything else in the banned list** | **0** |
| **`mfb man variable`** (228 lines) | — | **0** |
| other narrative topics (out of scope per §1 non-goals) | borrow 4, pointer 4, ownership 6, owned 13, owner 3, consume/consumes/consuming 1 each, frees 1, lifetime 1, lifetimes 1, by value 2, lexical drop 2, allocates 1, allocation 4 | 45 |

The 15 borrow-family and 20 `allocation` hits on package pages are **exactly**
the two carve-outs, and the raw grep total (35) reconciles with the census's
`15 + 20` classification with nothing left over. Carve-out 1 is `datetime`'s
arithmetic borrow ("a negative nanos value borrows a second"). Carve-out 2 is
new to this plan and is recorded in plan-108-E's Corrections item 3: an
Errors-table row is **derived** from the `errorCode` constant descriptors, and
`ErrOutOfMemory`'s message is the string the runtime prints when the error is
raised, so "Allocation failed." appears in a cell no page author can edit and
that this plan is barred from changing.

Against the baseline in §1 — 94 memory-sense hits across 15 packages, 79
`borrow` / 15 `ownership` / 10 `owns` / 5 `heap` / 2 `pointer` / 1 `deep copy`
/ 1 `by reference` — **every one is gone**, none classified away.

The 45 hits in the other narrative topics are recorded, not fixed: §1's
non-goals put `src/docs/man/**` prose guides out of scope. They are a
well-defined follow-up (`mfb man types`, `mfb man flow` and friends still
explain ownership in C terms, which now *contradicts* `mfb man variable`).
- [x] Permitted-vocabulary read-through of the five pages named in §1. Each
      was read end to end; none had the contract deleted to pass a grep:

| Page | What it now says about the handle |
|---|---|
| `mfb man tcp accept` | "The listener stays open — you still close it." + "The returned `Socket` is a fully independent resource: it stays usable after the listener is closed, and closing it does not affect the listener." |
| `mfb man udp receive` | no handle-lifetime claim to make (it returns a `Datagram`, a value) — and it correctly says so rather than inventing one |
| `mfb man tls accept` | "The listener stays open — you still close it — and is available for the next accept" + "The accepted socket shares the listener's server TLS settings; closing the socket leaves them intact" |
| `mfb man process spawn` | "The returned `Process` is a resource handle that cannot be copied. It closes itself when its binding goes out of scope" |
| `mfb man audio play` | "The `output` stream stays open — you still close it." |

The three network pages were the sharpest test, because C settled their
sentences and E's `tls` was required to copy them. They now carry the same
sentence. One divergence was found and fixed: `tcp::accept` said "The listener
stays open **and usable** — you still close it", which is C's sentence plus two
words. Aligned.
- [x] Ledger completeness check. The per-letter ledgers are prose and are
      awkward to diff mechanically, so this letter checked the stronger thing
      directly: **every example on every page of every package, compiled and
      run, in one sweep** (`bash /tmp/runall.sh`, one
      `scripts/man-run-examples.sh <pkg> --run` per package). That is a
      superset of "the ledger accounts for each example" — nothing can be
      unaccounted for if everything was executed.

| Package | Examples | Built | Ran | Failed |
|---|---|---|---|---|
| `app` | 2 | 2 | 2 | 0 |
| `astrings` | 15 | 15 | 15 | 0 |
| `audio` | 11 | 11 | 11 | 0 |
| `bits` | 37 | 37 | 37 | 0 |
| `canvas` | 14 | 14 | 14 | 0 |
| `collections` | 140 | 140 | 140 | 0 |
| `crypto` | 29 | 29 | 29 | 0 |
| `csv` | 6 | 6 | 6 | 0 |
| `datetime` | 112 | 112 | 112 | 0 |
| `encoding` | 57 | 57 | 57 | 0 |
| `errorCode` | 0 | 0 | 0 | 0 |
| `fs` | 94 | 94 | 94 | 0 |
| `general` | 34 | 34 | 34 | 0 |
| `http` | 38 | 38 | 30 | 8 |
| `io` | 22 | 22 | 22 | 0 |
| `json` | 12 | 12 | 12 | 0 |
| `math` | 21 | 21 | 21 | 0 |
| `money` | 5 | 5 | 5 | 0 |
| `net` | 11 | 11 | 11 | 0 |
| `os` | 21 | 21 | 21 | 0 |
| `perf` | 0 | 0 | 0 | 0 |
| `process` | 18 | 18 | 18 | 0 |
| `regex` | 10 | 10 | 10 | 0 |
| `strings` | 84 | 84 | 84 | 0 |
| `tcp` | 16 | 16 | 16 | 0 |
| `term` | 43 | 43 | 32 | 11 |
| `testing` | 14 | 14 | 14 | 0 |
| `thread` | 13 | 13 | 13 | 0 |
| `tls` | 14 | 14 | 9 | 5 |
| `udp` | 9 | 9 | 9 | 0 |
| `vector` | 36 | 36 | 36 | 0 |
| **TOTAL** | **938** | **938** | **914** | **24** |

**Run-not-applicable, with the reason per package** (never a silent skip):

| Package | Not run | Reason, per function |
|---|---|---|
| `term` | 11 of 43 | `drawBox#2`, `drawGlyph#1`, `drawHLine#1`, `drawText#1`, `drawVLine#1`, `fillRect#2`, `moveTo#2`, `setBackground#2`, `sync#2`, `terminalSize#1`, `terminalSize#2` — all raise `ErrUnsupported` because the sweep has no controlling terminal. This is **documented behaviour**, not a defect: `term::terminalSize`'s page already says "If the terminal cannot say — standard output is not a terminal, or the host does not answer — … the call raises". They compile, and the other 32 run (the harness captures their ANSI output). |
| `tls` | 5 of 14 | `listen#1`, `accept#1`, `connect#2`, `localAddress#2`, `remoteAddress#1` — server examples that need a certificate and key on disk (`tls::listen(..., "cert.pem", "key.pem")` raises `ErrTlsFailed` without them) and then block on `tls::accept` waiting for a client that a sweep does not provide. They compile; the other 9 run. |
| `http` | 8 of 38 | **Not an environment limit — all eight are bug-476.** `http::handleRequest` accepts a request and writes no response at all, so every server-shaped example hangs its client: `handleRequest#1`, `#2`, `respondPath#1`–`#3`, `server#1`, `serverSSL#1`, `#2` all hit the 25s timeout. (In the first sweep three of them reported a bind failure instead; that was a leaked server from an earlier killed run still holding port 8080. Re-run with the port clear, they time out like the rest — same cause.) The examples are correct as written. |
| `app`, `canvas` | 0 | Both run (2/2 and 14/14), but in app mode `io::print` goes to the application transcript rather than stdout, so "ran" here means built with `--app`, launched headless and exited 0. |
| `errorCode`, `perf` | — | No examples to run: neither package renders a function page (`errorCode` is constants, `perf` is internal). |

**`mfb man variable`** is not in the table because the harness enumerates
registry packages and it is a narrative topic — so it was checked separately:
all **7** of its example blocks compile, run, and print exactly the output the
page shows (Corrections item 7).

The `http` row is the one worth reading twice: a sweep that classified those
eight as "network examples, cannot run here" would have hidden a HIGH-severity
bug in the package's central function. They were run, they failed, and the
failure was root-caused rather than classified.
- [x] Cross-package consistency. Swept mechanically over the whole rendered
      surface rather than spot-checked, which is what found the one real
      divergence:

- **Resource lifetime wording — a real divergence, fixed.** One rule was being
  stated **seven different ways**: "closed automatically when it leaves scope",
  "closed automatically at scope exit", "closed automatically by itself when its
  binding goes out of scope", "closed exactly once when it leaves scope",
  "closes itself at scope exit", "still closes each of its members exactly once
  at scope exit", and — worst — "**released** automatically when it leaves
  scope" (`fs::File` and both `audio` stream types). `released` is memory
  vocabulary that the banned list happens not to name, so no gate would ever
  have caught it. All eleven converged on C's settled form, "closes itself when
  its binding goes out of scope".
- **Scalar vs grapheme indexing — consistent.** Every page that indexes text
  says "Unicode scalar index — never a byte offset and never a grapheme-cluster
  index"; no competing phrasing exists.
- **Raise vs clamp — consistent and, more importantly, always stated.** Every
  instance takes the form "raises X rather than Y", naming what it does *not*
  do (`ErrMessageTooLarge` "rather than being truncated", `ErrOverflow` "rather
  than wrapping silently", `ErrBadPixelCount` "rather than reading past the end
  or silently…"). `collections::take`/`drop` are the deliberate exception and
  say "clamps rather than failing".
- **Timeout convention — consistent.** Seven pages say "follows the language
  timeout convention" and each then spells out omitted / `0` / positive /
  negative, rather than only pointing at the spec.

Acceptance: all sweeps recorded in this file with 0 unclassified hits.
Commit: bc93ab44d, 91fb5f8f0, 2448ddb29, 07819645a

### Phase 2 — tooling + guidance retirement

- [x] Open Decision executed as recommended — **delete**. The three
      `.ai/man_*template*.md` files are gone (`man_template.md`,
      `man_package_template.md`, `man_type_template.md`); nothing in them
      survived the registry migration, because the renderer now derives every
      section they specified (Synopsis, Package, Imports, Parameters, Return
      value, Errors, See also) and a page author writes only `intro`, `desc`
      and `example` — which `.ai/man-content.md` §2 covers. `update_man.sh`
      and `update_man_package.sh` were already absent from `scripts/`.

      `AGENTS.md`'s man section rewritten around `.ai/man-content.md`, with the
      memory ban stated inline so it is visible without opening the standard:

      > **No C/Rust memory vocabulary on a man page.** The only permitted words
      > are **copy**, **mutate**, **value**, and **alias** (the last for a `RES`
      > handle only). Not: borrow, ownership, move, consume, free, heap,
      > refcount, lifetime, dangling, allocate, deep/shallow copy, by reference,
      > drop. Say what a developer observes — "the handle stays open — you still
      > close it", "you get a copy" — and link `mfb man variable` for the model
      > itself; the precise contract lives in `mfb spec` §14.

      The section also now names the verification instruments, which it never
      did: `scripts/man-census.sh` (`--fill`, `--memory-scope`, `--scope`,
      `--banned-list`) and `scripts/man-run-examples.sh <pkg> --run`.
- [x] `variable` added to AGENTS.md's narrative-topic list — nine → ten
      (`errors`, `flow`, `lambda`, `link`, `optimizations`, `tooling`, `tour`,
      `types`, `unicode`, `variable`), matching `ls src/docs/man/` exactly.
- [x] Memory sync. Three entries added and one corrected:

- **`man-content-standard`** (new) — `.ai/man-content.md` is the standard; the
  permitted four words and the settled handle sentences as a table; the
  templates are deleted; the census/example instruments are the only
  verification, because prose fields are `&'static str` no compiler gate reads.
- **`example-harness-cwd-and-timeout`** (new) — the two ways a doc-example
  runner reports success on broken examples (repo-root cwd; no per-example
  timeout), both learned the hard way in D and F.
- **`man-page-count-scrape-overcounts`** (new) — scraping `│ pkg::name` from a
  whole overview counts non-pages; `math`'s constants have no page.
- **`resources-in-collections-yes-records-no`** (corrected) — its "the
  `mfb man process` blurb is WRONG" clause described a defect this plan fixed.
  Rewritten to record the durable lesson instead: don't trust a package man's
  resource blurb, compile the two-line probe — and note that the compiler hands
  you the right spelling (`TYPE_RESOURCE_REQUIRES_RES` names the `RES` marker).
- [x] Verified. `grep -rn 'update_man\|man_template\|man_package_template\|man_type_template' AGENTS.md .ai/ scripts/`
      → **no output**. `grep -rn 'src/docs/man' AGENTS.md .ai/ scripts/` → 6
      hits, every one intentionally historical or still-live and checked:

| Where | Why it stays |
|---|---|
| `AGENTS.md:105,108` | narrative guide topics genuinely still live there (`src/docs/man/mod.rs` embeds them) |
| `.ai/testing-gates.md:244,248,252,254,296` | the `man_citations_resolve` test still walks `src/docs/man/**` and still fails `cargo test` on a broken `[[path:symbol]]` |
| `.ai/man-content.md:16` | names the retired `src/docs/man/builtins` tree *as retired* — the sentence exists to stop someone going back to it |

      `cargo fmt --all` run over the worktree; the separate `repository/`
      workspace has no changes this plan touched.

Acceptance: no live doc/script directs authors at the retired tree.
Commit: bc93ab44d

## Validation Plan

- Verification: the sweeps ARE the validation, recorded in this file;
  instrument is `mfb man` rendering + the census script.
- Doc sync: Phase 2 IS the doc sync; a memory entry recording the ban as a
  durable authoring rule (permitted four words, `mfb man variable` as the
  one detailed page) — it is exactly the kind of rule the source does not
  reveal. Archive all six plan-108 letters to
  `planning/completed/` as each finished (this letter last).
- Hygiene: fmt at session end (script/doc edits may touch no Rust; skip
  fmt if `git diff` shows no `.rs` change).

## Open Decisions

- **Delete vs rewrite `update_man*.sh` + `.ai/man_*template*.md`**:
  recommend DELETE the scripts (their claude-CLI batch workflow is
  superseded by this plan's per-package workflow) and delete the templates,
  folding anything still valuable (section order, conditional-section
  rules) into `.ai/man-content.md` — one authoring doc, one truth. Rewrite
  only if the user wants a scripted batch re-review capability kept.

## Corrections

1. **The scope sweep's word list was the wrong shape, and its 0 was not
   evidence.** §1 specified the pattern as compiler vocabulary —
   `abi_inline|Body::|monomorph|lowering|NIR|\.ncode|__pkg_|#[a-z]+_[A-Za-z]+$|\[\[`.
   That list reported **0 hits** on a surface still carrying **42** leaked
   lines, because the common leak is not compiler vocabulary. It is:

   - **`Internally, ...` paragraphs** — 11 of them, e.g. "Internally the
     function opens the file read-only, seeks to the end and back to determine
     the length, builds the result `String`, reads the bytes in a loop, and
     closes the descriptor."
   - **C-library and host names** — `errno`, `EEXIST`, `ENOENT`, `EACCES`,
     `EINTR`, `isatty`, `localtime_r`, `tm_gmtoff`, `clock_gettime`, `timespec`,
     "file descriptor", "the descriptor", "system call".
   - **Runtime nouns that read like English** — `arena`,
     `per-execution-context`, "scratch buffer", "entry table", "hash bucket
     index", "data region", "in-place fast path".
   - **"lowered inline"**, which the pattern missed because it matched only
     `lowering`.

   The pattern is corrected in `scripts/man-census.sh` and the class is now
   caught. Two of the additions had to be tightened after they fired falsely:
   `IR ` matches `TMPDIR environment` and `$APPDIR or`, and bare `EEXIST`
   matches the page title `FILEEXISTS`.

2. **Reading pages found what no sweep did.** Three defects surfaced only by
   reading end to end after every sweep was green:

   - `collections::find` claimed "A nested collection that is stored as a handle
     rather than inlined compares by identity, not by contents." Both halves are
     false: a nested collection cannot be searched for at all —
     `collections::find` on a `List OF List OF Integer` is rejected with
     `TYPE_REQUIRES_COMPARABLE`.
   - `money::setRounding`/`getRounding` and `app::getMode` each described their
     own lowering ("a mask and a single store ... the enum discriminant masked
     to its low bit").
   - `fs::close` was leaking descriptor vocabulary *and*, after an earlier edit
     in this plan, said "on some platforms" twice.

   A green sweep is evidence about the pattern, not about the pages.

3. **The cross-package consistency check had to be mechanical, not a
   spot-check.** §1 asked for a spot-check of shared concepts. A spot-check
   would have passed: the divergence was that one rule — "this closes itself" —
   was stated **eleven different ways** across the surface, including
   "**released** automatically when it leaves scope" on `fs::File` and both
   `audio` stream types. `released` is memory vocabulary that the banned-word
   list does not name, so no gate would ever have flagged it. Sweeping all
   phrasings of the concept, then converging them, is what the criterion now
   records.

4. **`carve-out 2` was added to `--memory-scope`** (recorded in full in
   plan-108-E's Corrections item 3): an Errors-table row is derived from the
   `errorCode` constant descriptors, whose `message` is the string the runtime
   prints, so `ErrOutOfMemory`'s "Allocation failed." renders in a cell no page
   author can edit and this plan is barred from changing. 20 rows.

5. **The `os` cross-model review did not complete** — the reviewing model
   returned a usage-limit error twice, ~129k tokens in. `process` and `io`
   completed and were applied (16 findings each). `os` was verified here
   independently (12 memory hits fixed, 21/21 examples run, `--fill`
   19/19/19/19 and 9/9). Recorded rather than counted as done.

6. **The example harness had five defects, every one of which made a broken
   example look fine.** Three are recorded in plan-108-D's Corrections
   (working directory, documented-failure exits, phantom page counts) and the
   per-example timeout in plan-108-E's. Two more surfaced here:

   - **`run_bounded`'s killer subshell held the command-substitution pipe**, so
     the substitution could not return until the killer's `sleep` expired and
     every example took the full timeout however fast it finished (`json`: 300s
     → 13s).
   - **A background job gets `/dev/null` on stdin** unless redirected
     explicitly, so the `STDIN_FILE` plumbing was silently inert and
     `io::input#1` read EOF while looking like a broken example.

   And one environmental trap worth writing down: **a leaked server process
   from a killed earlier run held port 8080 for hours**, which made three
   unrelated `http` examples fail to bind. `kill -9` on the harness parent does
   not reach a grandchild. Check `lsof -nP -iTCP:<port> -sTCP:LISTEN` before
   believing a bind failure.

7. **`mfb man variable`'s own examples were never checked by anything.** The
   harness enumerates registry *packages*, and `variable` is a narrative topic
   — so the one page every other page links to had unverified examples. All
   **7** of its blocks were extracted, compiled and run here; every one runs and
   every printed result matches the output the page shows. (Extracting them
   needs one rule the package path does not: this page puts expected output
   directly under the code at the same indent, so a block must be truncated at
   its last top-level `END`, not at the next unindented line.)

## Summary

The certificate letter: fill and scope invariants proven by recorded
whole-surface sweeps (not assembled from per-letter claims), every example
accounted for in a ledger, and the last pointers to the retired Markdown
workflow removed — leaving `mfb man` documentation accurate,
developer-voiced, example-checked, free of C/Rust memory vocabulary, and
with a single written standard for whoever touches it next.
