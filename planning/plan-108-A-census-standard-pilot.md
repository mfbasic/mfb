# plan-108-A: man-content census, the developer-doc standard, and the workflow pilot

Last updated: 2026-08-30
Effort: large (3h–1d) — grew with the memory-vocabulary ban (§3 (2a)) and
the new `mfb man variable` topic (Phase 2b)
Overall Effort: huge (> 3d) — plan-108 spans A–F
Depends on: nothing (first letter)

`mfb man` renders every builtin package straight from the clean-room registry
descriptors in `src/codegen/builtins/**` (`src/cli/man.rs:1-15` — the old
`src/docs/man` Markdown tree was retired to `planning/old_man`). The prose
fields on those descriptors are the product's developer documentation, and
they are in a measured half-state: **466 function pages render, 272 carry a
Description + Example, 194 carry neither** — nine whole packages are bare
auto-derived skeletons. (That 466 undercounts: it was measured over 28 of
the 30 registry packages, missing `tcp` and `udp` — see Corrections. Phase
1's census over all 30 sets the real denominator.)

Plan-108's end state (delivered across A–F): every builtin man page is
**accurate to the actual code, written for the MFBASIC developer (never
compiler-internals spec), carries a runnable example that was actually
compiled and run during authoring, and has survived an independent
cross-model review**.

**And it is free of C/Rust memory vocabulary.** MFBASIC is designed so a
developer does not think about memory management day to day; the man pages
must read that way. The permitted memory vocabulary is exactly four words —
**copy**, **mutate**, **value**, and **alias** (`alias` only for `RES`
handles) — and everything else (`borrow`, `pointer`, `ownership`/`owns`,
`move`, `free`, `heap`, `refcount`, …) is **banned from rendered `mfb man`
output**. This is not aspirational: 79 `borrow` lines, 15 `ownership`, 10
`owns`, and 2 `pointer` render today
(`./target/release/mfb man --all | grep -cE '[Bb]orrow'` etc., 2026-08-30),
including the literal `"The returned Socket is a borrowed pointer"`
(`man --all` line 36160). A: writes the ban into the standard and adds the
one page where the model IS explained in MFBASIC terms (`mfb man variable`);
B–E: apply it; F: certifies it at 0 hits.

This is a documentation plan. Its verification instrument is `mfb man <pkg>
[<func>|--all|types]` — rendering the pages. **No compiler test gates
apply**: prose fields are `&'static str` the compiler never reads, so
`artifact-gate`, the cargo suite, and `test-accept` can neither catch a doc
error nor fail on one — do not run them for this plan.

This letter builds what every later letter runs on: the exact census, the
content standard, and the four-step per-package workflow (accuracy pass →
scope pass → cross-model Codex review → apply), piloted end-to-end on one
filled package (`bits`) and one empty package (`thread`).

References:

- `src/cli/man.rs` — the renderer; empty sections are omitted
  (`man.rs:357,385,393` — `intro/desc/example.is_empty()` gates), so a
  rendered census is ground truth for fill state.
- `src/codegen/registry/mod.rs:401-406` — `RegistryFunction { intro, desc,
  example }`; `:147-148` `Parameter::desc`; `:531,556,607` — record / prop /
  resource `description` (the `mfb man <pkg> types` page content).
- `planning/old_man/builtins/**` — 543 retired pages
  (`find planning/old_man/builtins -name '*.md' | wc -l` → 543): the prose
  source material AND a warning — their `[[path:symbol]]` citations pre-date
  the plan-102/103 code motion and must never be trusted or leaked.
- `.ai/man_template.md` / `scripts/update_man.sh` — tooling for the RETIRED
  tree; F retires/replaces it. AGENTS.md's man-page guidance points at the
  retired tree too (F updates it).
- `src/docs/spec/language/14_memory-semantics.md` — the memory model's
  *internals* home, and the vocabulary foil: §14.1 "Copy, move, and freeze",
  §14.3 "Function arguments are owned values", §14.3.1 "Native heap value
  contract" are correct AS SPEC and stay exactly as they are. Everything in
  that register is precisely what man may not say. Plan-108 does not edit
  `mfb spec` (see Open Decisions).
- Memory `resources-in-collections-yes-records-no` — a KNOWN accuracy defect:
  the `mfb man process` blurb claiming a resource "cannot be stored as a
  collection element" contradicts spec §15.6. E fixes it; A's standard uses
  it as the canonical accuracy-failure example.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| release `mfb` at current HEAD (needed to render pages and run examples) | `cargo build --release`; `ls -l target/release/mfb` mtime ≥ HEAD commit date | **MET** 2026-08-30 — built at the worktree tip; binary mtime 20:12 = HEAD commit time |
| no concurrent letter of plan-104-C/D touching the same builtins package | check active worktrees/sessions | **MET** 2026-08-30 — plan-104-A–D are ARCHIVED (`ls planning/completed/ \| grep plan-104` → all four), so no letter of it can be running. The three peer sessions sharing the checkout were each asked directly; all three confirmed no pending builtins-prose edits, and the one overlapping change (`6f99d0af2`, http/tls prose) had already landed and is merged in here. |

Interaction with plans 104–107: plan-108 edits ONLY prose string fields
(`intro`/`desc`/`example`/param `desc`/type `description`) in files that
plan-104-C/D will separately rewrite for typed internals. The edits are
disjoint in content and usually merge clean, but do not work the SAME package
in both plans at the same moment. 108 has no ordering dependency on 104–107.

## 1. Goal

- **The census is committed** (a table in this file): for each of the **30**
  registry packages (`ls src/codegen/builtins/ | grep -v mod.rs | wc -l` →
  30; the "~28" in the first draft missed `tcp` and `udp` — see Corrections) — function-page count, and per page the fill state of
  `intro`, `desc`, `example`, per-parameter `desc`, plus the types-page
  description fill; produced by a script over rendered output (renderer
  omission gates make rendering the truth), kept as
  `scripts/man-census.sh` so every later letter re-runs it verbatim.
- **The content standard exists** as `.ai/man-content.md`: what a man page
  must contain and what it must never contain (see Design). Every later
  letter's accuracy/scope passes and every cross-model reviewer cite it.
- **The memory-vocabulary ban is written into that standard and mechanized**
  (§3 (2a)): the permitted four words, the banned list, the rewrite table,
  and the two carve-outs (arithmetic borrow; `mfb spec`). `scripts/man-census.sh`
  grows a `--memory-scope` mode that greps rendered output for the banned
  list and prints file/function/line for every hit, so every letter can
  measure its own package and F can certify 0.
- **`mfb man variable` exists**: the ONE page where the memory model is
  explained end-to-end, in MFBASIC terms only (copy / mutate / value /
  alias), authored in this letter. Every package page that needs to say
  something about handles or copies links to it instead of re-explaining
  (or C-explaining) the model inline.
- **The pilot is complete**: `bits` (17 filled pages) has been through the
  full accuracy + scope + cross-model review + fix cycle; `thread` (13 empty
  pages) has been authored from scratch (old_man as source material) through
  the same cycle. Both packages' overview and types pages included; every
  example compiled and run during the pass.
- **The cross-model review workflow is written down** (in this file, §3) and
  calibrated by the pilot: reviewer model, prompt, structured-findings
  format, and the triage rule.
- The `errorcode` and `perf` anomaly is resolved: both rendered **0 function
  pages** in the census run (`mfb man perf --all`) — determine whether they
  are prose-guide topics, empty packages, or a renderer gap, and record the
  answer here (it sets whether any letter owns them).

### Non-goals (explicit constraints)

- **No compiler testing.** No `artifact-gate`, no `cargo test` runs, no
  `test-accept` — verification is rendering pages and (during authoring)
  compiling/running examples and probe programs with the release binary.
  The one tree-hygiene exception: `tests/cli_man_summary_plain.rs` pins some
  rendered summary text — if a letter corrects a summary that test pins,
  update the pinned text in the same commit so the tree stays green for
  whoever runs the suite next.
- **Prose fields only.** Several builtins files carry byte-significant
  MFBASIC bodies ("Body byte-significant … do not reformat", e.g.
  `src/codegen/builtins/collections/func_sort_by.rs:2`) — never touch a
  body, a descriptor type, an error table, or any non-prose code; check the
  commit's `git diff` shows string-literal prose changes only.
- No renderer changes; no registry schema changes (no new fields).
- `src/docs/man/**` prose guides (tour, errors, link, lambda, …) are OUT of
  plan-108's scope — this plan is the builtins registry prose only, **with
  one carve-out: the new `variable` topic** (§3 (4)), which this letter
  authors because the ban needs a destination to point at. No other topic
  under `src/docs/man/**` is edited by any letter; leakage noticed there is
  recorded by F, not fixed.
- **The ban does not extend to `mfb spec` or `.ai/**`.** The specification's
  ownership/move/pointer vocabulary is the language contract and is correct
  where it is; plan-108 changes no spec file (Open Decisions).
- No wording churn for its own sake on accurate, in-scope pages.

## 2. Current State

`mfb man <pkg> [fn]` renders package `intro`/`desc`, each function's
`intro`/`desc`/`example` + auto-derived Declaration/Parameters/Errors tables
(from the `Implementation` descriptors — those tables are correct by
construction), and `mfb man <pkg> types` renders record/resource
descriptions.

### Measured populations

> **SUPERSEDED 2026-08-30 by Phase 1.** Every count in this table is the
> 2026-08-24 run at HEAD `254506f9b` and much of it has since moved — six of
> the nine "all-empty packages" are now filled, and the memory-vocabulary
> baseline is 402 rendered lines, not 94. The authoritative figures are
> Phase 1's census and memory-scope tables, and the two Corrections entries
> that explain the deltas. The table is kept because the *methodology* rows
> (source greps overcount; the renderer omits empty sections) are still true
> and still load-bearing. **Cite Phase 1, never this table.**

| What | Count | Command |
|---|---|---|
| function pages rendered | 466 over 28 packages — **undercount**, `tcp` (11) and `udp` (8) were not in the run (Corrections); Phase 1 re-measures over all 30 | python census over `./target/release/mfb man <pkg> --all`, splitting on function-page headers (2026-08-24 run, HEAD 254506f9b) |
| pages with Description + Example | 272 | same census (`\nDescription\n`/`\nExample` section presence; renderer omits empty — `man.rs:385,393`) |
| pages with NEITHER | 194 | 466 − 272 |
| all-empty packages | 9: strings 39, term 25, net 23, http 19, astrings 18, general 18, vector 17, thread 13, testing 12 (= 184; remaining 10 empties are stragglers inside filled packages) | same census, desc column = 0 |
| filled packages needing verification | 16: datetime 45, fs 42, encoding 28, collections 24, math 21, bits 17, crypto 17, os 15, io 15, process 15, audio 12, tls 10, json 5, csv 5, money 4, regex 4, app 3 (−1 straggler each in several) | same census |
| function-level `intro` fill | UNMEASURED precisely (crude source greps conflict: 122 `intro: ""` literals in func files; rendered `strings::mid` shows no intro line) — the census script's first output nails it | Phase 1 |
| packages rendering 0 function pages | 2 (`errorcode`, `perf`) | same census — anomaly, resolved in Phase 1 |
| retired source-material pages | 543 | `find planning/old_man/builtins -name '*.md' \| wc -l` |
| rendered lines carrying banned memory vocabulary | 79 `[Bb]orrow`, 15 `[Oo]wnership`, 10 `\bowns\b`, 5 `heap`, 2 `[Pp]ointer`, 1 `deep copy`, 1 `by reference`; 0 for `dangling`/`refcount`/`garbage collect`/`malloc` | `./target/release/mfb man --all > /tmp/man-all.txt` then `grep -cE '<word>' /tmp/man-all.txt` (2026-08-30) |
| of those 79 `borrow` lines, the MEMORY sense | **64** — 25 are the parameter-desc phrase `Borrowed, not consumed`, 39 more in prose; the other 15 are `datetime` ARITHMETIC borrow (carve-out 1) | `grep -cE 'Borrowed, not consumed' /tmp/man-all.txt` = 25; `mfb man datetime --all \| grep -cE '[Bb]orrow'` = 15; 79 − 15 = 64 |
| **packages with banned vocabulary in RENDERED output** | 15: tls 23, process 18, datetime 15 (all arithmetic — carve-out 1), tcp 14, udp 11, audio 10, http 5, collections 4, then 1 each: strings, net, astrings, vector, crypto, io, money | `for p in …; do echo "$p $(./target/release/mfb man $p --all \| grep -cEi 'borrow\|ownership\|\bowns\b\|pointer\|deep copy\|shallow copy\|by reference\|heap\|refcount\|dangling')"; done` (2026-08-30) |
| source greps OVERCOUNT — do not use them | `fs` shows 37 source hits and **0** rendered: all 37 are the Rust module-doc line 3 (`//! … owns …`) of its func files, invisible to a reader. Same for `term` (4→0) and `thread` (3→0). | `grep -rnoE 'Borrowed\|borrowed\|ownership\|owns' --include='*.rs' src/codegen/builtins/fs/ \| head` vs `mfb man fs --all \| grep -ci borrow` |
| worst single leak | `"The returned Socket is a borrowed pointer — an alias into the list"` (`tls::selectRead`) | `sed -n '36160p' /tmp/man-all.txt` |
| existing `src/docs/man/**` topics (no `variable`) | 9: errors, flow, lambda, link, optimizations, tooling, tour, types, unicode | `ls src/docs/man/` |
| anything that compiles/runs the examples today | 0 | `rg -rn 'example' tests/ src/cli/man.rs` — only `cli_man_summary_plain.rs` touches man at all |
| builtins func files | 419 (some are impl-only, e.g. `collections/func_sort_by.rs` holds a native fast path, its descriptor prose lives elsewhere) | `ls src/codegen/builtins/*/func_*.rs \| wc -l` |

### Verified properties

- The renderer omits empty prose sections (`src/cli/man.rs:357,385,393`) —
  VERIFIED by read; this is why rendered output is the honest census surface
  (per memory `rename-census-by-grep-underreports`: grep RENDERED output,
  not source, when spellings vary).
- Declaration/Parameters/Errors tables derive from the same descriptors the
  compiler executes — VERIFIED (`src/cli/man.rs` module doc + render fns);
  so *those* are accurate by construction and the audit targets PROSE claims.
- The banned vocabulary is concentrated in the RESOURCE packages (tls, tcp,
  udp, process, audio, fs, io) because that is where the docs reached for
  C/Rust words to explain handle lifetime — VERIFIED by the per-package
  count above. This is the exact material the `variable` page replaces.
- `borrow` has one LEGITIMATE non-memory sense in these docs: arithmetic
  borrow in `datetime` normalization ("a negative nanos value borrows a
  second") — VERIFIED by reading the 25 datetime hits. The ban is on the
  memory sense; the census script must not force these to be rewritten
  (§3 (2a) carve-out).
- old_man prose quality is high but citation-stale — VERIFIED by sample:
  `planning/old_man/builtins/strings/mid.md` is excellent developer prose
  (and matches the known `mid` raises-not-clamps behavior) but cites
  `src/target/shared/code/builder_search.rs:lower_mid`, a pre-migration
  location.

## 3. Design Overview

Four artifacts, then the pilot proves them.

**(1) `scripts/man-census.sh`** — renders every package (`--all` + `types`),
emits a per-package, per-function fill table (intro/desc/example/param-desc).
Deterministic output committed alongside each letter's close. It runs
nothing but `mfb man`.

**(2) `.ai/man-content.md` — the standard.** Contents (drafted here, finalized
in Phase 2):

- *A man page is written for the MFBASIC developer at the terminal.* Test:
  if a sentence only matters to someone reading compiler source, it belongs
  in `src/docs/spec/**` or `.ai/**`, not man.
- MUST contain: what the function does in MFBASIC terms; parameter semantics
  (units, valid ranges, zero/empty behavior); return semantics; every
  raisable error and the condition that raises it (consistent with the
  auto-derived Errors table); sharp edge cases (clamps vs raises, Unicode
  scalar vs grapheme, mutation vs new-value, ordering/stability); one
  example the author actually compiled and ran (or compiled only, where the
  environment genuinely can't run it — a tty, a live endpoint, an audio
  device — noted per function in the letter's ledger); cross-references to
  sibling functions where a developer would reach for the wrong one
  (`left`/`right` clamp, `mid` raises).
- MUST NOT contain: registry/descriptor/lowering/monomorph/ABI vocabulary;
  helper or mangled symbols (`#pkg_…`, `__pkg_…`, `$T` suffixes);
  `Body::`/`abi_inline`/NIR/IR/`.ncode`/codegen mechanics; Rust
  implementation details; plan/bug numbers; `[[path:symbol]]` citation
  markers (old_man artifacts — strip on port, never re-derive into prose);
  **and any C/Rust memory-management vocabulary — see (2a), which is a hard
  ban, not a style preference.**
- Accuracy rule: **every behavioral claim is verified by running a program
  against the release binary or by the descriptor tables** — old_man text is
  source material, never authority (behavior may have changed since
  retirement; citations there are stale by construction).
- The canonical accuracy-failure example: the `process` overview's
  resources-in-collections claim (wrong vs spec §15.6).
- The canonical scope-failure example: `tls::selectRead`'s "the returned
  Socket is a borrowed pointer" — every banned word in one sentence.

**(2a) The memory-vocabulary hard ban** (a numbered section of
`.ai/man-content.md`, quoted in every reviewer prompt).

MFBASIC is designed so the developer does not think about memory management
on a day-to-day basis. The docs are part of that design: a page that
explains a handle in terms of borrows and pointers hands the developer a
C/Rust mental model the language deliberately does not require. The ban
exists to keep that model out of the product, not to police wording.

*Permitted memory vocabulary — these four, and nothing else:*

| Word | Means, in MFBASIC terms | Applies to |
|---|---|---|
| **value** | the thing a variable holds; two variables never share one | everything |
| **copy** | assigning/passing gives an independent value; changing one cannot change the other | everything copyable |
| **mutate** | changing a `MUT` binding or a collection in place | `MUT` bindings, collections |
| **alias** | a second name for the SAME open handle — no copy is made, and closing through either name closes it | `RES` handles ONLY |

`alias` is the resource-only escape hatch. It is the ONLY permitted way to
say "this is not a copy", and it may not be used for values.

*Banned outright from rendered `mfb man` output* (memory sense):
`borrow`/`borrowed`, `pointer`, `reference` (as a memory concept),
`ownership`/`owns`/`owned`/`owner`, `move`/`moved`/`move semantics`,
`consume`/`consumed`/`consumes` (DECIDED 2026-08-30 — see below),
`free`/`frees`, `allocate`/
`allocation`/`heap`/`stack`, `refcount`/`reference count`/`garbage
collect`, `lifetime`, `dangling`, `deep copy`/`shallow copy`, `by
reference`/`by value`, `drop` (memory sense), `RAII`, `escape analysis`.

`consume`/`consumed` is banned deliberately, and it is the ban's most
common single hit: 25 rendered parameter descriptions say "Borrowed, not
consumed" (`mfb man --all | grep -c 'Borrowed, not consumed'`, 2026-08-30 —
process 7, tcp 6, audio 4, tls 4, udp 4). It is Rust move-semantics
vocabulary, but there IS a real developer-visible event underneath it, so
the rewrite must keep the fact and drop the word: the handle is either left
open (the caller still closes it) or closed by the call (and cannot be used
again). MFBASIC already has the verb for both — `close`.

*Rewrite table* (the mechanical translations; letters B–E apply these):

| Banned today | Write instead |
|---|---|
| "Borrowed, not consumed." (param desc — 25 of these) | "The handle stays open — the caller still closes it." |
| "consumes its `RES` argument" / "the handle is consumed" | "closes the handle; it cannot be used again" |
| "the caller keeps ownership and must close it" | "the caller still closes it" |
| "the returned Socket is a borrowed pointer — an alias into the list" | "the returned socket is an alias of the one in the list — closing it closes that one" |
| "the list keeps ownership and still closes both" | "the list still closes both" |
| "close moves the value into the call" | "close takes the handle; it is closed afterwards and cannot be used again" |
| "closing the socket never frees the listener's context" | "closing the socket leaves the listener open" |
| "letting a Process drop at scope exit" | "letting a Process go out of scope" |
| "so the caller owns the returned value unconditionally" | "so the caller always gets a value back" |
| "fromString builds a deep copy of text" | "fromString builds its own copy of text" |
| "a resource cannot be stored as a collection element" | (delete — it is also FALSE, spec §15.6; E) |

*Two carve-outs, both narrow and both recorded per hit:*

1. **Arithmetic borrow.** `datetime` normalization legitimately borrows a
   second — **15** rendered lines, and every `borrow` in `datetime` is this
   sense (`mfb man datetime --all | grep -cE '[Bb]orrow'` = 15, 2026-08-30).
   Not a memory claim; keep. The census script flags them and D classifies
   the whole set once, in its ledger — not per page.
2. **`mfb spec` and `.ai/**` are untouched.** The spec's §14 memory model
   IS the precise contract and needs its precise words; the ban is on the
   `mfb man` surface only. If a man page needs to be that precise, that is
   a signal it is saying too much — cut it and link `mfb man variable`.

*Enforcement:* `scripts/man-census.sh --memory-scope` greps rendered output
for the banned list. Every letter runs it for its own packages before
closing; F runs it whole-surface and certifies 0 unclassified hits.

**(3) The four-step per-package workflow** (used by A's pilot and every
later letter):

1. **Accuracy pass** — for each page: check every prose claim against the
   implementation and by running probe programs; fix or excise. Port from
   old_man where the page is empty, re-verifying each ported claim. Compile
   and run the example as part of writing it.
2. **Scope pass** — apply `.ai/man-content.md`'s MUST-NOT list; rewrite
   internals-speak into developer terms or delete it. Run
   `scripts/man-census.sh --memory-scope <pkg>` and drive it to 0
   unclassified hits using the (2a) rewrite table; a claim that survives
   only in banned words is a claim that belongs in `mfb spec` — cut it and
   link `mfb man variable`. Paste the before/after counts in the ledger.
3. **Cross-model review** — run the **Codex CLI** (a different vendor's
   model entirely — stronger independence than another Claude tier;
   `codex-cli 0.149.1` at `~/local/bin/codex`, verified installed), one
   non-interactive run per package:
   `codex exec -C <repo> -s workspace-write '<review prompt>'`.
   The prompt instructs it to: render `mfb man <pkg> --all` and
   `mfb man <pkg> types`; independently verify every factual claim against
   the code and by compiling/running MFBASIC probe programs (scratch
   projects under `/tmp`); flag (a) inaccuracies with evidence,
   (b) internals leakage per `.ai/man-content.md` — **including the (2a)
   memory-vocabulary ban, quoted into the prompt verbatim, with the
   instruction to flag any sentence that would teach a reader a
   borrow/ownership mental model even without using a banned word**,
   (c) missing
   developer-critical information (unlisted error conditions, surprising
   edge cases). Output: a structured findings list
   (function / claim / verdict / evidence), captured verbatim into the
   letter's ledger along with `codex --version` and the model it reports.
   `workspace-write` sandbox because verification requires building probes;
   the reviewer never commits — the main thread applies every fix
   (step 4), and `git status` must be clean of reviewer-made edits after
   each run.
4. **Apply** — triage each finding on the main thread: confirmed → fix;
   rejected → record WITH the disproving command in the letter's ledger.
   Re-render the touched pages and re-run `scripts/man-census.sh` for the
   package.

**(4) `mfb man variable` — the one detailed page.** A narrative topic
(`src/docs/man/variable/package.md`, embedded per `src/docs/man/mod.rs`),
the single place the memory model is spelled out, entirely in MFBASIC terms:

- `LET` vs `MUT` — what a variable holds and when it can change.
- **Every value is independent**: assigning or passing copies it; there is
  no way for two variables to share one value. Show it (mutate a copy,
  print the original unchanged).
- **Mutation** — `MUT` bindings and in-place collection updates; records
  are `WITH`-only.
- **`RES` handles are the one exception**: a handle is not copied, it is
  aliased. Two names, one open thing; closing through either closes it.
  When the scope ends, it closes itself. Show `WITH`/scope close, show a
  handle passed to a function and still usable after.
- Scope: values go away when their scope ends; the developer does not free
  anything and cannot leak by forgetting.
- What this page does NOT say: no allocation strategy, no stack/heap, no
  optimizer behavior. A pointer to `mfb spec memory` for anyone who wants
  the internals — one line, at the end.
- Every code block compiled and run, like any other page in this plan.

**Risk concentration:** (a) the ban being applied as find-and-replace,
losing a TRUE contract (e.g. "the handle stays open" is load-bearing for
`tls::accept`) — held by the rewrite table preserving the developer-visible
fact in every row, and by the reviewer being asked to check the rewritten
sentence still states what happens to the handle;
(b) porting old_man prose without re-verification —
held by the accuracy rule + the cross-model reviewer being prompted to
verify, not proofread; (c) prose edits straying into byte-significant
descriptor/body code — held by the prose-fields-only constraint and a
`git diff` check per commit (string-literal changes only).

### Rejected alternatives

- **Restore the old Markdown tree instead of filling registry fields.**
  Rejected: the registry renderer is the shipped design (`man.rs` module
  doc); two sources would immediately diverge and the tables are already
  derived from the descriptors.
- **A permanent example-running test harness (`tests/man_examples.rs`).**
  Rejected by the user: this is a docs plan and needs no test
  infrastructure; examples are verified by compiling/running them at
  authoring time, recorded in each letter's ledger.
- **One giant end-of-plan review instead of per-package review in each
  letter.** Rejected: findings arrive after the author-context is gone;
  per-package review keeps the fix loop short and lets the reviewer verdicts
  calibrate the very next package.
- **Trust old_man citations to locate implementations.** Rejected: they
  pre-date the builtins migration (sampled stale).
- **Allow `borrow`/`ownership` where it is "technically accurate".**
  Rejected by the user: accuracy is not the test — the test is whether the
  developer has to hold a C/Rust memory model to read the page. MFBASIC is
  designed so they do not, and the docs must not reintroduce the
  requirement. Precision of that kind lives in `mfb spec`.
- **Explain the model inline on each resource package's overview.**
  Rejected: 20 packages carry this vocabulary today and they already
  disagree with each other; one `mfb man variable` page plus a link is the
  single explanation, and F's cross-package consistency sweep can then
  actually pass.
- **Ban the words globally, including `mfb spec` and `.ai/**`.** Rejected
  (scope): the spec is the language contract and needs `copy`/`move`/
  `owned` precisely; the internals docs are for compiler work. The product
  surface a developer reads day to day is `mfb man`, and that is what the
  ban covers. Revisit as its own plan if desired (Open Decisions).

## Compatibility / Format Impact

None to codegen/wire (prose strings only — the compiler never reads them).
`mfb man` rendered output changes are the deliverable.
`tests/cli_man_summary_plain.rs` pinned text updated in the same commit ONLY
if a pinned summary is itself corrected.

## Phases

### Phase 1 — census tooling + the exact census

- [x] Write `scripts/man-census.sh`; run it; commit the per-package
      fill table into this file (replacing the UNMEASURED intro row).
      Four modes: `--fill` (default), `--functions`, `--memory-scope`,
      `--banned-list`.
- [x] Add `--memory-scope` mode (§3 (2a) banned list over rendered output,
      per-package hit list with line context); run it whole-surface and
      commit the baseline table here — the number B–F drive to 0.
      Baseline is **402** unclassified hits, not the 94 §2 predicted
      (Corrections).
- [x] Resolve the `errorcode`/`perf` zero-page anomaly; record the answer
      and assign ownership (a later letter's package list, or out of scope
      with the reason). Resolved below — they resolve differently.
- [x] Verify: script is deterministic (two runs, identical output).
      `./scripts/man-census.sh > a; ./scripts/man-census.sh > b; diff a b`
      → no output.

Acceptance: census table in this file; memory-scope baseline table in this
file; anomaly resolved in writing. — **MET**.
Commit: (hash recorded in the following commit)

#### The census (measured 2026-08-30, `./scripts/man-census.sh`)

PKGDOC is `<overview-intro><overview-desc>`, 1 = present. TYPES is
described-entries / total, where an entry is one record FIELD, one union or
enum VARIANT, or one resource; `-` means the package renders no types page.

```
PACKAGE        PAGES  INTRO   DESC  EXAMPLE  PARAM-DESC PKGDOC  TYPES
---------------------------------------------------------------------------
app                2      2      2        2         1/1     11    2/2
astrings          15     15     15       15       23/23     11  17/17
audio             11     11     11       11       22/22     11  17/17
bits              17     17     17       17       27/27     11      -
collections       49     49     49       49      51/103     11      -
crypto            20     20     20       20       55/55     11  28/28
csv                4      4      4        4       11/11     11    8/8
datetime          44     44     44       44        0/73     11  40/40
encoding          28     28     28       28       28/28     11      -
errorCode          0      0      0        0         0/0     11      -
fs                41     41     41       41       54/54     11    1/1
general           18     18      0        0        0/21     11      -
http              19     19     19       19       38/38     11  25/25
io                15     15     15       15         7/7     11      -
json               4      4      4        4         7/7     11  12/12
math              21     21     21       21        0/28     11      -
money              3      3      3        3         3/3     11    2/2
net                5      5      5        5       10/10     11  19/19
os                19     19     19       19         9/9     11      -
process           14     14     14       14       26/26     11    7/7
regex              4      4      4        4       11/11     11      -
strings           39     39     39       39       64/64     11      -
tcp               11     11     11       11       24/24     11    2/2
term              24     24     24       24       34/34     11  13/18
testing           12     12      0        0       23/23     11      -
thread            12      0      0        0        0/22     10      -
tls               11     11     11       11       28/28     11    2/2
udp                8      8      8        8       17/17     11    3/3
vector            19     19     19       19        0/38     11  27/27
---------------------------------------------------------------------------
TOTAL            489    477    447      447     573/807
pages with neither Description nor Examples: 42
```

**The denominator is 489 function pages across 29 packages** (30 directories
minus `perf`, which is not a package — see the anomaly below). This settles
§2's open question: the "466 over 28 packages" figure was an undercount, and
489 is the number B–F cite.

**What is actually left to do is not what §2 says.** Six of the nine
"all-empty packages" have been filled since the 2026-08-24 census; only three
remain empty. See Corrections — B and C are re-scoped accordingly.

| Work | Population | Where |
|---|---|---|
| author desc + example from scratch | **42 pages** | general 18, testing 12, thread 12 |
| verify existing desc + example | **447 pages** | the other 26 packages |
| author the missing function `intro` | **12** | thread (every other package is 100%) |
| author missing parameter descriptions | **234** | datetime 73, collections 52, vector 38, math 28, thread 22, general 21 |
| author missing types-page entries | **5** | term (`TermColor.r/g/b`, `TermSize.columns/rows`) |
| remove banned memory vocabulary | **402 lines** | 22 packages — table below |

#### The memory-vocabulary baseline (`./scripts/man-census.sh --memory-scope`)

**402 unclassified hits; 15 carve-out 1** — every `borrow` in `datetime`, all
arithmetic, exactly as §3 (2a) predicted. A hit is a RENDERED LINE, so a line
carrying two banned words counts once.

```
collections 66   fs 52   tls 48   process 32   tcp 28   strings 28   audio 25
http 23   udp 21   term 18   crypto 18   os 12   io 9   encoding 6
datetime 5 (+15 carve-out)   net 3   regex 2   bits 2
vector 1   money 1   json 1   astrings 1
```

By word (a line may match more than one): `consumed` 69, `owned` 61,
`borrowed` 59, `allocation` 46, `lexical drop` 38, `allocated` 37,
`consumes` 22, `allocates` 22, `ownership` 15, `consuming` 12, `by value` 11,
`owns` 10, `frees` 9, `owner` 8, `lifetime` 6, `allocating` 6, `freed` 5,
`consume` 5, `allocate` 5, `borrows` 4, `moved into` 3, `moves the value` 2,
`pointer` 1, `free the` 1, `deep copy` 1, `by reference` 1, `borrow` 1,
`allocator` 1.

#### The `errorcode` / `perf` anomaly — RESOLVED

Both were census artifacts rather than renderer gaps, and they resolve
differently:

- **`errorCode` is a real package the census misspelled.** The directory is
  `src/codegen/builtins/errorcode/` but the import name is camelCase
  `errorCode`, so `mfb man errorcode` answers ``error: mfb man: unknown
  package `errorcode` `` while `mfb man errorCode` renders a full overview.
  It exports **constants only** — no callables, no types
  (`errorcode/mod.rs:3-6`: "a flat set of `Integer` constants … and nothing
  else: no callables, no builtin types, no resource") — so **0 function pages
  is correct and permanent**, not an emptiness to fill. Its overview intro and
  desc are present (PKGDOC `11`). **Ownership: plan-108-E**, as a one-page
  overview verification alongside its other small packages.
  `scripts/man-census.sh` now maps the directory name to the import name.
- **`perf` is not an MFB package at all.** `src/codegen/builtins/perf/perf.rs:1-6`
  says so outright: "These are NOT an MFB `perf::` package — there is no
  language surface; the four helpers are invoked only by compiler-injected
  calls in a `--cfg perf`-built … program". `mfb man perf` correctly errors.
  **Out of scope, owned by no letter**, and excluded by `packages()` in the
  census script with that reason recorded inline.

Two enumeration facts the anomaly turned up, recorded so no later letter
re-derives them:

- The `mfb man` index lists **27** packages; `general` and `testing` are
  deliberately absent because their members are unqualified globals needing
  no `IMPORT` (`mfb man general`: they "are written as bare names and have no
  `general::` spelling"). Both nonetheless render full pages and **are in
  scope** — two of the three packages still to be authored.
- 30 directories − `perf` = **29 censusable packages**.

### Phase 2 — the standard

- [ ] Author `.ai/man-content.md` per §3 (2), including the intro policy
      (Open Decision below) so the census can enforce it.
- [ ] Write §3 (2a) into it verbatim: permitted four, banned list, rewrite
      table, both carve-outs, and the "link `mfb man variable`, do not
      re-explain" rule.
- [ ] Confirm the `consume`/`consumed` ban survives contact with the
      pilot's pages and that the two replacement sentences ("stays open —
      the caller still closes it" / "closed by this call; the handle cannot
      be used again") cover every one of the 25 parameter descriptions;
      amend the rewrite table if a third case exists.

Acceptance: standard committed; census script checks every field the
standard requires; `--memory-scope` banned list and the standard's list are
the same list (one source — the script reads it or the doc quotes it).
Commit: —

### Phase 2b — `mfb man variable`

- [ ] Author `src/docs/man/variable/package.md` per §3 (4); wire it the way
      the other nine topics are wired (`src/docs/man/mod.rs`).
- [ ] Compile and run every code block in it.
- [ ] Verify: `mfb man variable` renders; `mfb man` with no args lists it
      alongside the other topics.

Acceptance: the page renders and contains no word from the (2a) banned
list (`mfb man variable | grep -E '<banned>'` → 0).
Commit: —

### Phase 3 — pilot: `bits` (verify-filled) + `thread` (author-empty)

- [ ] `bits`: accuracy pass + scope pass over its 17 pages + overview +
      types page; every example compiled and run.
- [ ] `thread`: author all 13 pages (+ overview check, types page) from
      code + old_man source material, every claim behavior-verified, every
      example compiled and run (thread examples spawn and join).
- [ ] Cross-model review (Codex, one `codex exec` run per package) + apply
      findings; record the findings ledger (confirmed/rejected + evidence)
      here.
- [ ] Memory-scope pass on both. Measured 2026-08-30
      (`mfb man <pkg> --all | grep -nEi 'borrow|ownership|owns|pointer|deep
      copy|by reference|heap|refcount'`): `bits` = 2 rendered lines (one
      `ctz` sentence, "a pointer or size is 2^k-aligned…" — a real leak,
      rewrite as "a value is 2^k-aligned…"), `thread` = 0. Drive to 0 and
      confirm the rewrite table survives contact with real pages; amend it
      with any row the pilot needed and B–E will reuse.
      Note the pilot's own lesson for the census script: `thread`'s source
      greps hit `owns` three times, all in Rust comments, none rendered —
      `--memory-scope` MUST read rendered output only (memory
      `rename-census-by-grep-underreports`, inverted).
- [ ] Verify: re-render both packages (`mfb man bits --all`, `mfb man
      thread --all`, `types`); census re-run shows both at 100% fill and 0
      memory-scope hits.

Acceptance: both pilot packages 100% filled/verified/reviewed with 0
memory-scope hits; workflow §3 amended with anything the pilot taught
(reviewer prompt fixes, triage rules, rewrite-table rows).
Commit: —

## Validation Plan

- Verification instrument: `mfb man <pkg> [<func>|--all|types]` rendering +
  `scripts/man-census.sh`; examples and probe programs compiled/run ad hoc
  with the release binary during authoring. No compiler test gates (see
  Non-goals).
- Coverage check: census script output = the denominator; pilot packages at
  100% fill.
- Doc sync: `.ai/man-content.md` and `src/docs/man/variable/package.md` are
  NEW docs; AGENTS.md/template retirement deferred to F (recorded there).
  AGENTS.md's "Creating or updating `mfb man` content" section gains a
  pointer to the (2a) ban in F's Phase 2.
- Hygiene: `rustup run 1.96.0 cargo fmt --all` at session end (the prose
  lives in `.rs` files — standing AGENTS.md requirement, not a test).

## Open Decisions

- **Codex model pinning**: `codex exec` uses whatever model
  `~/.codex/config.toml` configures unless `-m` is passed. Decide during
  the pilot whether to pin one with `-m` for review consistency across
  letters; record the choice and the reported model here either way.
- **`mfb spec` boundary**: this plan leaves the spec's ownership/move
  vocabulary alone (Rejected alternatives). If the ban is meant to reach
  `mfb spec` too, that is a separate plan with a different risk profile
  (the spec's words carry normative weight) — confirm the boundary at
  kickoff and record it here.
- **Function-level `intro` policy**: the one-line intro under the title is
  empty nearly everywhere; recommend REQUIRED (one sentence, distinct from
  desc's first line) — decide in Phase 2 when writing the standard; the
  census then reports it.

## Corrections

**2026-08-30 (Phase 1) — six of the nine "all-empty packages" are no longer
empty; B and C are re-scoped.** §2's census was taken 2026-08-24 at HEAD
`254506f9b`. Measured again at this worktree's tip with
`./scripts/man-census.sh`, the fill state has moved a long way:

| Package | §2 said | Phase 1 measures | Letter |
|---|---|---|---|
| strings | 39 pages, 0 desc | 39 pages, **39** desc + 39 example | B → verify, not author |
| term | 25 pages, 0 desc | 24 pages, **24** desc + 24 example | B → verify, not author |
| net | 23 pages, 0 desc | **5** pages, 5 desc + 5 example | C → verify, not author |
| http | 19 pages, 0 desc | 19 pages, **19** desc + 19 example | C → verify, not author |
| astrings | 18 pages, 0 desc | **15** pages, 15 desc + 15 example | C → verify, not author |
| vector | 17 pages, 0 desc | **19** pages, 19 desc + 19 example | C → verify, not author |
| general | 18 pages, 0 desc | 18 pages, **0** desc — still empty | C → author |
| testing | 12 pages, 0 desc | 12 pages, **0** desc — still empty | B → author |
| thread | 13 pages, 0 desc | **12** pages, **0** desc — still empty | A pilot → author |

So the authoring population is **42 pages across 3 packages**, not 184 across
9; the other six move from B/C's authoring column into the verification
column. The whole-plan population is unchanged in kind — every page still
needs the accuracy + scope + review cycle — but the balance shifts sharply
from *authoring* to *verification*, which A's §3 workflow already covers as
its cheaper branch (B's own §2 note: "verification of an existing page is
cheaper per page than authoring").

This does NOT re-split the feature: B keeps strings/term/testing, C keeps
net/http/general/astrings/vector/tcp/udp. Only the per-package starting
point changes, and each letter's Phase text is amended in place.

Two populations §2 never measured, now measured and assigned:

- **234 missing parameter descriptions**, concentrated in packages §2 called
  "filled": datetime 0/73, collections 51/103, vector 0/38, math 0/28,
  thread 0/22, general 0/21. A page with prose but bare parameter cells still
  fails this plan's Goal ("every function page … has … per-parameter `desc`"),
  so these are D's and C's work, not a cosmetic gap.
- **5 missing types-page entries**, all in `term` (`TermColor.r/g/b`,
  `TermSize.columns/rows`) — B's.

**2026-08-30 (Phase 1) — the memory-vocabulary baseline is 402 rendered
lines, not 94.** §2 and F both quote "94 memory-sense hits". That figure was
produced by a regex that omitted four of the ban's own words, and each is
common:

```
consumed 69   owned 61   allocation 46   lexical drop 38   allocated 37
consumes 22   allocates 22   consuming 12   allocate 5   freed 5   allocator 1
```

§3 (2a) bans `consume`/`consumed` explicitly (calling it "the ban's most
common single hit") and bans `allocate`/`allocation` in its banned list, yet
the measuring command in §2 searched for neither, and searched `owns` without
`owned`. Measured with the full list — as `scripts/man-census.sh
--memory-scope` now does, using the script's `BANNED_CORE` as the one source
— the surface carries **402 unclassified hits plus 15 carve-out 1**.

One false positive was found and fixed while calibrating: unbounded `heap`
matches **`cheap`** (5 rendered lines, e.g. `vector::perpendicular`'s "a cheap
exact negation"). The script now matches whole words only, spelling the
boundaries as `[^A-Za-z]` because BSD grep has no portable `\b`. Bare `own` is
deliberately NOT banned — "builds its own copy" is the rewrite table's own
prescribed replacement.

Consequences: F's certification regex must be replaced by
`scripts/man-census.sh --banned-list` (F Phase amended), and B–E's per-letter
baselines are restated from this run.

**2026-08-30 — memory-vocabulary hard ban added (user directive).** The
plan as first written treated "developer voice, not compiler spec" as a
style rule enforced by a reviewer's judgement. It is now a hard,
mechanically-checked ban with a fixed permitted vocabulary (copy / mutate /
value / alias-for-`RES`), a rewrite table, a `--memory-scope` census mode,
and a new `mfb man variable` topic as the single place the model is
explained. Rationale: MFBASIC is designed so the developer does not think
about memory management day to day, and 79 `borrow` / 15 `ownership` / 2
`pointer` rendered lines were teaching the opposite model. See §3 (2a) and
(4); B–F amended to match.

**2026-08-30 — `tcp` and `udp` were assigned to no letter (coverage gap).**
Found while measuring the memory vocabulary: `src/codegen/builtins/` holds
**30** packages (`ls src/codegen/builtins/ | grep -v mod.rs | wc -l`), but
A–E's package lists name only 28 (26 assigned + `errorcode`/`perf` pending
this letter's anomaly resolution). `tcp` and `udp` appear in no letter.
They are **assigned to plan-108-C, with `net`** (user's call, 2026-08-30):
`net` supplies the addresses they take and return, the family reads as one
unit, and C authoring `net`/`http` at the same moment it rewrites
`tcp`/`udp`'s handle prose is what produces ONE set of handle sentences
instead of four. E's `tls` is then required to copy C's wording rather than
invent its own (E's Prerequisites). C's title, page counts, phases, and
effort are updated; E's are reduced accordingly.

Two sub-corrections from the same measurement:

- **They are fully filled, not partially.** The first draft of this
  correction read `tcp` 12 desc / 11 example and `udp` 9 / 8 and inferred a
  straggler each. The extra `Description` is the package OVERVIEW's own
  heading: `tcp` has **11** function pages and `udp` **8**
  (`mfb man <pkg> | grep -cE '^│ <pkg>::'`), all with desc + example. So
  they need C's verification cycle, not authoring — no stragglers.
- **The 466-page denominator excluded them.** §2's census counted 466
  function pages "for all 28 packages", so tcp's 11 and udp's 8 are not in
  it. A summary-table count over all 30 packages on 2026-08-30 gives 489,
  which does not reconcile to 466 + 19 = 485 — the two counting methods
  differ (overload collapsing is the likely cause). Phase 1's census script
  must run over all 30 packages and its output is the authority; C and F
  cite it rather than a fixed number.

The "~28 registry packages" figure in §1 is corrected to 30.

## Summary

The machinery letter: an honest rendered census, a written standard for
"developer docs, not compiler spec" — including a hard ban on C/Rust memory
vocabulary and the one MFBASIC-terms page (`mfb man variable`) that replaces
it — and a four-step workflow proven on one filled and one empty package — so B–F are production-line letters, not
design work, and the only tool any of them runs is `mfb` itself.
